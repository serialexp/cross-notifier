//! Scheduler: owns the set of pending fires, merges refreshes, handles
//! snooze/dismiss, and fires anything due.
//!
//! Runs as a single tokio task reading commands off an mpsc channel. One
//! command per tick — no inner locking needed. The channel lets the
//! orchestrator (service.rs), action handlers, and the periodic ticker
//! all push work into the same loop.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::notifier::Notifier;
use crate::store::{PendingMap, PendingStore};
use crate::types::{Occurrence, PendingFire};

/// Keep delivered-and-done entries around for this long after fire_at so
/// a subsequent calendar refresh doesn't resurrect them. Anything older
/// gets garbage-collected at tick time.
const RETAIN_AFTER_DONE: Duration = Duration::days(2);

pub enum SchedulerCmd {
    /// Replace the known set of future occurrences with these, merging
    /// scheduler state (fired_at / snoozed_until) where ids overlap.
    Refresh(Vec<Occurrence>),
    Snooze {
        occurrence_id: String,
        duration: Duration,
    },
    Dismiss {
        occurrence_id: String,
    },
    /// Manual tick for tests. Production uses the wall-clock ticker.
    Tick,
    /// Snapshot all pending fires, sorted by effective fire time (ascending).
    /// Includes already-delivered entries within the retention window so
    /// callers can display "just fired" as well as "upcoming."
    Inspect(oneshot::Sender<Vec<PendingFire>>),
    Shutdown,
}

#[derive(Clone)]
pub struct SchedulerHandle {
    tx: mpsc::UnboundedSender<SchedulerCmd>,
}

impl SchedulerHandle {
    pub fn send(&self, cmd: SchedulerCmd) -> anyhow::Result<()> {
        self.tx
            .send(cmd)
            .map_err(|_| anyhow::anyhow!("scheduler task has stopped"))
    }

    /// Ask the scheduler for a snapshot of its pending fires. The reply
    /// arrives once the command reaches the head of the queue, so the
    /// result is consistent with whatever refresh/snooze/tick commands
    /// were already in flight.
    pub async fn list_pending(&self) -> anyhow::Result<Vec<PendingFire>> {
        let (tx, rx) = oneshot::channel();
        self.send(SchedulerCmd::Inspect(tx))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("scheduler dropped the inspect reply"))
    }
}

pub struct Scheduler {
    pending: PendingMap,
    store: Arc<dyn PendingStore>,
    notifier: Arc<dyn Notifier>,
    /// Used in tests to freeze time. Production leaves this None.
    clock: Arc<Mutex<Option<DateTime<Utc>>>>,
}

impl Scheduler {
    /// Build a scheduler that's ready to run. `store` is loaded eagerly
    /// so startup merges with persisted state before any ticks fire.
    pub async fn new(
        store: Arc<dyn PendingStore>,
        notifier: Arc<dyn Notifier>,
    ) -> anyhow::Result<Self> {
        let pending = store.load().await.unwrap_or_else(|e| {
            warn!("scheduler: failed to load pending state ({e}); starting fresh");
            PendingMap::new()
        });
        Ok(Self {
            pending,
            store,
            notifier,
            clock: Arc::new(Mutex::new(None)),
        })
    }

    /// For tests: override `now()`.
    pub async fn set_clock(&self, now: DateTime<Utc>) {
        *self.clock.lock().await = Some(now);
    }

    async fn now(&self) -> DateTime<Utc> {
        self.clock.lock().await.unwrap_or_else(Utc::now)
    }

    /// Take ownership and spawn the loop. Returns a handle for enqueuing
    /// commands plus the task's `JoinHandle` so a supervisor can await a
    /// clean shutdown (via `SchedulerCmd::Shutdown`) before spawning a
    /// replacement scheduler.
    pub fn spawn(self) -> (SchedulerHandle, JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel::<SchedulerCmd>();
        let handle = SchedulerHandle { tx: tx.clone() };
        // Periodic tick at 30s resolution. Calendar reminders don't need
        // second-level precision and 30s keeps the loop quiet.
        let join = tokio::spawn(run_loop(self, rx, tx));
        (handle, join)
    }

    // ── Command handlers ──────────────────────────────────────────────

