//! Parse VCALENDAR text into a list of [`Occurrence`]s for the next
//! `horizon`. Handles:
//!
//! * Non-recurring VEVENTs
//! * Recurring VEVENTs (RRULE + optional EXDATE)
//! * One or more VALARMs per VEVENT (each producing its own Occurrence)
//! * DATE-only (all-day) events
//! * DATE-TIME in UTC (`...Z`)
//!
//! Deliberately out of scope for v1:
//!
//! * TZID-qualified local times (floating times are treated as UTC with
//!   a warning — for a personal calendar that's mostly fine; Fastmail
//!   stores meeting times in UTC anyway)
//! * RECURRENCE-ID override events (a modified instance of a recurring
//!   event). The master RRULE expansion will still emit an occurrence
//!   at the original time — the override is ignored.
//! * Absolute TRIGGER (VALUE=DATE-TIME); only relative duration
//!   triggers are honored.
//!
//! These are tracked with `TODO:` comments and can be layered in later
//! without changing the output shape.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use icalendar::parser::{read_calendar, unfold, Component as RawComponent};
use rrule::{RRuleSet, Tz as RTz};
use tracing::{debug, warn};

use crate::types::{EventInstance, Occurrence};

pub fn parse(
    ics: &str,
    now: DateTime<Utc>,
    horizon: Duration,
) -> anyhow::Result<Vec<Occurrence>> {
    let until = now + horizon;
    // Don't resurrect reminders that fired while we were down — but give
    // a small grace window so a restart right at fire time still catches it.
    let earliest = now - Duration::minutes(5);

    let mut out = Vec::new();
    let mut parse_errors = 0usize;

    // CalDAV REPORT returns one `VCALENDAR` block per event (Fastmail does
    // this, and so do most CalDAV servers). The icalendar nom parser only
    // accepts a single top-level VCALENDAR + EOF, so we split on each
    // `BEGIN:VCALENDAR` and parse the blocks one at a time.
    for block in split_vcalendar_blocks(ics) {
        let unfolded = unfold(&block);
        match read_calendar(&unfolded) {
            Ok(raw) => {
                let mut events: Vec<&RawComponent> = Vec::new();
                collect_events(&raw.components, &mut events);
                for vevent in events {
                    if let Err(e) = expand_event(vevent, now, earliest, until, &mut out) {
                        warn!(
                            uid = prop(vevent, "UID").unwrap_or("?"),
                            "skipping event: {}",
                            e
                        );
                    }
                }
            }
            Err(e) => {
                parse_errors += 1;
                // One bad block shouldn't abort the whole refresh — just
                // skip it and log. First-seen error is enough detail;
                // we don't want to spam with a per-block truncated dump.
                if parse_errors <= 3 {
                    warn!("ics block parse error: {e}");
                }
            }
        }
    }
    if parse_errors > 0 {
        warn!(count = parse_errors, "calendar: total parse errors this refresh");
    }
    debug!(count = out.len(), "parsed occurrences");
    Ok(out)
}

/// Parse the same VCALENDAR text into event *instances* — one per VEVENT
/// recurrence occurrence — ignoring alarms entirely. Returns every event
/// whose `event_start` falls within `[now, now + horizon]`, sorted
/// ascending by start time.
///
/// This is the data path for summary features (e.g. "what's on tomorrow").
/// Most calendar events have no VALARMs attached, so `parse()` wouldn't
/// surface them.
pub fn parse_events(
    ics: &str,
    now: DateTime<Utc>,
    horizon: Duration,
) -> anyhow::Result<Vec<EventInstance>> {
    let until = now + horizon;
    let mut out: Vec<EventInstance> = Vec::new();
    let mut parse_errors = 0usize;

    for block in split_vcalendar_blocks(ics) {
        let unfolded = unfold(&block);
        match read_calendar(&unfolded) {
            Ok(raw) => {
                let mut events: Vec<&RawComponent> = Vec::new();
                collect_events(&raw.components, &mut events);
                for vevent in events {
                    if let Err(e) = expand_event_instances(vevent, now, until, &mut out) {
                        warn!(
                            uid = prop(vevent, "UID").unwrap_or("?"),
                            "skipping event for summary: {}",
                            e
                        );
                    }
                }
            }
            Err(e) => {
                parse_errors += 1;
                if parse_errors <= 3 {
                    warn!("ics block parse error (events path): {e}");
                }
            }
        }
    }
    out.sort_by_key(|e| e.event_start);
    debug!(count = out.len(), "parsed event instances");
    Ok(out)
}

