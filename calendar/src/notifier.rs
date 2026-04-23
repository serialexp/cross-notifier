//! Delivery abstraction. The scheduler knows *when* to fire but not *how*
//! to reach the user — that's the Notifier's job. Two impls are provided:
//!
//! * [`CoreNotifier`] — in-process delivery via `cross_notifier_core::CoreState`.
//!   Used by both the headless server and the daemon (which both embed
//!   CoreState); skips the HTTP hop a cross-process notifier would need.
//!
//! * [`RecordingNotifier`] — test double that just collects deliveries.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cross_notifier_core::{
    protocol::{Action, Notification},
    state::CoreState,
    subscriber::OutboundMessage,
};
use tokio::sync::Mutex;

use crate::types::PendingFire;

#[async_trait]
pub trait Notifier: Send + Sync {
    /// Deliver a reminder for a scheduled PendingFire. Implementations
    /// typically build a `Notification` tailored to reminder semantics
    /// (snooze/dismiss actions, exclusive flag) and then hand it off.
    async fn deliver(&self, pending: &PendingFire) -> anyhow::Result<()>;

    /// Deliver an already-constructed notification. Used for non-reminder
    /// outputs like the daily summary, where the calling code knows the
    /// exact shape of the notification it wants.
    async fn deliver_notification(&self, notification: Notification) -> anyhow::Result<()>;
}

// ── CoreNotifier ───────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CoreNotifierConfig {
    /// Root URL for action callbacks. The notifier appends `/snooze` and
    /// `/dismiss` to produce the per-action URLs stored on the Notification.
    pub action_base_url: String,
    /// Optional Bearer token to attach to action callback requests. `None`
    /// for localhost-only daemons; `Some(secret)` for a public server.
    pub action_auth: Option<String>,
    /// Default snooze duration in hours, baked into the snooze action label
    /// and payload. 4 by default ("Bart forgets it exists for 4 hours").
    pub snooze_hours: u32,
}

impl Default for CoreNotifierConfig {
    fn default() -> Self {
        Self {
            action_base_url: "http://127.0.0.1:9876/calendar/action".to_string(),
            action_auth: None,
            snooze_hours: 4,
        }
    }
}

/// Delivers reminders via an embedded `CoreState`. Mirrors what an HTTP
/// POST to `/notify` would do, minus the HTTP hop and the auth check
/// (we're the trusted caller).
pub struct CoreNotifier {
    state: CoreState,
    cfg: CoreNotifierConfig,
}

impl CoreNotifier {
    pub fn new(state: CoreState, cfg: CoreNotifierConfig) -> Self {
        Self { state, cfg }
    }

    /// Build the wire-level Notification we hand to CoreState. Kept
    /// separate so tests can assert on the exact payload.
    pub fn build_notification(&self, p: &PendingFire) -> Notification {
        let occ = &p.occurrence;
        let time = occ
            .event_start
            .with_timezone(&chrono::Local)
            .format("%H:%M")
            .to_string();
        let message = match occ.location.as_deref() {
            Some(loc) if !loc.is_empty() => format!("{time} · {loc}"),
            _ => time,
        };

        let base = self.cfg.action_base_url.trim_end_matches('/');
        let mut headers: HashMap<String, String> = HashMap::new();
        if let Some(token) = &self.cfg.action_auth {
            headers.insert("Authorization".into(), format!("Bearer {token}"));
        }
        headers.insert("Content-Type".into(), "application/json".into());

        let snooze_body = serde_json::json!({
            "occurrenceId": occ.id,
            "hours": self.cfg.snooze_hours,
        })
        .to_string();
        let dismiss_body = serde_json::json!({ "occurrenceId": occ.id }).to_string();

        Notification {
            // Reusing the occurrence id as the notification id means a
            // snooze re-fire replaces the prior card cleanly on clients
            // that dedupe by id.
            id: occ.id.clone(),
            source: "calendar".to_string(),
            title: if occ.summary.is_empty() {
                "(untitled event)".to_string()
            } else {
                occ.summary.clone()
            },
            message,
            actions: vec![
                Action {
                    label: format!("Snooze {}h", self.cfg.snooze_hours),
                    method: "POST".into(),
                    url: format!("{base}/snooze"),
                    body: snooze_body,
                    headers: headers.clone(),
                    ..Default::default()
                },
                Action {
                    label: "Dismiss".into(),
                    method: "POST".into(),
                    url: format!("{base}/dismiss"),
                    body: dismiss_body,
                    headers,
                    ..Default::default()
                },
            ],
            exclusive: true,
            store_on_expire: true,
            ..Default::default()
        }
    }
}

#[async_trait]
impl Notifier for CoreNotifier {
    async fn deliver(&self, pending: &PendingFire) -> anyhow::Result<()> {
        let n = self.build_notification(pending);
        self.deliver_notification(n).await
    }

    async fn deliver_notification(&self, n: Notification) -> anyhow::Result<()> {
        self.state
            .broadcast(OutboundMessage::Notification(n.clone()))
            .await;
        if self.state.apns().is_some() && self.state.devices().is_some() {
            // Fire-and-forget, matching /notify's handler: don't let a slow
            // APNS round-trip stall the scheduler tick.
            let state = self.state.clone();
            let n = n.clone();
            tokio::spawn(async move {
                state.dispatch_push(&n).await;
            });
        }
        Ok(())
    }
}

// ── RecordingNotifier (tests) ──────────────────────────────────────────

#[derive(Default)]
pub struct RecordingNotifier {
    pub records: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Notifier for RecordingNotifier {
    async fn deliver(&self, pending: &PendingFire) -> anyhow::Result<()> {
        self.records
            .lock()
            .await
            .push(pending.occurrence.id.clone());
        Ok(())
    }

    async fn deliver_notification(&self, n: Notification) -> anyhow::Result<()> {
        // Prefix with "ntf:" so tests can tell reminder deliveries apart
        // from raw-notification deliveries when both paths run.
        self.records.lock().await.push(format!("ntf:{}", n.id));
        Ok(())
    }
}
