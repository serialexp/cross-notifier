//! Abstraction over "something that wants outbound messages from the core":
//! a WebSocket client is the obvious case, but the desktop daemon also
//! subscribes in-process so POSTed notifications appear on its own screen
//! without round-tripping through a WebSocket.

use tokio::sync::mpsc;

use crate::protocol::{ExpiredMessage, Notification, ResolvedMessage};

/// A message the core wants to deliver to a subscriber. Each subscriber
/// decides how to serialize it — WebSockets wrap it in a [`crate::Message`]
/// envelope; a local in-process subscriber can pattern-match directly.
#[derive(Debug, Clone)]
pub enum OutboundMessage {
    Notification(Notification),
    Resolved(ResolvedMessage),
    Expired(ExpiredMessage),
}

/// A handle the core uses to push messages at one subscriber. The core
/// clones the sender on registration and drops it (removing the subscriber)
/// when the send fails — subscribers don't need to de-register explicitly.
#[derive(Debug, Clone)]
pub struct Subscriber {
    pub name: String,
    pub tx: mpsc::UnboundedSender<OutboundMessage>,
}

impl Subscriber {
    pub fn new(name: impl Into<String>) -> (Self, mpsc::UnboundedReceiver<OutboundMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Subscriber {
                name: name.into(),
                tx,
            },
            rx,
        )
    }
}
