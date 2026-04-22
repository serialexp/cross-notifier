//! Central mutable state for the core notification server: the set of
//! connected subscribers and the map of exclusive notifications awaiting
//! resolution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, watch};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::action::{ActionError, execute_http_action};
use crate::device::{DeviceRegistry, Platform};
use crate::protocol::{
    Action, ActionMessage, ExpiredMessage, Notification, ResolvedMessage,
};
use crate::push::{ApnsClient, PushError, PushOutcome};
use crate::subscriber::{OutboundMessage, Subscriber};

/// Short wait used when a caller sets `wait=0` but the notification is
/// exclusive. Small enough to sit under typical reverse-proxy idle timeouts.
pub const DEFAULT_WAIT_SECS: u64 = 25;

/// Terminal state of a pending notification, observable via a `watch`
/// channel so an arbitrary number of waiters can see the same result.
#[derive(Debug, Clone)]
pub enum PendingState {
    /// Still waiting for a client response.
    Live,
    /// A client clicked an action that resolved the notification.
    Resolved(ResolvedMessage),
    /// `max_wait` elapsed before any client resolved it.
    Expired,
}

struct PendingEntry {
    notif: Notification,
    /// Watch channel: writers publish the terminal state once, readers
    /// subscribe and await a non-Live value.
    state_tx: watch::Sender<PendingState>,
    /// Cancels the max-wait timer when resolution wins the race.
    #[allow(dead_code)]
    timer: JoinHandle<()>,
}

/// Shared, clone-cheap handle to the core server state.
#[derive(Clone)]
pub struct CoreState {
    inner: Arc<CoreInner>,
}

struct CoreInner {
    secret: String,
    subscribers: RwLock<Vec<Subscriber>>,
    pending: Mutex<HashMap<String, Arc<PendingEntry>>>,
    http: reqwest::Client,
    /// Optional registry of mobile push targets. `None` ⇒ no /devices
    /// endpoints and no push dispatch.
    devices: Option<DeviceRegistry>,
    /// Optional APNS client. Requires `devices` to also be set for push
    /// dispatch to do anything useful.
    apns: Option<ApnsClient>,
}

