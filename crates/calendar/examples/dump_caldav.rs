//! One-shot helper: fetch the CalDAV REPORT and dump the raw ICS we
//! extracted. Useful to diagnose parser mismatches against Fastmail.

use std::env;

use cross_notifier_calendar::{CalDav, CalendarSource};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let endpoint = env::var("CALDAV_ENDPOINT")?;
    let user = env::var("CALDAV_USER")?;
    let password = env::var("CALDAV_PASSWORD")?;
    let src = CalDav::new(endpoint, user, password);
    let ics = src.fetch().await?;
    println!("{}", ics);
    Ok(())
}
