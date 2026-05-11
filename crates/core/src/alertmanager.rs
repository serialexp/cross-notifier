//! Prometheus Alertmanager webhook → `Notification` translation.
//!
//! Alertmanager (`https://github.com/prometheus/alertmanager`) posts alert
//! groups to webhook receivers as v4-shaped JSON. This module accepts that
//! JSON directly so callers can wire Alertmanager at a cross-notifier
//! endpoint without an intermediate adapter. Each accepted webhook produces
//! exactly one `Notification` (one notification per alert *group*, not per
//! alert — grouping is configured on the Alertmanager side and represents
//! a single logical signal to the operator).
//!
//! Wire format reference:
//!   https://prometheus.io/docs/alerting/latest/configuration/#webhook_config
//!
//! Severity convention: we look at `commonLabels.severity` to map firing
//! alerts to cross-notifier statuses. Both `page` (gothab convention) and
//! `critical` (the Prometheus mixin convention) are recognised as
//! page-worthy. Anything else firing → `info`. Resolved → `success`,
//! regardless of severity.
//!
//! Dedup: the alertmanager `groupKey` is stable across re-fires of the
//! same alert group (every `repeat_interval`, default 4h). We hash it
//! into a short id so clients can recognise the re-fire and update their
//! existing notification rather than stack a fresh one. Hashing rather
//! than passing through directly because the raw groupKey contains `"`,
//! `{`, `}` etc. and is awkward as a notification identifier.
//!
//! Author note: the translation is intentionally lossy. The whole point
//! of cross-notifier is "is this important enough to look at my phone for"
//! — full alert payload context belongs in Alertmanager / Grafana, which
//! we link to via the action buttons.

use std::collections::HashMap;

use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::protocol::{Action, Notification};

/// One Alertmanager v4 webhook payload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertmanagerWebhook {
    /// Wire format version. Currently `"4"`. We accept any value — older
    /// versions of Alertmanager have shipped `"3"` and the field is more
    /// useful for debugging than for branching.
    #[serde(default)]
    pub version: String,
    /// Stable per-alert-group key. Used as the dedup seed.
    #[serde(default)]
    pub group_key: String,
    /// `"firing"` or `"resolved"` — applies to the *group* as a whole.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub receiver: String,
    #[serde(default)]
    pub group_labels: HashMap<String, String>,
    /// Labels shared by every alert in this group. The `severity` label
    /// here is what we map to cross-notifier status.
    #[serde(default)]
    pub common_labels: HashMap<String, String>,
    /// Annotations shared by every alert in this group. The `summary`
    /// and `description` keys here, when present, are preferred over
    /// per-alert ones for building the notification title/message.
    #[serde(default)]
    pub common_annotations: HashMap<String, String>,
    /// Alertmanager's own URL (used as a fallback action button so the
    /// operator can silence or inspect from the alert).
    ///
    /// Note: the wire field is `externalURL` (all-caps "URL"), inherited
    /// from Go's standard `URL`-initialism JSON tag convention. The
    /// `rename_all = "camelCase"` above would produce `externalUrl`,
    /// which silently mismatches — so we override explicitly.
    #[serde(default, rename = "externalURL")]
    pub external_url: String,
    #[serde(default)]
    pub alerts: Vec<Alert>,
}

/// One individual alert within a group.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    /// RFC3339 — kept as a string because we don't actually need to parse
    /// it for the translation; passing it through avoids a chrono parse
    /// failure rejecting an otherwise-valid payload.
    #[serde(default)]
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: String,
    /// Prometheus's URL for the originating query (the "graph" link).
    /// Used as the primary action button.
    ///
    /// Same casing quirk as `externalURL` — see the comment there.
    #[serde(default, rename = "generatorURL")]
    pub generator_url: String,
    #[serde(default)]
    pub fingerprint: String,
}

impl AlertmanagerWebhook {
    /// Lossy translation to a `Notification`. Returns `None` if the
    /// payload has nothing displayable (no summary, no alertname, no
    /// description) — that's the only "malformed" case worth rejecting.
    pub fn to_notification(&self) -> Option<Notification> {
        let title = self.derive_title()?;
        let message = self.derive_message();
        let status = self.cross_notifier_status();
        let actions = self.derive_actions();
        let id = self.derive_id();

        Some(Notification {
            id,
            source: "alertmanager".to_string(),
            title,
            message,
            status,
            actions,
            // Alerts are useful to keep in notification-center history
            // even after they auto-dismiss from the popup.
            store_on_expire: true,
            ..Default::default()
        })
    }

