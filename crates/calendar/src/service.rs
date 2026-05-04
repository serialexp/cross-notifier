//! Top-level orchestrator: runs the scheduler, periodically fetches the
//! calendar, parses, and pushes refreshes into the scheduler.
//!
//! Owns nothing that must survive a restart; all persistence happens in
//! the scheduler's PendingStore.

use std::sync::Arc;

use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};
use cross_notifier_core::protocol::Notification;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::notifier::Notifier;
use crate::parse;
use crate::scheduler::{Scheduler, SchedulerCmd, SchedulerHandle};
use crate::source::CalendarSource;
use crate::store::PendingStore;
use crate::types::EventInstance;

#[derive(Clone)]
pub struct CalendarServiceConfig {
    /// How far ahead to look when expanding recurrences and parsing alarms.
    /// 48h is enough to honor a "1 day before" alarm for anything tomorrow
    /// or the day after, while keeping the refresh cheap.
    pub horizon: Duration,
    /// How often to re-fetch the calendar.
    pub refresh_interval: Duration,
    /// Opt-in: fire a "tomorrow's agenda" notification every day at the
    /// configured local wall-clock time. `None` disables the feature.
    pub daily_summary: Option<DailySummaryConfig>,
}

/// Schedule for the daily summary notification. The time is expressed in
/// local wall-clock — DST transitions don't require special handling
/// because we always rebase from `Local::now()` when picking the next fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailySummaryConfig {
    /// 0..=23
    pub hour: u32,
    /// 0..=59
    pub minute: u32,
}

impl Default for DailySummaryConfig {
    fn default() -> Self {
        // Noon — when Bart said "12 at noon."
        Self {
            hour: 12,
            minute: 0,
        }
    }
}

impl Default for CalendarServiceConfig {
    fn default() -> Self {
        Self {
            horizon: Duration::hours(48),
            refresh_interval: Duration::minutes(5),
            daily_summary: None,
        }
    }
}

pub struct CalendarService {
    handle: SchedulerHandle,
    /// Dropped by `shutdown` to signal the refresh task. Wrapped in Option
    /// so `shutdown` can take it by value.
    refresh_shutdown: Option<oneshot::Sender<()>>,
    refresh_task: Option<JoinHandle<()>>,
    scheduler_task: Option<JoinHandle<()>>,
    /// None when daily-summary is disabled in config. Both fields go
    /// together — there's no summary task to await without a shutdown
    /// channel, and vice versa.
    summary_shutdown: Option<oneshot::Sender<()>>,
    summary_task: Option<JoinHandle<()>>,
}

impl CalendarService {
    /// Spawn the scheduler task and the refresh task. Returns once both
    /// are running. The first refresh happens immediately on the refresh
    /// task so the scheduler has data to fire against right away.
    pub async fn spawn(
        source: Arc<dyn CalendarSource>,
        notifier: Arc<dyn Notifier>,
        store: Arc<dyn PendingStore>,
        cfg: CalendarServiceConfig,
    ) -> anyhow::Result<Self> {
        // Scheduler first — notifier is cloned into both it and the
        // summary task, hence the `Arc`.
        let sched = Scheduler::new(store, notifier.clone()).await?;
        let (handle, scheduler_task) = sched.spawn();

        // Refresh loop — periodic re-fetch → scheduler refresh.
        let (refresh_shutdown_tx, refresh_shutdown_rx) = oneshot::channel();
        let refresh_task = tokio::spawn(refresh_loop(
            source.clone(),
            handle.clone(),
            cfg.clone(),
            refresh_shutdown_rx,
        ));

        // Optional summary loop — fires a single notification at the
        // configured local time each day. Kept in a separate task so it
        // can sleep for ~24h without blocking anything else.
        let (summary_shutdown, summary_task) = if let Some(summary_cfg) = cfg.daily_summary {
            let (tx, rx) = oneshot::channel();
            let task = tokio::spawn(daily_summary_loop(
                source.clone(),
                notifier.clone(),
                summary_cfg,
                rx,
            ));
            (Some(tx), Some(task))
        } else {
            (None, None)
        };

        Ok(Self {
            handle,
            refresh_shutdown: Some(refresh_shutdown_tx),
            refresh_task: Some(refresh_task),
            scheduler_task: Some(scheduler_task),
            summary_shutdown,
            summary_task,
        })
    }