/// Event-path sibling of `expand_event`. Collects recurrence instances
/// whose start falls in the window; no alarm axis.
fn expand_event_instances(
    v: &RawComponent<'_>,
    now: DateTime<Utc>,
    until: DateTime<Utc>,
    out: &mut Vec<EventInstance>,
) -> anyhow::Result<()> {
    let uid = prop(v, "UID")
        .ok_or_else(|| anyhow::anyhow!("missing UID"))?
        .to_string();
    let summary = prop(v, "SUMMARY").unwrap_or("").to_string();
    let location = prop(v, "LOCATION").map(str::to_string);
    let description = prop(v, "DESCRIPTION").map(str::to_string);

    let dtstart_raw = prop(v, "DTSTART").ok_or_else(|| anyhow::anyhow!("missing DTSTART"))?;
    let dtend_raw = prop(v, "DTEND");
    let (start, all_day) = parse_dt(dtstart_raw)?;
    let end = match dtend_raw {
        Some(r) => parse_dt(r)?.0,
        None => {
            if all_day {
                start + Duration::days(1)
            } else {
                start
            }
        }
    };
    let duration = end - start;

    let starts: Vec<DateTime<Utc>> = if let Some(rule) = prop(v, "RRULE") {
        expand_recurrence(dtstart_raw, rule, prop(v, "EXDATE"), now, until)?
    } else {
        // Non-recurring: include only if the instance itself lands in window.
        if start > until || start + duration < now {
            return Ok(());
        }
        vec![start]
    };

    for occ_start in starts {
        if occ_start > until {
            continue;
        }
        // Keep an event that's currently happening (started before `now`
        // but still ongoing) — useful for a noon summary that wants to
        // remind about the lunch meeting that started at 11:45.
        if occ_start + duration < now {
            continue;
        }
        out.push(EventInstance {
            event_uid: uid.clone(),
            event_start: occ_start,
            event_end: occ_start + duration,
            summary: summary.clone(),
            location: location.clone(),
            description: description.clone(),
            all_day,
        });
    }
    Ok(())
}

/// Split a concatenated-VCALENDAR blob into individual `BEGIN:VCALENDAR …
/// END:VCALENDAR` blocks. Tolerates leading whitespace and blank lines
/// between blocks (which the CalDAV XML extractor leaves in).
///
/// If the input contains no `BEGIN:VCALENDAR` marker, the input is
/// returned as-is as a single block — for bare-VEVENT responses or a
/// file that happens to be just one VCALENDAR.
fn split_vcalendar_blocks(ics: &str) -> Vec<String> {
    let marker = "BEGIN:VCALENDAR";
    // Count markers so we can short-circuit the common single-block case
    // without allocating a Vec of owned strings. Most local ICS files
    // have exactly one.
    let count = ics.matches(marker).count();
    if count <= 1 {
        return vec![ics.to_string()];
    }
    let mut blocks = Vec::with_capacity(count);
    let mut rest = ics;
    while let Some(start) = rest.find(marker) {
        let after = &rest[start..];
        let end = after
            .find("END:VCALENDAR")
            .map(|e| e + "END:VCALENDAR".len())
            .unwrap_or(after.len());
        blocks.push(after[..end].to_string());
        rest = &after[end..];
    }
    blocks
}

fn collect_events<'a>(comps: &'a [RawComponent<'a>], out: &mut Vec<&'a RawComponent<'a>>) {
    for c in comps {
        match c.name.as_str() {
            "VEVENT" => out.push(c),
            "VCALENDAR" => collect_events(&c.components, out),
            _ => {}
        }
    }
}

