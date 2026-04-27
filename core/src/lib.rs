//! Shared notification server logic used by both the headless `cross-notifier-server`
//! binary and the local HTTP server embedded in the desktop daemon.
//!
//! Exposes an axum [`Router`] factory so consumers can mount the notification
//! endpoints under their own path, alongside any extra routes they need.

pub mod action;
pub mod device;
pub mod icon;
pub mod openapi;
pub mod protocol;
pub mod push;
pub mod router;
pub mod state;
pub mod subscriber;

pub use protocol::{
    Action, ActionMessage, ExpiredMessage, Message, MessageType, Notification, PendingResponse,
    ResolvedMessage, ServerCalendarInfo, ServerInfoMessage,
};
pub use router::router;
pub use state::CoreState;
pub use subscriber::{OutboundMessage, Subscriber};