    async fn on_refresh(&mut self, fresh: Vec<Occurrence>) {
        let now = self.now().await;
        let fresh_ids: HashSet<String> = fresh.iter().map(|o| o.id.clone()).collect();

        // Drop anything the calendar no longer contains — unless it's an
        // already-delivered entry within the retention window (we don't
        // want to re-resurrect it if the calendar flickers) OR it has an
        // active snooze we haven't fired yet.
        let cutoff = now - RETAIN_AFTER_DONE;
        self.pending.retain(|id, p| {
            if fresh_ids.contains(id) {
                return true;
            }
            if let Some(fired) = p.fired_at {
                return fired > cutoff;
            }
            if p.snoozed_until.is_some() {
                return true;
            }
            false
        });

        // Merge fresh occurrences into the map. Preserve scheduler-owned
        // fields (fired_at, snoozed_until) for known ids, and replace the
        // occurrence data (summary/location may have changed).
        for occ in fresh {
            match self.pending.get_mut(&occ.id) {
                Some(existing) => {
                    existing.occurrence = occ;
                }
                None => {
                    self.pending.insert(occ.id.clone(), PendingFire::new(occ));
                }
            }
        }
        self.persist("refresh").await;
        debug!(pending = self.pending.len(), "refresh merged");
    }

    async fn on_snooze(&mut self, id: &str, duration: Duration) {
        let now = self.now().await;
        let Some(p) = self.pending.get_mut(id) else {
            warn!(id, "snooze for unknown occurrence");
            return;
        };
        p.snoozed_until = Some(now + duration);
        p.fired_at = None; // re-arm
        info!(
            id,
            until = %p.snoozed_until.unwrap().to_rfc3339(),
            "snoozed",
        );
        self.persist("snooze").await;
    }

    async fn on_dismiss(&mut self, id: &str) {
        let now = self.now().await;
        let Some(p) = self.pending.get_mut(id) else {
            warn!(id, "dismiss for unknown occurrence");
            return;
        };
        p.snoozed_until = None;
        p.fired_at = Some(now);
        info!(id, "dismissed");
        self.persist("dismiss").await;
    }

    async fn on_tick(&mut self) {
        let now = self.now().await;

        // Snapshot ids that are due so we can mutate `self.pending` inside
        // the loop without iterator aliasing.
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.is_due(now))
            .map(|(id, _)| id.clone())
            .collect();

        for id in due {
            let Some(p) = self.pending.get(&id).cloned() else {
                continue;
            };
            match self.notifier.deliver(&p).await {
                Ok(()) => {
                    if let Some(entry) = self.pending.get_mut(&id) {
                        entry.fired_at = Some(now);
                        entry.snoozed_until = None;
                    }
                    debug!(id, "fired");
                }
                Err(e) => {
                    warn!(id, error = %e, "delivery failed; will retry next tick");
                }
            }
        }

        // GC delivered entries past retention.
        let cutoff = now - RETAIN_AFTER_DONE;
        let before = self.pending.len();
        self.pending
            .retain(|_, p| p.fired_at.map(|t| t > cutoff).unwrap_or(true) || p.snoozed_until.is_some());
        let pruned = before - self.pending.len();

        if pruned > 0 {
            debug!(pruned, "gc'd old entries");
        }
        self.persist("tick").await;
    }

    async fn persist(&self, reason: &str) {
        if let Err(e) = self.store.save(&self.pending).await {
            warn!(reason, "scheduler persist failed: {e}");
        }
    }

    // ── Test inspection helpers ──────────────────────────────────────

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub fn get(&self, id: &str) -> Option<&PendingFire> {
        self.pending.get(id)
    }
}

