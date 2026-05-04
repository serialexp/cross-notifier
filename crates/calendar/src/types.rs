//! Core types for the calendar reminder pipeline.
//!
//! `Occurrence` is the fundamental unit: one fireable reminder, already
//! resolved to an absolute UTC fire time. Recurring events expand to many
//! occurrences (one per instance × VALARM).
//!
//! `PendingFire` layers scheduler state on top of an occurrence — whether
//! it's been delivered, whether it's currently snoozed.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A single fireable reminder. Stable across calendar refreshes via [`Self::id`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    /// Stable hash of (event_uid, recurrence_id, trigger_offset_seconds).
    /// Lets us dedupe when a calendar refresh returns the same event again.
    pub id: String,

    pub event_uid: String,

    /// None for non-recurring events. For recurring events this is the
    /// DTSTART of this specific instance — *not* the master VEVENT's DTSTART.
    pub recurrence_id: Option<DateTime<Utc>>,

    /// When to deliver the reminder (wall-clock UTC).
    pub fire_at: DateTime<Utc>,

    pub event_start: DateTime<Utc>,
    pub event_end: DateTime<Utc>,

    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Occurrence {
    /// Build a stable short id for this occurrence. Deterministic so the same
    /// (event, recurrence, alarm) hashes the same across refreshes.
    pub fn stable_id(
        uid: &str,
        recurrence_id: Option<DateTime<Utc>>,
        trigger_offset: Duration,
    ) -> String {
        let mut h = Sha256::new();
        h.update(uid.as_bytes());
        if let Some(r) = recurrence_id {
            h.update(b"|");
            h.update(r.to_rfc3339().as_bytes());
        }
        h.update(b"|");
        h.update(trigger_offset.num_seconds().to_le_bytes());
        let bytes = h.finalize();
        // 16 hex chars = 64 bits — plenty for dedup in a personal calendar.
        hex::encode(&bytes[..8])
    }
}

/// Scheduler-side state for an occurrence. Persisted across restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFire {
    pub occurrence: Occurrence,

    /// If set, fire at this time instead of `occurrence.fire_at`. Populated
    /// by user-initiated snooze actions. Cleared on delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,

    /// When we successfully delivered this reminder. Present means "done";
    /// combined with a present `snoozed_until`, means "delivered once, now
    /// re-armed for a snooze." Cleared on snooze so the next tick re-fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fired_at: Option<DateTime<Utc>>,
}

/// One calendar event instance — a VEVENT (expanded to a single recurrence
/// instance) with no alarm-trigger axis. Used for things like the daily
/// summary notification, where we want "all events on this date" rather
/// than "all alarms that fire on this date."
///
/// This is deliberately *not* a stored type — it's a transient view of
/// the calendar, recomputed each time the summary runs. No stable id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventInstance {
    pub event_uid: String,
    pub event_start: DateTime<Utc>,
    pub event_end: DateTime<Utc>,
    pub summary: String,
    pub location: Option<String>,
    pub description: Option<String>,
    /// True if the source was a DATE (not DATE-TIME). Callers typically
    /// render these without a clock time.
    pub all_day: bool,
}

impl PendingFire {
    pub fn new(occurrence: Occurrence) -> Self {
        Self {
            occurrence,
            snoozed_until: None,
            fired_at: None,
        }
    }

    pub fn effective_fire_at(&self) -> DateTime<Utc> {
        self.snoozed_until.unwrap_or(self.occurrence.fire_at)
    }

    /// True once delivered and not currently snoozed — we can garbage-collect
    /// these after some grace period.
    pub fn is_done(&self) -> bool {
        self.fired_at.is_some() && self.snoozed_until.is_none()
    }

    /// True iff the effective fire time is at-or-before `now` and the
    /// fire hasn't already happened (or has been re-armed by snooze).
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        if self.effective_fire_at() > now {
            return false;
        }
        // If snoozed_until is in the past, it's due regardless of fired_at.
        // If fired_at is set and snoozed_until is None, we're done — not due.
        match (self.snoozed_until, self.fired_at) {
            (Some(_), _) => true,
            (None, None) => true,
            (None, Some(_)) => false,
        }
    }
}
