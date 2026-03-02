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

    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub debug_font_metrics: bool,
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
