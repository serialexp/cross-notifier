// Configuration types matching the Go daemon's JSON format.
// Ensures config compatibility so users can switch between Go and Rust daemons.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<Server>,

    #[serde(default)]
    pub rules: RulesConfig,

    #[serde(default)]
    pub center_panel: CenterPanelConfig,

    /// Optional calendar integration. `None` (or omitted) disables the
    /// calendar pipeline — no fetches, no reminders. Configuring this
    /// at all means the daemon locally watches a calendar and produces
    /// reminder notifications via its embedded notification core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<CalendarConfig>,

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub debug_font_metrics: bool,
}

/// Calendar integration settings persisted into the daemon's config.json.
///
/// Exactly one source variant is active at a time; the UI mirrors that by
/// letting the user pick a kind and only exposing the relevant fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CalendarSource {
    #[serde(rename = "caldav")]
    CalDav {
        endpoint: String,
        user: String,
        password: String,
    },
    #[serde(rename = "icsUrl")]
    IcsUrl {
        url: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        user: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        password: String,
    },
    #[serde(rename = "icsFile")]
    IcsFile { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarConfig {
    /// Required. Where events come from.
    pub source: CalendarSource,

    /// Look-ahead window in hours (default 48). Events further out than
    /// this aren't scheduled yet; the next refresh will pick them up as
    /// they come into range.
    #[serde(default = "default_horizon_hours")]
    pub horizon_hours: u32,

    /// How often to re-fetch the calendar (default 5).
    #[serde(default = "default_refresh_minutes")]
    pub refresh_minutes: u32,

    /// Default snooze duration, in hours, for the snooze action button
    /// (default 4).
    #[serde(default = "default_snooze_hours")]
    pub snooze_hours: u32,

    /// Opt-in daily-summary: a single "tomorrow's agenda" notification
    /// fired at the configured local wall-clock time. `None` disables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_summary: Option<DailySummarySettings>,
}

/// Config payload for the daily summary. Hour is 0..=23, minute 0..=59.
/// We don't serialise a separate "enabled" flag — presence of the object
/// in config is the enable.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DailySummarySettings {
    #[serde(default = "default_summary_hour")]
    pub hour: u32,
    #[serde(default)]
    pub minute: u32,
}

fn default_summary_hour() -> u32 {
    12
}

impl Default for DailySummarySettings {
    fn default() -> Self {
        Self { hour: 12, minute: 0 }
    }
}

fn default_horizon_hours() -> u32 {
    48
}
fn default_refresh_minutes() -> u32 {
    5
}
fn default_snooze_hours() -> u32 {
    4
}

impl Default for CalendarConfig {
    fn default() -> Self {
        // Reasonable blank placeholder the UI can edit in place. Source
        // URL must be filled in by the user before the daemon spawns the
        // service (empty URL → spawn skipped with a warning).
        Self {
            source: CalendarSource::IcsUrl {
                url: String::new(),
                user: String::new(),
                password: String::new(),
            },
            horizon_hours: default_horizon_hours(),
            refresh_minutes: default_refresh_minutes(),
            snooze_hours: default_snooze_hours(),
            daily_summary: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Server {
    pub url: String,
    pub secret: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RulesConfig {
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<NotificationRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationRule {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pattern: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sound: String,

    #[serde(default, skip_serializing_if = "RuleAction::is_normal")]
    pub action: RuleAction,

    /// Deprecated: use action = "dismiss" instead
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppress: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    #[default]
    Normal,
    Silent,
    Dismiss,
}

impl RuleAction {
    pub fn is_normal(&self) -> bool {
        matches!(self, RuleAction::Normal)
    }
}

impl NotificationRule {
    /// Returns the effective action, handling backward compatibility with `suppress`.
    pub fn effective_action(&self) -> &RuleAction {
        if self.suppress {
            return &RuleAction::Dismiss;
        }
        &self.action
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CenterPanelConfig {
    #[serde(default)]
    pub respect_work_area_top: bool,

    #[serde(default)]
    pub respect_work_area_bottom: bool,
}

impl Config {
    pub fn path() -> PathBuf {
        let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        config_dir.join("cross-notifier").join("config.json")
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let cfg: Config = serde_json::from_str(&data)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }
}
