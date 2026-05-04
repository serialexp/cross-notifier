// Notification data model matching the Go daemon's JSON protocol.
// Handles notification lifecycle: creation, expiry, dismissal.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub label: String,

    #[serde(default)]
    pub url: String,

    #[serde(default)]
    pub method: String,

    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,

    #[serde(default)]
    pub body: String,

    #[serde(default)]
    pub open: bool,
}

/// Notification as received from the server or local HTTP endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPayload {
    #[serde(default)]
    pub id: String,

    #[serde(default)]
    pub source: String,

    #[serde(default)]
    pub title: String,

    #[serde(default)]
    pub message: String,

    /// One of: info, success, warning, error
    #[serde(default)]
    pub status: String,

    /// Base64-encoded image data (highest priority icon source).
    #[serde(default)]
    pub icon_data: String,

    /// URL to fetch icon from (medium priority).
    #[serde(default)]
    pub icon_href: String,

    /// Local file path for icon (lowest priority, daemon-only).
    #[serde(default)]
    pub icon_path: String,

    /// Display duration in seconds. >0 = auto-close, <=0 = persistent.
    #[serde(default)]
    pub duration: i32,

    #[serde(default)]
    pub actions: Vec<Action>,

    /// If true, resolved when any client takes action.
    #[serde(default)]
    pub exclusive: bool,

    /// If true, persist to notification center after popup expires.
    /// Defaults to true — set to false to suppress center storage.
    #[serde(default = "default_true")]
    pub store_on_expire: bool,
}

/// Internal notification with local state for rendering and lifecycle.
#[allow(dead_code)]
pub struct Notification {
    /// Local sequential ID.
    pub id: i64,

    /// Server-assigned ID (for exclusive notifications).
    pub server_id: String,

    /// Label of the server that sent this.
    pub server_label: String,

    /// Original payload.
    pub payload: NotificationPayload,

    /// Decoded icon image (if any, before GPU upload).
    pub icon: Option<image::RgbaImage>,

    /// GPU bind group for the icon texture (set after upload).
    pub icon_bind_group: Option<Arc<wgpu::BindGroup>>,

    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,

    /// UI state: expanded to show full message.
    pub expanded: bool,

    /// Action button states.
    pub action_states: Vec<ActionState>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum ActionState {
    Idle,
    Loading,
    Success,
    Error,
}

impl Notification {
    pub fn from_payload(local_id: i64, server_label: String, payload: NotificationPayload) -> Self {
        let now = Utc::now();
        let expires_at = if payload.duration > 0 {
            Some(now + chrono::Duration::seconds(payload.duration as i64))
        } else {
            None
        };
        let action_count = payload.actions.len();

        Self {
            id: local_id,
            server_id: payload.id.clone(),
            server_label,
            payload,
            icon: None,
            icon_bind_group: None,
            created_at: now,
            expires_at,
            expanded: false,
            action_states: vec![ActionState::Idle; action_count],
        }
    }

    pub fn is_expired(&self) -> bool {
        if self.expanded {
            return false; // expanded notifications don't auto-expire
        }
        match self.expires_at {
            Some(expires) => Utc::now() >= expires,
            None => false,
        }
    }

    pub fn title(&self) -> &str {
        &self.payload.title
    }

    pub fn message(&self) -> &str {
        &self.payload.message
    }

    pub fn status(&self) -> &str {
        &self.payload.status
    }

    pub fn source(&self) -> &str {
        &self.payload.source
    }
}

/// Manages the active notification queue.
pub struct NotificationQueue {
    notifications: Vec<Notification>,
    next_id: i64,
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
            next_id: 1,
        }
    }

    pub fn add(&mut self, server_label: String, payload: NotificationPayload) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let notification = Notification::from_payload(id, server_label, payload);
        self.notifications.push(notification);
        id
    }

    pub fn dismiss(&mut self, id: i64) -> Option<Notification> {
        if let Some(pos) = self.notifications.iter().position(|n| n.id == id) {
            Some(self.notifications.remove(pos))
        } else {
            None
        }
    }

    /// Remove expired notifications, returning them for potential center storage.
    pub fn prune_expired(&mut self) -> Vec<Notification> {
        let mut expired = Vec::new();
        let mut i = 0;
        while i < self.notifications.len() {
            if self.notifications[i].is_expired() {
                expired.push(self.notifications.remove(i));
            } else {
                i += 1;
            }
        }
        expired
    }

    pub fn visible(&self) -> &[Notification] {
        &self.notifications
    }

    pub fn get_mut(&mut self, id: i64) -> Option<&mut Notification> {
        self.notifications.iter_mut().find(|n| n.id == id)
    }

    pub fn find_by_server_id(&self, server_id: &str) -> Option<i64> {
        self.notifications
            .iter()
            .find(|n| n.server_id == server_id)
            .map(|n| n.id)
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.notifications.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notifications.is_empty()
    }
}