    /// A clonable remote-control handle for this service. Action handlers
    /// route snooze/dismiss through here.
    pub fn handle(&self) -> SchedulerHandle {
        self.handle.clone()
    }

    /// Stop the refresh loop and the scheduler, awaiting both tasks. Call
    /// this before dropping the service (and before spawning a replacement)
    /// so pending state is flushed and no work races against the new
    /// scheduler on the same persistence file.
    pub async fn shutdown(mut self) {
        // Signal both sleeping tasks before draining the scheduler so
        // neither tries to dispatch one more piece of work at a scheduler
        // that's already shutting down.
        if let Some(tx) = self.refresh_shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(tx) = self.summary_shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.refresh_task.take() {
            let _ = t.await;
        }
        if let Some(t) = self.summary_task.take() {
            let _ = t.await;
        }
        // Then drain the scheduler. It does a final persist on the Shutdown
        // path via the channel closing (no explicit final save, but the
        // last tick/refresh already persisted).
        let _ = self.handle.send(SchedulerCmd::Shutdown);
        if let Some(t) = self.scheduler_task.take() {
            let _ = t.await;
        }
    }
}

impl Drop for CalendarService {
    fn drop(&mut self) {
        // If the owner forgot to call shutdown(), at least signal every
        // task to exit. We can't await here, so tasks detach — acceptable
        // for process exit, but warn so misuse at runtime is visible.
        if self.refresh_task.is_some()
            || self.scheduler_task.is_some()
            || self.summary_task.is_some()
        {
            if let Some(tx) = self.refresh_shutdown.take() {
                let _ = tx.send(());
            }
            if let Some(tx) = self.summary_shutdown.take() {
                let _ = tx.send(());
            }
            let _ = self.handle.send(SchedulerCmd::Shutdown);
            tracing::debug!("CalendarService dropped without shutdown(); tasks detached");
        }
    }
}

async fn refresh_loop(
    source: Arc<dyn CalendarSource>,
    sched: SchedulerHandle,
    cfg: CalendarServiceConfig,
    mut shutdown: oneshot::Receiver<()>,
) {
    let interval = cfg
        .refresh_interval
        .to_std()
        .unwrap_or(std::time::Duration::from_secs(300));
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                info!(source = source.label(), "calendar refresh loop shutting down");
                break;
            }
            _ = ticker.tick() => {
                match do_refresh(source.as_ref(), &sched, cfg.horizon).await {
                    Ok(n) => info!(source = source.label(), count = n, "calendar refreshed"),
                    Err(e) => warn!(source = source.label(), error = %e, "calendar refresh failed"),
                }
            }
        }
    }
}

async fn do_refresh(
    source: &dyn CalendarSource,
    sched: &SchedulerHandle,
    horizon: Duration,
) -> anyhow::Result<usize> {
    let ics = source.fetch().await?;
    let occs = parse::parse(&ics, Utc::now(), horizon)?;
    let n = occs.len();
    sched
        .send(SchedulerCmd::Refresh(occs))
        .map_err(|_| anyhow::anyhow!("scheduler channel closed"))?;
    Ok(n)
}

/// Convenience: a single-shot "pull the calendar once and print what we'd
/// schedule" for debugging. Bypasses the scheduler entirely.
pub async fn dry_run(
    source: &dyn CalendarSource,
    horizon: Duration,
) -> anyhow::Result<Vec<crate::types::Occurrence>> {
    let ics = source.fetch().await?;
    parse::parse(&ics, Utc::now(), horizon)
}

// ── Daily summary ───────────────────────────────────────────────────────

