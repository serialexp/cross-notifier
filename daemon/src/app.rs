// Application state and event types.
// Bridges the async tokio world (WebSocket, HTTP) with the synchronous winit event loop.

use crate::notification::NotificationPayload;
use crate::protocol::ResolvedMessage;

/// Events sent from async tasks to the winit event loop.
pub enum AppEvent {
    /// Notification received from server or local HTTP endpoint.
    IncomingNotification {
        server_label: String,
        payload: NotificationPayload,
    },

    /// Connection status changed for a server.
    ConnectionStatus {
        server_url: String,
        connected: bool,
    },

    /// Exclusive notification was resolved by another client.
    NotificationResolved(ResolvedMessage),

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

    /// Request to quit the application.
    Quit,
}

impl std::fmt::Debug for AppEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncomingNotification { server_label, .. } => {
                write!(f, "IncomingNotification({})", server_label)
            }
            Self::ConnectionStatus { server_url, connected } => {
                write!(f, "ConnectionStatus({}, {})", server_url, connected)
            }
            Self::NotificationResolved(r) => {
                write!(f, "NotificationResolved({})", r.notification_id)
            }
            Self::IconLoaded { notification_id, .. } => {
                write!(f, "IconLoaded({})", notification_id)
            }
            Self::ToggleCenter => write!(f, "ToggleCenter"),
            Self::CenterDirty => write!(f, "CenterDirty"),
            Self::OpenSettings => write!(f, "OpenSettings"),
            Self::ConfigChanged => write!(f, "ConfigChanged"),
            Self::Quit => write!(f, "Quit"),
        }
    }
}