impl CoreState {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(CoreInner {
                secret: secret.into(),
                subscribers: RwLock::new(Vec::new()),
                pending: Mutex::new(HashMap::new()),
                http: reqwest::Client::new(),
                devices: None,
                apns: None,
            }),
        }
    }

    /// Attach a device registry. Only meaningful when combined with a
    /// push provider — the registry without APNS is just a list.
    pub fn with_device_registry(mut self, reg: DeviceRegistry) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("CoreState::with_device_registry called after cloning")
            .devices = Some(reg);
        self
    }

    /// Attach an APNS push client.
    pub fn with_apns(mut self, apns: ApnsClient) -> Self {
        Arc::get_mut(&mut self.inner)
            .expect("CoreState::with_apns called after cloning")
            .apns = Some(apns);
        self
    }

    pub fn devices(&self) -> Option<&DeviceRegistry> {
        self.inner.devices.as_ref()
    }

    pub fn apns(&self) -> Option<&ApnsClient> {
        self.inner.apns.as_ref()
    }

    pub fn secret(&self) -> &str {
        &self.inner.secret
    }

    /// Registers a new subscriber. Returns a handle that, when dropped,
    /// does NOT unregister — the subscriber is cleaned up lazily the next
    /// time a broadcast fails to send.
    pub async fn add_subscriber(&self, sub: Subscriber) {
        self.inner.subscribers.write().await.push(sub);
    }

    /// Number of currently-registered subscribers. For logging/tests.
    pub async fn subscriber_count(&self) -> usize {
        self.inner.subscribers.read().await.len()
    }

    /// Sends `msg` to every subscriber. Subscribers whose channels are
    /// closed are removed in place — the next `add_subscriber` reuses
    /// the slot only via append, but removal keeps the Vec small.
    pub async fn broadcast(&self, msg: OutboundMessage) {
        let mut subs = self.inner.subscribers.write().await;
        subs.retain(|s| s.tx.send(msg.clone()).is_ok());
    }

    /// Registers an exclusive notification and starts its max-wait timer.
    /// Returns a watch receiver the HTTP handler can await for the
    /// terminal state (resolved or expired).
    pub async fn register_pending(
        &self,
        notif: Notification,
        max_wait: Duration,
    ) -> watch::Receiver<PendingState> {
        let id = notif.id.clone();
        let (state_tx, state_rx) = watch::channel(PendingState::Live);

        let timer_state = self.clone();
        let timer_id = id.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(max_wait).await;
            timer_state.expire(&timer_id).await;
        });

        let entry = Arc::new(PendingEntry {
            notif,
            state_tx,
            timer,
        });
        self.inner.pending.lock().await.insert(id, entry);
        state_rx
    }

    /// Returns a watch receiver for an existing pending notification, or
    /// None if unknown (never registered, already resolved, or expired
    /// and cleaned up).
    pub async fn watch_pending(&self, id: &str) -> Option<watch::Receiver<PendingState>> {
        self.inner
            .pending
            .lock()
            .await
            .get(id)
            .map(|e| e.state_tx.subscribe())
    }

    /// Called by the WebSocket handler when a client clicks an action.
    /// Looks up the pending notification, runs the HTTP side-effect, and
    /// publishes the terminal state. Does nothing if the notification is
    /// unknown or already finished.
    pub async fn handle_action(&self, client_name: &str, msg: ActionMessage) {
        let entry = {
            let pending = self.inner.pending.lock().await;
            match pending.get(&msg.notification_id) {
                Some(e) => e.clone(),
                None => {
                    warn!(id = %msg.notification_id, "action for unknown notification");
                    return;
                }
            }
        };

        let action: Action = match entry.notif.actions.get(msg.action_index) {
            Some(a) => a.clone(),
            None => {
                warn!(
                    id = %msg.notification_id,
                    idx = msg.action_index,
                    "invalid action index"
                );
                return;
            }
        };

        let display_name = if client_name.is_empty() {
            "anonymous"
        } else {
            client_name
        };
        debug!(
            client = display_name,
            action = %action.label,
            id = %msg.notification_id,
            "executing action",
        );

        let exec_err = if action.open {
            // Open-in-browser actions are resolved client-side; the server
            // just reports success so exclusive coordination still works.
            None
        } else {
            execute_http_action(&self.inner.http, &action).await.err()
        };

        let resolved = ResolvedMessage {
            notification_id: msg.notification_id.clone(),
            resolved_by: client_name.to_string(),
            action_label: action.label.clone(),
            success: exec_err.is_none(),
            error: exec_err
                .as_ref()
                .map(ActionError::to_string)
                .unwrap_or_default(),
        };

        // Atomically publish-and-delete. The first caller (either us or
        // the expiry timer) that finds the entry still in the map and
        // still Live wins; the other becomes a no-op.
        let won = self.finish(&msg.notification_id, PendingState::Resolved(resolved.clone())).await;
        if !won {
            return;
        }

        self.broadcast(OutboundMessage::Resolved(resolved)).await;
    }

    /// Invoked by the max-wait timer. Publishes [`PendingState::Expired`]
    /// and broadcasts an `ExpiredMessage` iff this is the first terminal
    /// event (i.e. the notification wasn't already resolved).
    pub async fn expire(&self, id: &str) {
        let expired = ExpiredMessage {
            notification_id: id.to_string(),
        };
        if !self.finish(id, PendingState::Expired).await {
            return;
        }
        tracing::info!(id, "notification expired");
        self.broadcast(OutboundMessage::Expired(expired)).await;
    }

    /// Atomically: if `id` is still pending and Live, write `state` into
    /// its watch channel, remove it from the map, and return true.
    /// Returns false if no longer pending or already finished.
    async fn finish(&self, id: &str, state: PendingState) -> bool {
        let mut pending = self.inner.pending.lock().await;
        let Some(entry) = pending.get(id).cloned() else {
            return false;
        };

        // watch::Sender::send_if_modified gives us the "only-once" guard.
        let mut fired = false;
        entry.state_tx.send_if_modified(|cur| {
            if matches!(cur, PendingState::Live) {
                *cur = state.clone();
                fired = true;
                true
            } else {
                false
            }
        });

        if fired {
            pending.remove(id);
            entry.timer.abort();
        }
        fired
    }

    /// Returns the stored notification payload for a pending ID. Used when
    /// a new subscriber connects and we want to re-send live exclusive
    /// notifications — currently unused but kept for future resync work.
    #[allow(dead_code)]
    pub async fn pending_notification(&self, id: &str) -> Option<Notification> {
        self.inner
            .pending
            .lock()
            .await
            .get(id)
            .map(|e| e.notif.clone())
    }

    /// Snapshot of the number of live pending notifications (tests only).
    pub async fn pending_count(&self) -> usize {
        self.inner.pending.lock().await.len()
    }

    /// Fire-and-forget push to every registered iOS device. No-op unless
    /// BOTH a registry and an APNS client are configured.
    ///
    /// Runs sequentially (one token at a time) to keep APNS happy — it
    /// reuses the same HTTP/2 connection and hates flood starts. For the
    /// single-tenant scale we're aiming for that's fine; if it ever
    /// matters we can bound-concurrent.
    pub async fn dispatch_push(&self, n: &Notification) {
        let (registry, apns) = match (self.devices(), self.apns()) {
            (Some(r), Some(a)) => (r.clone(), a.clone()),
            _ => return,
        };

        // Exclusive notifications rely on client-side resolution, which
        // backgrounded iOS can't do reliably. Skip them on mobile.
        if n.exclusive {
            debug!(
                id = %n.id,
                "skipping APNS dispatch for exclusive notification",
            );
            return;
        }

        let devices = registry.list_for(Platform::Ios).await;
        if devices.is_empty() {
            return;
        }

        let payload = ApnsClient::build_payload(n);
        let mut dead: Vec<String> = Vec::new();
        let mut delivered = 0usize;

        for device in &devices {
            match apns.send(&device.token, &payload).await {
                Ok(PushOutcome::Delivered) => {
                    delivered += 1;
                    registry.record_push(&device.token).await;
                }
                Ok(PushOutcome::PruneToken(reason)) => {
                    warn!(
                        token_prefix = &device.token.chars().take(8).collect::<String>(),
                        reason, "APNS reported dead token, pruning",
                    );
                    dead.push(device.token.clone());
                }
                Ok(PushOutcome::Retryable(reason)) => {
                    warn!(
                        token_prefix = &device.token.chars().take(8).collect::<String>(),
                        reason, "APNS transient failure, will retry on next push",
                    );
                }
                Err(PushError::AuthRejected(reason)) => {
                    // Config problem — no point continuing this batch.
                    warn!(reason, "APNS auth rejected, aborting dispatch");
                    break;
                }
                Err(e) => {
                    warn!(error = %e, "APNS dispatch error");
                }
            }
        }

        if !dead.is_empty() {
            registry.remove_many(&dead).await;
        }

        debug!(
            attempted = devices.len(),
            delivered,
            pruned = dead.len(),
            "apns dispatch complete",
        );
    }
}