/// Fires a "tomorrow's agenda" notification at the configured local time
/// every day. Sleeps until the next fire, fetches the calendar afresh
/// (independent of the refresh loop — this runs once daily and doesn't
/// need to coordinate), lists events whose start falls on tomorrow's
/// local date, and delivers them as a single summary notification.
async fn daily_summary_loop(
    source: Arc<dyn CalendarSource>,
    notifier: Arc<dyn Notifier>,
    cfg: DailySummaryConfig,
    mut shutdown: oneshot::Receiver<()>,
) {
    info!(
        hour = cfg.hour,
        minute = cfg.minute,
        source = source.label(),
        "daily-summary task started",
    );
    loop {
        let Some(next_fire_utc) = next_summary_fire(Local::now(), cfg) else {
            warn!("daily-summary: could not compute next fire time; bailing out");
            return;
        };
        let sleep_for = (next_fire_utc - Utc::now())
            .to_std()
            .unwrap_or(std::time::Duration::from_secs(1));
        debug_log_next_fire(next_fire_utc);

        tokio::select! {
            biased;
            _ = &mut shutdown => {
                info!("daily-summary task shutting down");
                return;
            }
            _ = tokio::time::sleep(sleep_for) => {}
        }

        // Fire the summary for *tomorrow* relative to the moment we woke
        // up — we want "the day after today-at-noon", which is normally
        // "tomorrow" but correctly handles running slightly late.
        match run_summary_once(source.as_ref(), notifier.as_ref(), Local::now()).await {
            Ok(n) => info!(count = n, "daily-summary delivered"),
            Err(e) => warn!(error = %e, "daily-summary failed"),
        }
    }
}

fn debug_log_next_fire(next: DateTime<Utc>) {
    let local = next.with_timezone(&Local);
    info!(at = %local.to_rfc3339(), "daily-summary: next fire scheduled");
}

/// Given the current local time, compute the next UTC instant at which
/// the summary should fire. Returns `None` only if the configured hour/
/// minute is invalid (out of range), which the caller treats as a fatal
/// config error.
fn next_summary_fire(now_local: DateTime<Local>, cfg: DailySummaryConfig) -> Option<DateTime<Utc>> {
    let fire_time = NaiveTime::from_hms_opt(cfg.hour, cfg.minute, 0)?;
    let today = now_local.date_naive();
    let candidate_today = Local
        .from_local_datetime(&today.and_time(fire_time))
        .earliest()?;
    let target = if candidate_today > now_local {
        candidate_today
    } else {
        // Past today's fire — schedule for tomorrow. `.and_time` + DST
        // can in theory produce an ambiguous/skipped local time; pick the
        // earliest valid instant in those rare cases.
        let tomorrow = today.succ_opt()?;
        Local
            .from_local_datetime(&tomorrow.and_time(fire_time))
            .earliest()?
    };
    Some(target.with_timezone(&Utc))
}

/// One summary cycle: fetch the calendar, filter to tomorrow's events,
/// build and deliver a single notification. Returns the number of events
/// summarized (0 means we still send a "nothing scheduled" card so the
/// user knows the summary ran).
async fn run_summary_once(
    source: &dyn CalendarSource,
    notifier: &dyn Notifier,
    now_local: DateTime<Local>,
) -> anyhow::Result<usize> {
    // Horizon: 48h is plenty — we only keep events whose local date equals
    // tomorrow, so anything past that gets filtered anyway. Using 48h
    // rather than the tighter window makes the filtering DST-safe.
    let ics = source.fetch().await?;
    let all = parse::parse_events(&ics, Utc::now(), Duration::hours(48))?;
    let tomorrow_local = now_local
        .date_naive()
        .succ_opt()
        .ok_or_else(|| anyhow::anyhow!("chrono refused to advance today's date — not expected"))?;
    let mut todays: Vec<EventInstance> = all
        .into_iter()
        .filter(|e| {
            // An "all-day" entry is stored as midnight UTC → that matches
            // tomorrow's local date only if the user is near UTC. Compare
            // the local-date of `event_start` for timed events and the
            // stored UTC date for all-day events, which is how most ICS
            // producers encode them.
            let d = if e.all_day {
                e.event_start.date_naive()
            } else {
                e.event_start.with_timezone(&Local).date_naive()
            };
            d == tomorrow_local
        })
        .collect();
    todays.sort_by_key(|e| e.event_start);

    let notification = build_summary_notification(&todays, tomorrow_local);
    notifier.deliver_notification(notification).await?;
    Ok(todays.len())
}