fn expand_event(
    v: &RawComponent<'_>,
    now: DateTime<Utc>,
    earliest: DateTime<Utc>,
    until: DateTime<Utc>,
    out: &mut Vec<Occurrence>,
) -> anyhow::Result<()> {
    let uid = prop(v, "UID")
        .ok_or_else(|| anyhow::anyhow!("missing UID"))?
        .to_string();
    let summary = prop(v, "SUMMARY").unwrap_or("").to_string();
    let location = prop(v, "LOCATION").map(str::to_string);
    let description = prop(v, "DESCRIPTION").map(str::to_string);

    let dtstart_raw = prop(v, "DTSTART").ok_or_else(|| anyhow::anyhow!("missing DTSTART"))?;
    let dtend_raw = prop(v, "DTEND");

    let (start, all_day) = parse_dt(dtstart_raw)?;
    let end = match dtend_raw {
        Some(r) => parse_dt(r)?.0,
        None => {
            // No DTEND: all-day → +1 day, timed → treat as instant (fine for reminders).
            if all_day {
                start + Duration::days(1)
            } else {
                start
            }
        }
    };
    let duration = end - start;

    let rrule = prop(v, "RRULE");
    let starts: Vec<DateTime<Utc>> = if let Some(rule) = rrule {
        expand_recurrence(dtstart_raw, rule, prop(v, "EXDATE"), now, until)?
    } else {
        vec![start]
    };

    // Collect VALARMs: only DISPLAY/AUDIO with a relative duration trigger.
    // Ignore EMAIL alarms — we're not an email gateway.
    let triggers: Vec<Duration> = v
        .components
        .iter()
        .filter(|c| c.name == "VALARM")
        .filter_map(|alarm| {
            let trig = prop(alarm, "TRIGGER")?;
            match parse_trigger_duration(trig) {
                Some(d) => Some(d),
                None => {
                    warn!(
                        uid = %uid,
                        trigger = trig,
                        "absolute or unrecognized TRIGGER — skipping alarm",
                    );
                    None
                }
            }
        })
        .collect();

    if triggers.is_empty() {
        // Event has no VALARMs → no reminders. Silent skip; plenty of
        // events exist without alarms and we shouldn't spam the log.
        return Ok(());
    }

    for occ_start in starts {
        for trigger in &triggers {
            let fire_at = occ_start + *trigger;
            if fire_at < earliest || fire_at > until {
                continue;
            }
            let id = Occurrence::stable_id(&uid, Some(occ_start), *trigger);
            out.push(Occurrence {
                id,
                event_uid: uid.clone(),
                recurrence_id: Some(occ_start),
                fire_at,
                event_start: occ_start,
                event_end: occ_start + duration,
                summary: summary.clone(),
                location: location.clone(),
                description: description.clone(),
            });
        }
    }
    Ok(())
}

/// Look up a property's value (case-insensitive name).
fn prop<'a>(c: &'a RawComponent<'a>, name: &str) -> Option<&'a str> {
    c.properties
        .iter()
        .find(|p| p.name.as_str().eq_ignore_ascii_case(name))
        .map(|p| p.val.as_str())
}

/// Parse a DTSTART/DTEND/EXDATE *value* (the right-hand side of the property).
/// Returns the UTC datetime and a flag indicating whether the source was
/// DATE-only (all-day).
///
/// Supported forms:
/// * `20260501T093000Z`   — UTC date-time
/// * `20260501T093000`    — floating date-time (treated as UTC w/ warning)
/// * `20260501`           — DATE (midnight UTC, all-day=true)
fn parse_dt(raw: &str) -> anyhow::Result<(DateTime<Utc>, bool)> {
    // EXDATE/DTSTART can carry a TZID= parameter that appears before the
    // value. By the time we get here the parser has stripped the property
    // name, but parameters may still be present as `TZID=Europe/...:VALUE`
    // if the caller passed the property *line*. Our callers pass `.val`
    // which should already be just the value, but some CalDAV payloads
    // embed the raw param inline. Be tolerant.
    let value = raw.rsplit(':').next().unwrap_or(raw).trim();

    // DATE: 8 digits, no 'T'
    if value.len() == 8 && !value.contains('T') {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d")?;
        let dt = date.and_hms_opt(0, 0, 0).unwrap();
        return Ok((Utc.from_utc_datetime(&dt), true));
    }

    // DATE-TIME: 15 or 16 chars, contains 'T'
    let utc = value.ends_with('Z');
    let stripped = value.trim_end_matches('Z');
    let naive = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S")?;
    if !utc {
        // Floating/local time. Without TZID handling we best-effort as UTC.
        debug!(
            value,
            "floating DATE-TIME treated as UTC (TZID parsing not yet implemented)"
        );
    }
    Ok((Utc.from_utc_datetime(&naive), false))
}

