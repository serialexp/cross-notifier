// Application state and event types.
// Bridges the async tokio world (WebSocket, HTTP) with the synchronous winit event loop.

use crate::notification::NotificationPayload;
use crate::protocol::{ExpiredMessage, ResolvedMessage, ServerInfoMessage};

/// Events sent from async tasks to the winit event loop.
pub enum AppEvent {
    /// Notification received from server or local HTTP endpoint.
    IncomingNotification {
        server_label: String,
        payload: NotificationPayload,
    },

    /// Connection status changed for a server.
    /// `error` is `Some` when a disconnect was caused by a failure
    /// (e.g. auth rejection, DNS lookup failure, connection refused) —
    /// the UI surfaces it so the user knows *why* it won't connect.
    /// On successful connect, `error` is always `None`.
    ConnectionStatus {
        server_url: String,
        connected: bool,
        error: Option<String>,
    },

    /// Server advertised its capabilities right after WS handshake.
    /// Stored against the connection so the settings UI can show what
    /// the server is doing on its end (e.g. pushing calendar reminders).
    ServerInfoReceived {
        server_url: String,
        info: ServerInfoMessage,
    },

    /// Exclusive notification was resolved by another client.
    NotificationResolved(ResolvedMessage),

    /// Exclusive notification's maxWait elapsed without resolution.
    /// TODO: keep card visible with a disabled "Timed out" pill instead
    /// of dismissing — see TODO.md.
    NotificationExpired(ExpiredMessage),

    /// Icon fetched asynchronously (from URL), ready for GPU upload.
    IconLoaded {
        notification_id: i64,
        image: image::RgbaImage,
    },

    /// Toggle the notification center window.
    ToggleCenter,

    /// Notification store changed externally (e.g. via HTTP endpoint).
    CenterDirty,

    /// Request to open the settings window.
    OpenSettings,

    /// Config file changed on disk — reload it.
    ConfigChanged,

    /// User requested manual reconnect for a specific server.
    ReconnectServer { url: String },

    /// Request to quit the application.
    Quit,
}

impl std::fmt::Debug for AppEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncomingNotification { server_label, .. } => {
                write!(f, "IncomingNotification({})", server_label)
            }
            Self::ConnectionStatus {
                server_url,
                connected,
                error,
            } => {
                write!(
                    f,
                    "ConnectionStatus({}, {}{})",
                    server_url,
                    connected,
                    error
                        .as_deref()
                        .map(|e| format!(", err={}", e))
                        .unwrap_or_default()
                )
            }
            Self::ServerInfoReceived { server_url, info } => {
                write!(
                    f,
                    "ServerInfoReceived({}, calendars={})",
                    server_url,
                    info.calendars.len()
                )
            }
            Self::NotificationResolved(r) => {
                write!(f, "NotificationResolved({})", r.notification_id)
            }
            Self::NotificationExpired(e) => {
                write!(f, "NotificationExpired({})", e.notification_id)
            }
            Self::IconLoaded {
                notification_id, ..
            } => {
                write!(f, "IconLoaded({})", notification_id)
            }
            Self::ToggleCenter => write!(f, "ToggleCenter"),
            Self::CenterDirty => write!(f, "CenterDirty"),
            Self::OpenSettings => write!(f, "OpenSettings"),
            Self::ConfigChanged => write!(f, "ConfigChanged"),
            Self::ReconnectServer { url } => write!(f, "ReconnectServer({})", url),
            Self::Quit => write!(f, "Quit"),
        }
    }
}
