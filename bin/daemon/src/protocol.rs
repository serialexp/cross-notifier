// WebSocket message protocol matching the Go server.
// Message envelope with typed payloads for notification, action, and resolved messages.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageType {
    Notification,
    Action,
    Resolved,
    Expired,
    /// Sent once by the server, immediately after WS handshake. Lets the
    /// daemon learn which calendars (if any) the server is pushing
    /// reminders from, so the UI can show the user "you're getting
    /// notifications from X even though local calendar is off".
    ServerInfo,
}

/// Action sent from client to server when user clicks a notification action button.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ActionMessage {
    pub notification_id: String,
    pub action_index: usize,
}

/// Resolution broadcast from server to all clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMessage {
    pub notification_id: String,
    pub resolved_by: String,
    pub action_label: String,
    pub success: bool,
    #[serde(default)]
    pub error: String,
}

/// Expired broadcast from server when a notification's maxWait elapses
/// without any client resolving it. Actions should no longer be offered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpiredMessage {
    pub notification_id: String,
}

/// Server capability advertisement. Mirrors `ServerInfoMessage` in
/// `cross_notifier_core::protocol`; duplicated here only because the
/// daemon's protocol module predates the shared core types and other
/// messages haven't been migrated yet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfoMessage {
    #[serde(default)]
    pub calendars: Vec<ServerCalendarInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerCalendarInfo {
    pub kind: String,
    pub label: String,
    pub fingerprint: String,
}

impl Message {
    #[allow(dead_code)]
    pub fn encode<T: Serialize>(msg_type: MessageType, data: &T) -> anyhow::Result<String> {
        let msg = Message {
            msg_type,
            data: serde_json::to_value(data)?,
        };
        Ok(serde_json::to_string(&msg)?)
    }

    pub fn decode(raw: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(raw)?)
    }
}
