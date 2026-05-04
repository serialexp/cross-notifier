//! Debugging helper: reads `CALDAV_*` from .env (or the process env),
//! fetches the calendar, parses it, and prints the first batch of
//! occurrences as JSON. No scheduling, no notifications — just lets you
//! see what the pipeline makes of your real calendar.
//!
//! Usage:
//!   # one-shot inspection against Fastmail
//!   cargo run --example fetch_caldav -p cross-notifier-calendar
//!
//!   # load a local .ics file instead
//!   CAL_ICS_FILE=./some.ics cargo run --example fetch_caldav -p cross-notifier-calendar
//!
//!   # hours to look ahead (default 72)
//!   CAL_HORIZON_HOURS=168 cargo run --example fetch_caldav -p cross-notifier-calendar
//!
//! .env knobs (all optional except when using CalDAV):
//!   CALDAV_ENDPOINT  — calendar collection URL
//!   CALDAV_USER      — username (typically email)
//!   CALDAV_PASSWORD  — Fastmail app-specific password
//!   CAL_ICS_FILE     — local .ics file; takes priority over CALDAV_*
//!   CAL_HORIZON_HOURS — look-ahead window in hours (default 72)

use std::env;
use std::sync::Arc;

use chrono::Duration;
use cross_notifier_calendar::{CalDav, CalendarSource, IcsFile};
use tracing::Level;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // .env is optional — env vars also work.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(Level::INFO.to_string())),
        )
        .with_target(false)
        .init();

    let horizon_hours: i64 = env::var("CAL_HORIZON_HOURS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(72);
    let horizon = Duration::hours(horizon_hours);

    let source: Arc<dyn CalendarSource> = if let Ok(path) = env::var("CAL_ICS_FILE") {
        eprintln!("# source: ics file {}", path);
        Arc::new(IcsFile::new(path))
    } else {
        let endpoint = env::var("CALDAV_ENDPOINT").map_err(|_| {
            anyhow::anyhow!("CALDAV_ENDPOINT not set (and no CAL_ICS_FILE fallback)")
        })?;
        let user = env::var("CALDAV_USER").map_err(|_| anyhow::anyhow!("CALDAV_USER not set"))?;
        let password =
            env::var("CALDAV_PASSWORD").map_err(|_| anyhow::anyhow!("CALDAV_PASSWORD not set"))?;
        eprintln!("# source: caldav {}", endpoint);
        Arc::new(CalDav::new(endpoint, user, password))
    };

    eprintln!("# horizon: {}h", horizon_hours);
    eprintln!("# fetching...");
    let occurrences = cross_notifier_calendar::dry_run(source.as_ref(), horizon).await?;
    eprintln!("# parsed {} occurrence(s)", occurrences.len());

    // Stable order for readability.
    let mut occurrences = occurrences;
    occurrences.sort_by_key(|o| o.fire_at);

    let out = serde_json::to_string_pretty(&occurrences)?;
    println!("{out}");
    Ok(())
}