    fn cross_notifier_status(&self) -> String {
        if self.status.eq_ignore_ascii_case("resolved") {
            return "success".to_string();
        }
        match self
            .common_labels
            .get("severity")
            .map(|s| s.as_str())
            .unwrap_or("")
        {
            // `page` is the gothab convention from
            // docs/design/production-database.md. `critical` is the
            // Prometheus mixin convention. Treat both the same.
            "page" | "critical" => "error".to_string(),
            "warning" => "warning".to_string(),
            _ => "info".to_string(),
        }
    }

    fn derive_title(&self) -> Option<String> {
        // Preference order: common summary → first per-alert summary →
        // alertname. Anything else is a payload we can't title.
        if let Some(s) = non_empty(self.common_annotations.get("summary")) {
            return Some(s);
        }
        for alert in &self.alerts {
            if let Some(s) = non_empty(alert.annotations.get("summary")) {
                return Some(s);
            }
        }
        if let Some(name) = non_empty(self.common_labels.get("alertname")) {
            return Some(format!("Alert: {name}"));
        }
        // Last-ditch: just say something happened. Better an ugly
        // notification than a silent dropped page.
        if !self.alerts.is_empty() {
            return Some(format!("Alertmanager: {} alert(s)", self.alerts.len()));
        }
        None
    }

    fn derive_message(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        if self.status.eq_ignore_ascii_case("resolved") {
            parts.push("Resolved.".to_string());
        }

        if let Some(desc) = non_empty(self.common_annotations.get("description")) {
            parts.push(desc);
        } else {
            // Fall back to per-alert descriptions. For groups with many
            // alerts this can get long; cap at three to keep the
            // notification scannable. The action buttons let the operator
            // open Alertmanager for the full list.
            for alert in self.alerts.iter().take(3) {
                if let Some(desc) = non_empty(alert.annotations.get("description")) {
                    parts.push(desc);
                } else if let Some(instance) = non_empty(alert.labels.get("instance")) {
                    parts.push(format!("on {instance}"));
                }
            }
        }

        if self.alerts.len() > 3 {
            parts.push(format!("(+{} more)", self.alerts.len() - 3));
        } else if self.alerts.len() > 1 {
            parts.push(format!("({} alerts in group)", self.alerts.len()));
        }

        parts.join("\n")
    }

    fn derive_actions(&self) -> Vec<Action> {
        let mut actions = Vec::new();

        // Prometheus graph URL from the first alert with one. This points
        // at the underlying query so the operator can see the time series
        // that fired the alert.
        if let Some(url) = self
            .alerts
            .iter()
            .map(|a| a.generator_url.as_str())
            .find(|s| !s.is_empty())
        {
            actions.push(Action {
                label: "Open in Prometheus".to_string(),
                url: url.to_string(),
                open: true,
                ..Default::default()
            });
        }

        // Alertmanager itself (where you silence). Always included if we
        // know its URL — silencing is the most useful thing to do from a
        // phone while away from a desk.
        if !self.external_url.is_empty() {
            actions.push(Action {
                label: "Open Alertmanager".to_string(),
                url: self.external_url.clone(),
                open: true,
                ..Default::default()
            });
        }

        actions
    }

    fn derive_id(&self) -> String {
        // Stable, short, URL-safe id derived from groupKey. SHA-256 →
        // first 8 bytes → base64-url. 11 characters, plenty for collision
        // avoidance at our notification volume, and stable across process
        // restarts (unlike DefaultHasher / SipHash).
        //
        // If groupKey is empty (e.g. a hand-crafted test payload), the
        // id still derives deterministically — from the SHA of "" — and
        // dedup just collapses all empty-groupKey alerts together.
        let hash = Sha256::digest(self.group_key.as_bytes());
        let short = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&hash[..8]);
        format!("am-{short}")
    }
}