/// Parse a VALARM `TRIGGER` value as a signed duration (offset from DTSTART).
/// Returns None for absolute triggers or malformed input.
///
/// Examples: `-PT15M` → -15 min; `PT0S` → 0; `-P1D` → -1 day; `P1DT2H` → +1d 2h.
fn parse_trigger_duration(trig: &str) -> Option<Duration> {
    let t = trig.trim();
    // Absolute datetime trigger — contains 'T' but starts with a year-ish digit run.
    if !(t.starts_with('P') || t.starts_with("-P") || t.starts_with("+P")) {
        return None;
    }
    let (sign, rest) = if let Some(r) = t.strip_prefix('-') {
        (-1i64, r)
    } else if let Some(r) = t.strip_prefix('+') {
        (1i64, r)
    } else {
        (1i64, t)
    };
    let body = rest.strip_prefix('P')?;
    // Split on 'T': before=date part (D/W), after=time part (H/M/S).
    let (date_part, time_part) = match body.split_once('T') {
        Some((a, b)) => (a, b),
        None => (body, ""),
    };

    let mut total_s: i64 = 0;
    let mut num = String::new();

    for ch in date_part.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else {
            let n: i64 = num.parse().ok()?;
            num.clear();
            match ch {
                'W' => total_s += n * 7 * 86400,
                'D' => total_s += n * 86400,
                _ => return None,
            }
        }
    }
    for ch in time_part.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
        } else {
            let n: i64 = num.parse().ok()?;
            num.clear();
            match ch {
                'H' => total_s += n * 3600,
                'M' => total_s += n * 60,
                'S' => total_s += n,
                _ => return None,
            }
        }
    }
    if !num.is_empty() {
        return None; // trailing number with no unit
    }
    Some(Duration::seconds(sign * total_s))
}

/// Expand a DTSTART + RRULE (+ optional EXDATE) into UTC instance starts
/// within `[now, until]`. We feed the strings back to `rrule` in RFC form.
fn expand_recurrence(
    dtstart_raw: &str,
    rrule_raw: &str,
    exdate_raw: Option<&str>,
    now: DateTime<Utc>,
    until: DateTime<Utc>,
) -> anyhow::Result<Vec<DateTime<Utc>>> {
    // rrule wants DTSTART in its own line. We normalize the form so floating
    // times get the :Z suffix — we already decided to treat them as UTC.
    let (start_dt, _) = parse_dt(dtstart_raw)?;
    let dtstart_line = format!("DTSTART:{}", start_dt.format("%Y%m%dT%H%M%SZ"));

    // The rrule crate rejects a mismatch between DTSTART's timezone and the
    // RRULE's UNTIL timezone. Many Google-exported calendars emit
    // `DTSTART:...Z` with `RRULE:...;UNTIL=YYYYMMDDTHHMMSS` (no Z), which
    // makes this check explode. Since we've already decided floating ==
    // UTC for DTSTART, we align UNTIL by appending Z when it's missing.
    let rrule_norm = normalize_until_to_utc(rrule_raw);

    let mut body = format!("{dtstart_line}\nRRULE:{}", rrule_norm.trim());
    if let Some(ex) = exdate_raw {
        // EXDATE may be comma-separated. Normalize each to UTC.
        let parts: Vec<String> = ex
            .split(',')
            .filter_map(|v| parse_dt(v).ok())
            .map(|(dt, _)| dt.format("%Y%m%dT%H%M%SZ").to_string())
            .collect();
        if !parts.is_empty() {
            body.push_str("\nEXDATE:");
            body.push_str(&parts.join(","));
        }
    }

    let set: RRuleSet = body
        .parse()
        .map_err(|e: rrule::RRuleError| anyhow::anyhow!("rrule parse: {e}"))?;
    let after: chrono::DateTime<RTz> = RTz::UTC.from_utc_datetime(&now.naive_utc());
    let before: chrono::DateTime<RTz> = RTz::UTC.from_utc_datetime(&until.naive_utc());
    // `limit` guards against runaway expansions from pathological RRULEs
    // (FREQ=SECONDLY forever). 500 events per refresh is far more than
    // any personal calendar produces inside our horizon.
    let result = set.after(after).before(before).all(500);
    if result.limited {
        warn!(
            rrule = %rrule_raw,
            "rrule expansion hit the 500-event limit; some occurrences may be missing"
        );
    }

    Ok(result
        .dates
        .into_iter()
        .map(|d| Utc.from_utc_datetime(&d.naive_utc()))
        .collect())
}