async fn run_loop(
    mut sched: Scheduler,
    mut rx: mpsc::UnboundedReceiver<SchedulerCmd>,
    self_tx: mpsc::UnboundedSender<SchedulerCmd>,
) {
    // Tick every 30s. First tick fires immediately (consume) so we don't
    // do work at startup — let the initial Refresh land first.
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Tick via the same channel so it serializes with commands.
                let _ = self_tx.send(SchedulerCmd::Tick);
            }
            cmd = rx.recv() => {
                match cmd {
                    Some(SchedulerCmd::Refresh(occs)) => sched.on_refresh(occs).await,
                    Some(SchedulerCmd::Snooze { occurrence_id, duration }) => {
                        sched.on_snooze(&occurrence_id, duration).await
                    }
                    Some(SchedulerCmd::Dismiss { occurrence_id }) => {
                        sched.on_dismiss(&occurrence_id).await
                    }
                    Some(SchedulerCmd::Tick) => sched.on_tick().await,
                    Some(SchedulerCmd::Inspect(reply)) => {
                        // Send the snapshot sorted by effective fire time so
                        // callers get "next reminder first" without needing
                        // to re-sort. Cloning PendingFire is cheap (one
                        // String + a few timestamps).
                        let mut items: Vec<PendingFire> =
                            sched.pending.values().cloned().collect();
                        items.sort_by_key(|p| p.effective_fire_at());
                        // Receiver may have gone away — not an error.
                        let _ = reply.send(items);
                    }
                    Some(SchedulerCmd::Shutdown) | None => {
                        info!("scheduler shutting down");
                        break;
                    }
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifier::RecordingNotifier;
    use crate::store::MemoryStore;
    use chrono::TimeZone;

    fn occ(id: &str, fire_at: DateTime<Utc>) -> Occurrence {
        Occurrence {
            id: id.into(),
            event_uid: "e".into(),
            recurrence_id: None,
            fire_at,
            event_start: fire_at + Duration::minutes(15),
            event_end: fire_at + Duration::minutes(45),
            summary: format!("event {id}"),
            location: None,
            description: None,
        }
    }

    async fn build() -> (Scheduler, Arc<RecordingNotifier>) {
        let notifier = Arc::new(RecordingNotifier::default());
        let store: Arc<dyn PendingStore> = Arc::new(MemoryStore::default());
        let s = Scheduler::new(store, notifier.clone()).await.unwrap();
        (s, notifier)
    }

    #[tokio::test]
    async fn refresh_inserts_occurrences() {
        let (mut s, _) = build().await;
        s.set_clock(Utc.with_ymd_and_hms(2026, 5, 1, 9, 0, 0).unwrap()).await;
        s.on_refresh(vec![occ("a", Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap())])
            .await;
        assert_eq!(s.pending_count(), 1);
    }

    #[tokio::test]
    async fn tick_fires_due_occurrences() {
        let (mut s, rec) = build().await;
        let t0 = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
        s.set_clock(t0).await;
        s.on_refresh(vec![
            occ("past", t0 - Duration::minutes(1)),
            occ("future", t0 + Duration::hours(1)),
        ])
        .await;
        s.on_tick().await;
        let fired = rec.records.lock().await.clone();
        assert_eq!(fired, vec!["past".to_string()]);
    }

    #[tokio::test]
    async fn snooze_rearms_fire() {
        let (mut s, rec) = build().await;
        let t0 = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
        s.set_clock(t0).await;
        s.on_refresh(vec![occ("x", t0 - Duration::minutes(5))]).await;
        s.on_tick().await; // fires once
        assert_eq!(rec.records.lock().await.len(), 1);

        s.on_snooze("x", Duration::hours(4)).await;
        // Advance clock past the snooze.
        s.set_clock(t0 + Duration::hours(5)).await;
        s.on_tick().await;
        assert_eq!(rec.records.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn dismiss_prevents_refire() {
        let (mut s, rec) = build().await;
        let t0 = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
        s.set_clock(t0).await;
        s.on_refresh(vec![occ("x", t0 - Duration::minutes(1))]).await;
        s.on_tick().await;
        s.on_dismiss("x").await;
        s.set_clock(t0 + Duration::hours(5)).await;
        s.on_tick().await;
        assert_eq!(rec.records.lock().await.len(), 1); // only the original fire
    }

    #[tokio::test]
    async fn refresh_drops_ghost_occurrences() {
        let (mut s, _) = build().await;
        let t0 = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
        s.set_clock(t0).await;
        s.on_refresh(vec![occ("ghost", t0 + Duration::hours(1))]).await;
        assert_eq!(s.pending_count(), 1);
        // Calendar refreshes without the occurrence → it's been deleted upstream.
        s.on_refresh(vec![]).await;
        assert_eq!(s.pending_count(), 0);
    }

    #[tokio::test]
    async fn active_snooze_survives_refresh_even_if_event_vanishes() {
        let (mut s, _) = build().await;
        let t0 = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
        s.set_clock(t0).await;
        s.on_refresh(vec![occ("x", t0 - Duration::minutes(1))]).await;
        s.on_tick().await; // fired
        s.on_snooze("x", Duration::hours(4)).await;
        // Event removed from the calendar — snooze should still survive so
        // the pending 4h reminder isn't silently lost mid-snooze.
        s.on_refresh(vec![]).await;
        assert!(s.get("x").is_some(), "snoozed entry should survive refresh");
    }
}