fn non_empty(s: Option<&String>) -> Option<String> {
    s.filter(|s| !s.is_empty()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a webhook with sensible defaults; tests override what they
    /// care about.
    fn sample_firing() -> AlertmanagerWebhook {
        AlertmanagerWebhook {
            version: "4".to_string(),
            group_key: "{}/{alertname=\"DiskFull\"}:{instance=\"db-1\"}".to_string(),
            status: "firing".to_string(),
            receiver: "cross-notifier".to_string(),
            group_labels: HashMap::new(),
            common_labels: HashMap::from([
                ("alertname".to_string(), "DiskFull".to_string()),
                ("severity".to_string(), "page".to_string()),
            ]),
            common_annotations: HashMap::from([
                (
                    "summary".to_string(),
                    "Disk usage on db-1 above 90%".to_string(),
                ),
                (
                    "description".to_string(),
                    "data dir at 91% — restore disk before WAL backs up".to_string(),
                ),
            ]),
            external_url: "https://alertmanager.example.com".to_string(),
            alerts: vec![Alert {
                status: "firing".to_string(),
                labels: HashMap::from([
                    ("alertname".to_string(), "DiskFull".to_string()),
                    ("instance".to_string(), "db-1".to_string()),
                    ("severity".to_string(), "page".to_string()),
                ]),
                annotations: HashMap::new(),
                starts_at: "2025-05-11T12:00:00Z".to_string(),
                ends_at: "0001-01-01T00:00:00Z".to_string(),
                generator_url: "https://prom.example.com/graph?...".to_string(),
                fingerprint: "abc123".to_string(),
            }],
        }
    }

    #[test]
    fn page_severity_maps_to_error() {
        let n = sample_firing().to_notification().unwrap();
        assert_eq!(n.status, "error");
        assert_eq!(n.title, "Disk usage on db-1 above 90%");
        assert!(n.message.contains("data dir at 91%"));
        assert_eq!(n.source, "alertmanager");
    }

    #[test]
    fn critical_severity_also_maps_to_error() {
        let mut w = sample_firing();
        w.common_labels
            .insert("severity".to_string(), "critical".to_string());
        assert_eq!(w.to_notification().unwrap().status, "error");
    }

    #[test]
    fn warning_severity_maps_to_warning() {
        let mut w = sample_firing();
        w.common_labels
            .insert("severity".to_string(), "warning".to_string());
        assert_eq!(w.to_notification().unwrap().status, "warning");
    }

    #[test]
    fn unknown_severity_falls_back_to_info() {
        let mut w = sample_firing();
        w.common_labels
            .insert("severity".to_string(), "notice".to_string());
        assert_eq!(w.to_notification().unwrap().status, "info");
    }

    #[test]
    fn missing_severity_falls_back_to_info() {
        let mut w = sample_firing();
        w.common_labels.remove("severity");
        assert_eq!(w.to_notification().unwrap().status, "info");
    }

    #[test]
    fn resolved_maps_to_success_regardless_of_severity() {
        let mut w = sample_firing();
        w.status = "resolved".to_string();
        // Severity stays "page" — but resolved wins.
        let n = w.to_notification().unwrap();
        assert_eq!(n.status, "success");
        assert!(n.message.starts_with("Resolved."));
    }

    #[test]
    fn title_falls_back_to_per_alert_summary_then_alertname() {
        let mut w = sample_firing();
        w.common_annotations.remove("summary");
        // Per-alert summary should win over alertname.
        w.alerts[0]
            .annotations
            .insert("summary".to_string(), "per-alert summary".to_string());
        assert_eq!(
            w.to_notification().unwrap().title,
            "per-alert summary"
        );

        // Remove that too — alertname should kick in.
        w.alerts[0].annotations.remove("summary");
        assert_eq!(w.to_notification().unwrap().title, "Alert: DiskFull");
    }

    #[test]
    fn returns_some_even_when_only_alerts_count_is_known() {
        let mut w = sample_firing();
        w.common_annotations.remove("summary");
        w.common_labels.remove("alertname");
        w.alerts[0].labels.remove("alertname");
        w.alerts[0].annotations.remove("summary");
        // Should still produce a notification — "Alertmanager: 1 alert(s)".
        let n = w.to_notification().unwrap();
        assert!(n.title.starts_with("Alertmanager:"));
    }

    #[test]
    fn returns_none_when_completely_empty() {
        let w = AlertmanagerWebhook {
            version: String::new(),
            group_key: String::new(),
            status: "firing".to_string(),
            receiver: String::new(),
            group_labels: HashMap::new(),
            common_labels: HashMap::new(),
            common_annotations: HashMap::new(),
            external_url: String::new(),
            alerts: vec![],
        };
        assert!(w.to_notification().is_none());
    }

    #[test]
    fn id_is_stable_across_invocations() {
        // The whole point of deriving from groupKey is that re-fires of
        // the same alert group get the same id. If this test ever fails,
        // dedup on the client side will silently break.
        let w1 = sample_firing();
        let w2 = sample_firing();
        assert_eq!(w1.derive_id(), w2.derive_id());
    }

    #[test]
    fn different_group_keys_get_different_ids() {
        let w1 = sample_firing();
        let mut w2 = sample_firing();
        w2.group_key = "{}/{alertname=\"OtherAlert\"}".to_string();
        assert_ne!(w1.derive_id(), w2.derive_id());
    }

    #[test]
    fn actions_link_to_prometheus_and_alertmanager() {
        let n = sample_firing().to_notification().unwrap();
        let labels: Vec<&str> = n.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, vec!["Open in Prometheus", "Open Alertmanager"]);
        assert!(n.actions[0].url.starts_with("https://prom"));
        assert!(n.actions[1].url.starts_with("https://alertmanager"));
        assert!(n.actions[0].open);
        assert!(n.actions[1].open);
    }

    #[test]
    fn message_summarises_multi_alert_group() {
        let mut w = sample_firing();
        w.common_annotations.remove("description");
        // Build a group of 5 alerts so the +N-more branch fires.
        for i in 0..4 {
            w.alerts.push(Alert {
                status: "firing".to_string(),
                labels: HashMap::from([(
                    "instance".to_string(),
                    format!("db-{i}"),
                )]),
                annotations: HashMap::new(),
                starts_at: String::new(),
                ends_at: String::new(),
                generator_url: String::new(),
                fingerprint: String::new(),
            });
        }
        let n = w.to_notification().unwrap();
        assert!(n.message.contains("+2 more"), "message: {}", n.message);
    }

    /// Deserialize from a representative Alertmanager v4 payload to make
    /// sure the struct shape matches the wire format exactly.
    #[test]
    fn deserializes_real_alertmanager_payload() {
        let raw = r#"{
            "version": "4",
            "groupKey": "{}/{alertname=\"DiskFull\"}:{instance=\"db-1\"}",
            "truncatedAlerts": 0,
            "status": "firing",
            "receiver": "cross-notifier",
            "groupLabels": {"alertname": "DiskFull"},
            "commonLabels": {"alertname": "DiskFull", "severity": "page"},
            "commonAnnotations": {
                "summary": "Disk usage on db-1 above 90%",
                "description": "data dir at 91%"
            },
            "externalURL": "https://alertmanager.example.com",
            "alerts": [{
                "status": "firing",
                "labels": {"alertname": "DiskFull", "instance": "db-1"},
                "annotations": {},
                "startsAt": "2025-05-11T12:00:00Z",
                "endsAt": "0001-01-01T00:00:00Z",
                "generatorURL": "https://prom.example.com/graph?...",
                "fingerprint": "abc123"
            }]
        }"#;
        let w: AlertmanagerWebhook = serde_json::from_str(raw).expect("deserialize");
        // Spot-check that the URL-initialism fields land where we expect
        // — these are renamed explicitly because serde's camelCase
        // rename produces `externalUrl`/`generatorUrl`, neither of which
        // match Alertmanager's wire format. If this test ever passes
        // with empty strings the actions in the resulting notification
        // will silently drop.
        assert_eq!(w.external_url, "https://alertmanager.example.com");
        assert_eq!(w.alerts[0].generator_url, "https://prom.example.com/graph?...");

        let n = w.to_notification().expect("translate");
        assert_eq!(n.title, "Disk usage on db-1 above 90%");
        assert_eq!(n.status, "error");
        // And the action buttons should have been built from those URLs.
        assert_eq!(n.actions.len(), 2);
        assert_eq!(n.actions[0].url, "https://prom.example.com/graph?...");
        assert_eq!(n.actions[1].url, "https://alertmanager.example.com");
    }
}