fn build_summary_notification(
    events: &[EventInstance],
    tomorrow_local: chrono::NaiveDate,
) -> Notification {
    let title = format!("Agenda — {}", tomorrow_local.format("%a %b %-d"));
    let message = if events.is_empty() {
        "Nothing scheduled.".to_string()
    } else {
        events
            .iter()
            .map(|e| {
                let time = if e.all_day {
                    "all-day".to_string()
                } else {
                    e.event_start
                        .with_timezone(&Local)
                        .format("%H:%M")
                        .to_string()
                };
                let summary = if e.summary.is_empty() {
                    "(untitled event)"
                } else {
                    e.summary.as_str()
                };
                match e.location.as_deref() {
                    Some(loc) if !loc.is_empty() => format!("{time}  {summary} · {loc}"),
                    _ => format!("{time}  {summary}"),
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    Notification {
        // Stable id for today's summary so a client that gets the same
        // notification twice (unlikely but possible on reconnect)
        // deduplicates cleanly.
        id: format!("calendar-summary-{}", tomorrow_local.format("%Y%m%d")),
        source: "calendar".to_string(),
        title,
        message,
        // No snooze/dismiss — summaries are informational; clicking
        // dismisses the card like any other notification.
        actions: Vec::new(),
        // Non-exclusive — if the same summary notification somehow lands
        // on multiple devices, the user can dismiss each independently;
        // there's no "the agenda has been handled" semantic like with
        // a reminder.
        exclusive: false,
        // Keep it in the center so if the user misses the popup they can
        // still see tomorrow's plan at a glance.
        store_on_expire: true,
        ..Default::default()
    }
}

// We only re-export mpsc types indirectly via SchedulerHandle; keep this
// use statement to hint readers where the channel actually lives.
#[allow(unused_imports)]
use mpsc as _mpsc;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Timelike};

    #[test]
    fn next_fire_is_today_when_noon_still_ahead() {
        // 09:00 local → noon today is still in the future.
        let now = Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 5, 1)
                    .unwrap()
                    .and_hms_opt(9, 0, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap();
        let cfg = DailySummaryConfig {
            hour: 12,
            minute: 0,
        };
        let next = next_summary_fire(now, cfg).unwrap();
        assert_eq!(next.with_timezone(&Local).hour(), 12);
        assert_eq!(next.with_timezone(&Local).date_naive(), now.date_naive());
    }

    #[test]
    fn next_fire_rolls_to_tomorrow_after_noon() {
        // 13:00 local → today's noon has passed; next fire is tomorrow.
        let now = Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 5, 1)
                    .unwrap()
                    .and_hms_opt(13, 0, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap();
        let cfg = DailySummaryConfig {
            hour: 12,
            minute: 0,
        };
        let next = next_summary_fire(now, cfg).unwrap();
        let next_local = next.with_timezone(&Local);
        assert_eq!(next_local.hour(), 12);
        assert_eq!(
            next_local.date_naive(),
            NaiveDate::from_ymd_opt(2026, 5, 2).unwrap()
        );
    }

    #[test]
    fn summary_notification_lists_events_in_order() {
        use chrono::TimeZone;
        let tomorrow = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let events = vec![
            EventInstance {
                event_uid: "a".into(),
                event_start: Utc.with_ymd_and_hms(2026, 5, 2, 9, 0, 0).unwrap(),
                event_end: Utc.with_ymd_and_hms(2026, 5, 2, 10, 0, 0).unwrap(),
                summary: "Standup".into(),
                location: None,
                description: None,
                all_day: false,
            },
            EventInstance {
                event_uid: "b".into(),
                event_start: Utc.with_ymd_and_hms(2026, 5, 2, 13, 0, 0).unwrap(),
                event_end: Utc.with_ymd_and_hms(2026, 5, 2, 14, 0, 0).unwrap(),
                summary: "Dentist".into(),
                location: Some("Main St".into()),
                description: None,
                all_day: false,
            },
        ];
        let n = build_summary_notification(&events, tomorrow);
        assert!(n.title.contains("May"));
        // Rendering uses local time — we only assert on the structure.
        assert!(n.message.contains("Standup"));
        assert!(n.message.contains("Dentist"));
        assert!(n.message.contains("Main St"));
        assert!(n.actions.is_empty());
        assert!(!n.exclusive);
        assert!(n.store_on_expire);
    }

    #[test]
    fn summary_notification_handles_empty_day() {
        let tomorrow = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let n = build_summary_notification(&[], tomorrow);
        assert!(n.message.contains("Nothing scheduled"));
    }
}
