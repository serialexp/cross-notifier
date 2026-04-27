//! Wire-format types shared by the HTTP and WebSocket surfaces. These types
//! match the JSON schemas published in `openapi.yaml` — keep the two in sync.

use serde::{Deserialize, Serialize};

/// Identifies the kind of envelope body on a WebSocket message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageType {
    Notification,
    Action,
    Resolved,
    Expired,
    /// Server → client, sent once immediately after WS handshake. Lets a
    /// new client see what the server is configured to do (e.g. which
    /// calendars it's pushing reminders for) without having to re-derive
    /// it from incoming traffic.
    ServerInfo,
}

/// Envelope for all WebSocket traffic between server and clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub data: serde_json::Value,
}

impl Message {
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

/// A clickable action button attached to a notification. Either triggers an
/// HTTP request server-side (for exclusive notifications) or is handled
/// locally by whichever client owns the notification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub label: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub headers: std::collections::HashMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub open: bool,
}

impl Action {
    /// Returns the HTTP method to use, uppercased. Defaults to GET.
    pub fn effective_method(&self) -> String {
        if self.method.is_empty() {
            "GET".to_string()
        } else {
            self.method.to_uppercase()
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A notification to display on clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon_data: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon_href: String,
    /// Local filesystem path for the icon. Only meaningful when sender
    /// and receiver share a filesystem (i.e. localhost daemon); the
    /// headless server treats it as opaque pass-through.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon_path: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub duration: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exclusive: bool,
    /// Seconds the initial `POST /notify` blocks waiting for a client
    /// response. Any non-zero value implies `exclusive`.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub wait: u64,
    /// Total lifetime in seconds; after this we broadcast `expired`.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub max_wait: u64,
    /// Daemon-side hint: persist to notification center after the popup
    /// auto-dismisses. Opaque to the server.
    #[serde(default, skip_serializing_if = "is_false")]
    pub store_on_expire: bool,
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}
fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// Client → server: user clicked action `action_index` on notification `notification_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionMessage {
    pub notification_id: String,
    pub action_index: usize,
}

/// Server → clients: an exclusive notification was resolved by someone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMessage {
    #[serde(rename = "id")]
    pub notification_id: String,
    pub resolved_by: String,
    pub action_label: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// Server → clients: an exclusive notification's `maxWait` elapsed with no
/// resolver. Clients should stop offering its action buttons.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpiredMessage {
    #[serde(rename = "id")]
    pub notification_id: String,
}

/// Server → client: capabilities advertisement. Sent once per WS
/// connection, immediately after handshake. Empty fields mean "feature
/// not configured here" — clients should treat absence as "the server
/// will not push that kind of traffic".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfoMessage {
    /// Calendars the server is pushing reminders from. Multiple entries
    /// are allowed for forward-compat; today the server has a single
    /// source so this is `len() <= 1`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<ServerCalendarInfo>,
}

/// One advertised calendar feed. The server intentionally does NOT
/// include the URL or any credentials — clients only get a stable
/// fingerprint they can compare against their own local config plus a
/// human-readable label and kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCalendarInfo {
    /// Source kind: "caldav" | "ics_url" | "ics_file". Stable wire form.
    pub kind: String,
    /// Human-readable label suitable for display ("Work CalDAV", etc).
    pub label: String,
    /// Hex fingerprint of the canonical (credential-free) source id.
    /// Both ends compute it the same way so equality means "same
    /// calendar".
    pub fingerprint: String,
}

/// `POST /notify` response body when the short `wait` elapsed but the
/// notification is still live — sender should fall back to long-polling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingResponse {
    pub id: String,
    pub status: String,
}

impl PendingResponse {
    pub fn new(id: String) -> Self {
        Self {
            id,
            status: "pending".to_string(),
        }
    }
}
