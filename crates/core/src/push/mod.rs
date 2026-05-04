//! Push-notification delivery for devices that can't hold a WebSocket open
//! (iOS backgrounded apps, eventually Android via FCM).
//!
//! Everything here is opt-in: if no provider is configured, `PushService`
//! is absent from `CoreState` and `/notify` works exactly as before.

pub mod apns;

#[cfg(any(test, feature = "test-util"))]
pub mod mock;

pub use apns::{ApnsClient, ApnsConfig, ApnsEnvironment, ApnsKey, PushError, PushOutcome};
