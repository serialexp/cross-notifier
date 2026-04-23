//! Where calendar data comes from. Each source produces raw ICS text;
//! parsing is unified in [`crate::parse`].
//!
//! Fastmail note: the CalDAV `REPORT` we issue is the simplest possible
//! `calendar-query` — no time-range filter, let the client-side parser
//! discard past occurrences. Fine for a personal calendar.

use std::path::PathBuf;

use async_trait::async_trait;

/// A source that produces VCALENDAR text on demand. Implementations may
/// concatenate multiple VCALENDAR blocks (CalDAV returns one per event);
/// the parser handles either shape.
#[async_trait]
pub trait CalendarSource: Send + Sync {
    /// Returns the current calendar contents as raw ICS text.
    async fn fetch(&self) -> anyhow::Result<String>;

    /// Short human label for logs.
    fn label(&self) -> &str;
}

// ── IcsFile ────────────────────────────────────────────────────────────

/// Read a `.ics` file from disk on every fetch. Handy for testing.
pub struct IcsFile {
    pub path: PathBuf,
    label: String,
}

impl IcsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let label = format!("ics:{}", path.display());
        Self { path, label }
    }
}

#[async_trait]
impl CalendarSource for IcsFile {
    async fn fetch(&self) -> anyhow::Result<String> {
        Ok(tokio::fs::read_to_string(&self.path).await?)
    }
    fn label(&self) -> &str {
        &self.label
    }
}

// ── IcsUrl ─────────────────────────────────────────────────────────────

/// Subscribe to a publicly-available ICS feed (Google Calendar public URL,
/// Fastmail's public subscription URL, etc). Supports optional HTTP basic
/// auth for minimally-protected feeds.
pub struct IcsUrl {
    pub url: String,
    pub basic_auth: Option<(String, String)>,
    client: reqwest::Client,
    label: String,
}

impl IcsUrl {
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        let label = format!("url:{}", redact_auth(&url));
        Self {
            url,
            basic_auth: None,
            client: reqwest::Client::new(),
            label,
        }
    }

    pub fn with_basic_auth(mut self, user: impl Into<String>, password: impl Into<String>) -> Self {
        self.basic_auth = Some((user.into(), password.into()));
        self
    }
}

#[async_trait]
impl CalendarSource for IcsUrl {
    async fn fetch(&self) -> anyhow::Result<String> {
        let mut req = self.client.get(&self.url);
        if let Some((u, p)) = &self.basic_auth {
            req = req.basic_auth(u, Some(p));
        }
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.text().await?)
    }
    fn label(&self) -> &str {
        &self.label
    }
}

// ── CalDav ─────────────────────────────────────────────────────────────

/// A CalDAV calendar collection URL (e.g. the per-calendar endpoint
/// Fastmail exposes under `/dav/calendars/user/<email>/<calendar-uuid>/`).
///
/// We issue a `calendar-query` REPORT for all VEVENTs and extract the
/// `<calendar-data>` payload from each `<response>` in the multistatus.
/// Concatenated result is handed to the ICS parser unchanged.
pub struct CalDav {
    pub endpoint: String,
    pub user: String,
    pub password: String,
    client: reqwest::Client,
    label: String,
}

impl CalDav {
    pub fn new(
        endpoint: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let endpoint = endpoint.into();
        let label = format!("caldav:{}", redact_auth(&endpoint));
        Self {
            endpoint,
            user: user.into(),
            password: password.into(),
            client: reqwest::Client::new(),
            label,
        }
    }

    /// The XML body of the `calendar-query` REPORT. We ask for only the
    /// etag (so we could diff later if we add sync-tokens) and the full
    /// `calendar-data` payload, filtered to VEVENTs.
    fn report_body() -> &'static str {
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:getetag/>
    <c:calendar-data/>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT"/>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>
"#
    }
}

#[async_trait]
impl CalendarSource for CalDav {
    async fn fetch(&self) -> anyhow::Result<String> {
        // axum/reqwest don't have a REPORT helper; build the method by hand.
        let method = reqwest::Method::from_bytes(b"REPORT")?;
        let resp = self
            .client
            .request(method, &self.endpoint)
            .basic_auth(&self.user, Some(&self.password))
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(Self::report_body())
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!(
                "CalDAV REPORT failed: {} — body: {}",
                status,
                body.chars().take(500).collect::<String>()
            );
        }

        extract_calendar_data(&body)
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// Pull the text contents of every `<calendar-data>` element out of a
/// CalDAV multistatus XML response and concatenate them. The result is
/// valid ICS input (icalendar tolerates multiple VCALENDAR blocks).
fn extract_calendar_data(xml: &str) -> anyhow::Result<String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut depth_in = 0u32; // non-zero while inside a <calendar-data>

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if is_calendar_data(e.name().as_ref()) => {
                depth_in = depth_in.saturating_add(1);
            }
            Ok(Event::End(e)) if is_calendar_data(e.name().as_ref()) => {
                depth_in = depth_in.saturating_sub(1);
                // Separate successive VCALENDAR blocks with a blank line
                // so they parse independently.
                if depth_in == 0 && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Ok(Event::Text(t)) if depth_in > 0 => {
                out.push_str(&t.unescape()?);
            }
            Ok(Event::CData(c)) if depth_in > 0 => {
                out.push_str(std::str::from_utf8(c.as_ref())?);
            }
            Ok(Event::Eof) => break,
            Err(e) => anyhow::bail!("CalDAV xml parse error: {e}"),
            _ => {}
        }
    }

    if out.trim().is_empty() {
        anyhow::bail!("CalDAV response contained no <calendar-data> payloads");
    }
    Ok(out)
}

/// True for both `calendar-data` and `C:calendar-data` / `cal:calendar-data`
/// — we match on local name only, ignoring the namespace prefix.
fn is_calendar_data(qname: &[u8]) -> bool {
    let local = qname
        .rsplit(|b| *b == b':')
        .next()
        .unwrap_or(qname);
    local.eq_ignore_ascii_case(b"calendar-data")
}

/// Strip embedded credentials from a URL for log-safe display.
fn redact_auth(url: &str) -> String {
    if let Some((scheme_end, rest)) = url.split_once("://") {
        if let Some((_, tail)) = rest.split_once('@') {
            return format!("{scheme_end}://***@{tail}");
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_calendar_data_from_multistatus() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:response>
    <d:propstat>
      <d:prop>
        <c:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
END:VCALENDAR</c:calendar-data>
      </d:prop>
    </d:propstat>
  </d:response>
  <d:response>
    <d:propstat>
      <d:prop>
        <c:calendar-data>BEGIN:VCALENDAR
X:2
END:VCALENDAR</c:calendar-data>
      </d:prop>
    </d:propstat>
  </d:response>
</d:multistatus>"#;
        let out = extract_calendar_data(xml).unwrap();
        assert!(out.contains("BEGIN:VCALENDAR"));
        assert!(out.contains("X:2"));
    }

    #[test]
    fn redacts_basic_auth_in_url() {
        assert_eq!(
            redact_auth("https://user:pw@example.com/cal/"),
            "https://***@example.com/cal/"
        );
        assert_eq!(
            redact_auth("https://example.com/cal/"),
            "https://example.com/cal/"
        );
    }
}