/// Normalize the UNTIL value in an RRULE so it matches a UTC DTSTART.
///
/// We always serialize DTSTART as a UTC DATE-TIME (`...Z`) in
/// [`expand_recurrence`], so UNTIL must also be a UTC DATE-TIME — the
/// rrule crate rejects a mismatch outright (DATE-only UNTIL is parsed
/// as a *local* DATE-TIME at midnight, so even that fails).
///
/// Transformations:
/// * `UNTIL=YYYYMMDDTHHMMSSZ` → unchanged
/// * `UNTIL=YYYYMMDDTHHMMSS`  → append `Z` (treat floating as UTC)
/// * `UNTIL=YYYYMMDD`         → expand to `YYYYMMDDT235959Z` (end of day),
///                              so the last day's instances are still
///                              included.
fn normalize_until_to_utc(rrule: &str) -> String {
    let mut out = String::with_capacity(rrule.len());
    for (i, part) in rrule.split(';').enumerate() {
        if i > 0 {
            out.push(';');
        }
        if let Some(rest) = part
            .strip_prefix("UNTIL=")
            .or_else(|| part.strip_prefix("until="))
        {
            let has_t = rest.contains('T');
            let has_z = rest.ends_with('Z');
            if has_t && !has_z {
                out.push_str("UNTIL=");
                out.push_str(rest);
                out.push('Z');
                continue;
            }
            // DATE-only UNTIL (8 digits, no T). Expand to end-of-day UTC
            // so we still include any instances on that final date.
            if !has_t && rest.len() == 8 && rest.chars().all(|c| c.is_ascii_digit()) {
                out.push_str("UNTIL=");
                out.push_str(rest);
                out.push_str("T235959Z");
                continue;
            }
        }
        out.push_str(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS_SINGLE: &str = "\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\n\
BEGIN:VEVENT\r\nUID:single@x\r\nDTSTART:20260501T100000Z\r\nDTEND:20260501T110000Z\r\nSUMMARY:Lunch\r\n\
BEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

    const ICS_RECUR: &str = "\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\n\
BEGIN:VEVENT\r\nUID:recur@x\r\nDTSTART:20260501T100000Z\r\nDTEND:20260501T103000Z\r\n\
RRULE:FREQ=WEEKLY;COUNT=4\r\nSUMMARY:Standup\r\n\
BEGIN:VALARM\r\nTRIGGER:-PT5M\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
BEGIN:VALARM\r\nTRIGGER:-P1D\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 15, 0, 0, 0).unwrap()
    }

    #[test]
    fn single_event_produces_one_occurrence() {
        let occs = parse(ICS_SINGLE, now(), Duration::days(30)).unwrap();
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].summary, "Lunch");
        assert_eq!(occs[0].fire_at, occs[0].event_start - Duration::minutes(15));
    }

    #[test]
    fn recurring_event_expands_each_alarm() {
        let occs = parse(ICS_RECUR, now(), Duration::days(60)).unwrap();
        // 4 recurrences × 2 alarms = 8
        assert_eq!(occs.len(), 8);
    }

    #[test]
    fn exdate_removes_instance() {
        let ics = "\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nUID:x@y\r\nDTSTART:20260501T100000Z\r\nDTEND:20260501T103000Z\r\n\
RRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE:20260508T100000Z\r\nSUMMARY:S\r\n\
BEGIN:VALARM\r\nTRIGGER:-PT5M\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";
        let occs = parse(ics, now(), Duration::days(60)).unwrap();
        assert_eq!(occs.len(), 3); // 4 instances minus the excluded one
    }

    #[test]
    fn past_occurrences_pruned() {
        // Event is in the past; horizon doesn't reach it from `now`.
        let occs = parse(ICS_SINGLE, now() + Duration::days(365), Duration::days(30)).unwrap();
        assert!(occs.is_empty());
    }

    #[test]
    fn no_alarms_means_no_occurrences() {
        let ics = "\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20260501T100000Z\r\nDTEND:20260501T110000Z\r\nSUMMARY:Quiet\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";
        let occs = parse(ics, now(), Duration::days(30)).unwrap();
        assert!(occs.is_empty());
    }

    #[test]
    fn parses_absolute_trigger_as_none() {
        // Malformed / absolute triggers are ignored with a warning, not a parse error.
        assert!(parse_trigger_duration("20260501T100000Z").is_none());
    }

    #[test]
    fn trigger_duration_forms() {
        assert_eq!(parse_trigger_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_trigger_duration("PT0S"), Some(Duration::seconds(0)));
        assert_eq!(parse_trigger_duration("-P1D"), Some(Duration::days(-1)));
        assert_eq!(
            parse_trigger_duration("-P1DT2H30M"),
            Some(-(Duration::days(1) + Duration::hours(2) + Duration::minutes(30))),
        );
        assert_eq!(parse_trigger_duration("P1W"), Some(Duration::weeks(1)));
    }

    #[test]
    fn normalize_until_appends_z_to_floating_datetime() {
        assert_eq!(
            normalize_until_to_utc("FREQ=WEEKLY;UNTIL=20260601T000000;BYDAY=MO"),
            "FREQ=WEEKLY;UNTIL=20260601T000000Z;BYDAY=MO"
        );
    }

    #[test]
    fn normalize_until_leaves_utc_alone() {
        assert_eq!(
            normalize_until_to_utc("FREQ=WEEKLY;UNTIL=20260601T000000Z"),
            "FREQ=WEEKLY;UNTIL=20260601T000000Z"
        );
    }

    #[test]
    fn normalize_until_expands_date_only_to_end_of_day_utc() {
        // DATE-only UNTIL (8 digits) — the rrule crate parses an unzulu'd
        // value as Local, which mismatches our UTC DTSTART. Expand to
        // end-of-day UTC so the final day's instances are still included.
        assert_eq!(
            normalize_until_to_utc("FREQ=DAILY;UNTIL=20260601"),
            "FREQ=DAILY;UNTIL=20260601T235959Z"
        );
        assert_eq!(
            normalize_until_to_utc("FREQ=DAILY;UNTIL=20260601;BYDAY=MO"),
            "FREQ=DAILY;UNTIL=20260601T235959Z;BYDAY=MO"
        );
    }

    #[test]
    fn recurring_with_date_only_until_expands() {
        // Reproduces the production warning: DTSTART in UTC, UNTIL as DATE.
        let ics = "\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nUID:dateuntil@x\r\nDTSTART:20260501T100000Z\r\nDTEND:20260501T103000Z\r\n\
RRULE:FREQ=WEEKLY;UNTIL=20260601\r\nSUMMARY:S\r\n\
BEGIN:VALARM\r\nTRIGGER:-PT5M\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";
        let occs = parse(ics, now(), Duration::days(60)).unwrap();
        // 5 weekly occurrences: May 1, 8, 15, 22, 29 (all <= 2026-06-01).
        assert_eq!(occs.len(), 5);
    }

    #[test]
    fn all_day_events_have_midnight_start() {
        let ics = "\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nUID:allday@x\r\nDTSTART;VALUE=DATE:20260501\r\nDTEND;VALUE=DATE:20260502\r\nSUMMARY:Off\r\n\
BEGIN:VALARM\r\nTRIGGER:-PT1H\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";
        let occs = parse(ics, now(), Duration::days(60)).unwrap();
        assert_eq!(occs.len(), 1);
        assert_eq!(
            occs[0].event_start,
            Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap()
        );
        assert_eq!(occs[0].fire_at, occs[0].event_start - Duration::hours(1));
    }
}
