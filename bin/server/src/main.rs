//! Headless notification server. Thin entry point around `cross_notifier_core::router`.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use cross_notifier_calendar::{
    CalDav, CalendarService, CalendarServiceConfig, CalendarSource, CoreNotifier,
    CoreNotifierConfig, IcsFile, IcsUrl, JsonStore, calendar_action_router,
};
use cross_notifier_core::{
    CoreState,
    device::DeviceRegistry,
    protocol::{ServerCalendarInfo, ServerInfoMessage},
    push::{ApnsClient, ApnsConfig, ApnsEnvironment, ApnsKey},
    router,
};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = parse_args()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(args))
}

struct Args {
    port: u16,
    secret: String,
    devices_file: Option<PathBuf>,
    apns: Option<ApnsConfig>,
}

#[derive(Default)]
struct ApnsCliParts {
    team_id: Option<String>,
    key_id: Option<String>,
    bundle_id: Option<String>,
    key_path: Option<PathBuf>,
    key_base64: Option<String>,
    environment: Option<String>,
}

impl ApnsCliParts {
    /// Merge in defaults from env vars for any field the CLI left unset.
    fn fill_from_env(&mut self) {
        self.team_id
            .get_or_insert_with(|| env::var("APNS_TEAM_ID").unwrap_or_default());
        self.key_id
            .get_or_insert_with(|| env::var("APNS_KEY_ID").unwrap_or_default());
        self.bundle_id
            .get_or_insert_with(|| env::var("APNS_BUNDLE_ID").unwrap_or_default());
        if self.key_path.is_none()
            && let Ok(p) = env::var("APNS_P8_KEY_PATH")
            && !p.is_empty()
        {
            self.key_path = Some(PathBuf::from(p));
        }
        if self.key_base64.is_none()
            && let Ok(b) = env::var("APNS_P8_KEY_BASE64")
            && !b.is_empty()
        {
            self.key_base64 = Some(b);
        }
        self.environment
            .get_or_insert_with(|| env::var("APNS_ENVIRONMENT").unwrap_or_default());

        // Empty strings behave like "unset" so the presence check below
        // treats `-apns-team-id ""` the same as absence.
        for field in [
            &mut self.team_id,
            &mut self.key_id,
            &mut self.bundle_id,
            &mut self.environment,
        ] {
            if field.as_deref() == Some("") {
                *field = None;
            }
        }
    }

    /// Build an ApnsConfig if everything necessary is present; return
    /// `Ok(None)` if nothing at all was configured (push disabled); error
    /// if the config is half-filled.
    fn into_config(self) -> Result<Option<ApnsConfig>> {
        let ApnsCliParts {
            team_id,
            key_id,
            bundle_id,
            key_path,
            key_base64,
            environment,
        } = self;

        let has_any = team_id.is_some()
            || key_id.is_some()
            || bundle_id.is_some()
            || key_path.is_some()
            || key_base64.is_some()
            || environment.is_some();
        if !has_any {
            return Ok(None);
        }

        let team_id = team_id.context("APNS_TEAM_ID required when APNS is configured")?;
        let key_id = key_id.context("APNS_KEY_ID required when APNS is configured")?;
        let bundle_id = bundle_id.context("APNS_BUNDLE_ID required when APNS is configured")?;

        if key_path.is_some() && key_base64.is_some() {
            bail!(
                "APNS_P8_KEY_PATH and APNS_P8_KEY_BASE64 are mutually exclusive — \
                 set one or the other"
            );
        }
        let key = match (key_path, key_base64) {
            (Some(p), None) => ApnsKey::from_file(&p)
                .with_context(|| format!("reading APNS key at {}", p.display()))?,
            (None, Some(b)) => ApnsKey::from_base64(&b)
                .context("decoding APNS_P8_KEY_BASE64 (expect base64-encoded .p8 PEM)")?,
            (None, None) => bail!(
                "APNS is partially configured but no key provided — \
                 set APNS_P8_KEY_PATH or APNS_P8_KEY_BASE64"
            ),
            (Some(_), Some(_)) => unreachable!("guarded above"),
        };

        let environment = match environment.as_deref().unwrap_or("production") {
            "production" | "prod" => ApnsEnvironment::Production,
            "sandbox" | "development" | "dev" => ApnsEnvironment::Sandbox,
            other => bail!(
                "APNS_ENVIRONMENT must be 'production' or 'sandbox', got {:?}",
                other
            ),
        };

        Ok(Some(ApnsConfig::for_environment(
            environment,
            team_id,
            key_id,
            bundle_id,
            key,
        )))
    }
}

fn parse_args() -> Result<Args> {
    let mut port: Option<u16> = None;
    let mut secret: Option<String> = None;
    let mut devices_file: Option<PathBuf> = None;
    let mut apns = ApnsCliParts::default();

    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        let mut take = |flag: &str| {
            it.next()
                .with_context(|| format!("missing value for {flag}"))
        };
        match a.as_str() {
            "-port" | "--port" => {
                port = Some(take("-port")?.parse()?);
            }
            "-secret" | "--secret" => {
                secret = Some(take("-secret")?);
            }
            "-devices-file" | "--devices-file" => {
                devices_file = Some(PathBuf::from(take("-devices-file")?));
            }
            "-apns-team-id" | "--apns-team-id" => {
                apns.team_id = Some(take("-apns-team-id")?);
            }
            "-apns-key-id" | "--apns-key-id" => {
                apns.key_id = Some(take("-apns-key-id")?);
            }
            "-apns-bundle-id" | "--apns-bundle-id" => {
                apns.bundle_id = Some(take("-apns-bundle-id")?);
            }
            "-apns-key" | "--apns-key" | "-apns-p8-path" | "--apns-p8-path" => {
                apns.key_path = Some(PathBuf::from(take("-apns-key")?));
            }
            "-apns-key-base64" | "--apns-key-base64" => {
                apns.key_base64 = Some(take("-apns-key-base64")?);
            }
            "-apns-env" | "--apns-env" | "-apns-environment" | "--apns-environment" => {
                apns.environment = Some(take("-apns-env")?);
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let port = port
        .or_else(|| {
            env::var("CROSS_NOTIFIER_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(9876);

    let secret = secret
        .or_else(|| env::var("CROSS_NOTIFIER_SECRET").ok())
        .filter(|s| !s.is_empty())
        .context("secret required: pass -secret or set CROSS_NOTIFIER_SECRET")?;

    if devices_file.is_none()
        && let Ok(p) = env::var("CROSS_NOTIFIER_DEVICES_FILE")
        && !p.is_empty()
    {
        devices_file = Some(PathBuf::from(p));
    }

    apns.fill_from_env();
    let apns = apns.into_config()?;

    Ok(Args {
        port,
        secret,
        devices_file,
        apns,
    })
}

fn print_usage() {
    eprintln!(
        "cross-notifier-server [options]\n\n\
         -port N                  Listen port (default 9876, env CROSS_NOTIFIER_PORT)\n\
         -secret STR              Shared secret (env CROSS_NOTIFIER_SECRET)\n\
         -devices-file PATH       Device registry JSON (env CROSS_NOTIFIER_DEVICES_FILE)\n\n\
         APNS (all required to enable mobile push):\n\
         -apns-team-id STR        Apple developer team ID (env APNS_TEAM_ID)\n\
         -apns-key-id STR         APNS auth key ID (env APNS_KEY_ID)\n\
         -apns-bundle-id STR      iOS app bundle identifier (env APNS_BUNDLE_ID)\n\
         -apns-key PATH           Path to the .p8 signing key (env APNS_P8_KEY_PATH)\n\
         -apns-key-base64 STR     OR base64-encoded .p8 contents (env APNS_P8_KEY_BASE64)\n\
         -apns-env ENV            'production' (default) or 'sandbox' (env APNS_ENVIRONMENT)\n\n\
         -h, --help               Show this help\n"
    );
}

async fn run(args: Args) -> Result<()> {
    // Keep a copy for the calendar action router auth — state owns one,
    // the router owns one, CLI args stay logically immutable.
    let secret = args.secret.clone();
    let mut state = CoreState::new(secret.clone());

    if let Some(path) = args.devices_file.as_ref() {
        let reg = DeviceRegistry::from_file(path)
            .await
            .with_context(|| format!("loading device registry at {}", path.display()))?;
        state = state.with_device_registry(reg);
        tracing::info!(path = %path.display(), "device registry enabled");
    } else {
        tracing::info!("device registry: in-memory (registrations lost on restart)");
    }

    match args.apns.as_ref() {
        Some(cfg) => {
            tracing::info!(
                bundle_id = %cfg.bundle_id,
                base_url = %cfg.base_url,
                "APNS push enabled",
            );
            state = state.with_apns(ApnsClient::new(cfg.clone()));
            // If APNS is on but no persistent registry, loudly warn — any
            // device registration is lost on restart and tokens go dead.
            if args.devices_file.is_none() {
                tracing::warn!(
                    "APNS enabled without -devices-file: registrations will be \
                     lost on restart and iOS push will silently stop working"
                );
            }
        }
        None => tracing::info!("APNS push: disabled (no configuration)"),
    }

    // Make sure the registry exists whenever APNS is enabled so the
    // /devices endpoints work — fall back to in-memory if the operator
    // hasn't opted into persistence.
    if state.apns().is_some() && state.devices().is_none() {
        state = state.with_device_registry(DeviceRegistry::in_memory());
    }

    // Optionally attach the calendar service. Driven purely by env vars
    // for the server (vs. the daemon, which plumbs this through its
    // persistent config). The calendar is off unless a source is set.
    //
    // We build the source up front so we can populate the
    // ServerInfoMessage *before* the first state clone — `with_server_info`
    // uses `Arc::get_mut`, which only succeeds while we're the sole owner.
    let calendar_source = build_calendar_source();
    let mut server_info = ServerInfoMessage::default();
    if let Some(src) = calendar_source.as_ref() {
        server_info.calendars.push(ServerCalendarInfo {
            kind: src.kind().to_string(),
            label: src.label().to_string(),
            fingerprint: src.fingerprint(),
        });
    }
    state = state.with_server_info(server_info);

    let calendar = if let Some(src) = calendar_source {
        Some(spawn_calendar(src, &args, &secret, state.clone()).await?)
    } else {
        tracing::info!("calendar: disabled (no source configured)");
        None
    };

    let mut app = router(state);
    if let Some(calendar) = calendar.as_ref() {
        // The server keeps the calendar service for the lifetime of the
        // process, but we still go through a slot so the router API is
        // uniform with the daemon (which swaps the handle on reload).
        let slot = cross_notifier_calendar::CalendarHandleSlot::new();
        slot.set(Some(calendar.handle()));
        app = app.nest(
            "/calendar/action",
            calendar_action_router(slot, Some(secret.clone())),
        );
    }

    let addr: SocketAddr = ([0, 0, 0, 0], args.port).into();
    tracing::info!("Notification server listening on {addr}");
    tracing::info!("  POST   /notify          - send notifications (requires auth)");
    tracing::info!("  GET    /notify/:id/wait - long-poll response (requires auth)");
    tracing::info!("  GET    /ws              - WebSocket for clients (requires auth)");
    tracing::info!("  POST   /devices         - register push device (requires auth)");
    tracing::info!("  GET    /devices         - list push devices (requires auth)");
    tracing::info!("  DELETE /devices/:token  - unregister device (requires auth)");
    tracing::info!("  GET    /health          - health check (no auth)");
    tracing::info!("  GET    /openapi.yaml    - OpenAPI spec, YAML (no auth)");
    tracing::info!("  GET    /openapi.json    - OpenAPI spec, JSON (no auth)");
    if calendar.is_some() {
        tracing::info!(
            "  POST   /calendar/action/snooze  - snooze calendar reminder (requires auth)"
        );
        tracing::info!(
            "  POST   /calendar/action/dismiss - dismiss calendar reminder (requires auth)"
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Build a calendar source from env. Returns `None` when nothing is
/// configured. Three selectors are honored, in priority order:
///   1. `CAL_ICS_FILE`  — local .ics file (handy for testing)
///   2. `CAL_ICS_URL`   — public ICS subscription URL (with optional basic auth)
///   3. `CALDAV_*` set  — full CalDAV with per-calendar credentials
fn build_calendar_source() -> Option<Arc<dyn CalendarSource>> {
    if let Ok(path) = env::var("CAL_ICS_FILE") {
        Some(Arc::new(IcsFile::new(path)))
    } else if let Ok(url) = env::var("CAL_ICS_URL") {
        let mut s = IcsUrl::new(url);
        if let (Ok(u), Ok(p)) = (env::var("CAL_ICS_USER"), env::var("CAL_ICS_PASSWORD")) {
            s = s.with_basic_auth(u, p);
        }
        Some(Arc::new(s))
    } else if let (Ok(endpoint), Ok(user), Ok(password)) = (
        env::var("CALDAV_ENDPOINT"),
        env::var("CALDAV_USER"),
        env::var("CALDAV_PASSWORD"),
    ) {
        Some(Arc::new(CalDav::new(endpoint, user, password)))
    } else {
        None
    }
}

/// Spawn a `CalendarService` over an already-constructed source.
///
/// Persistence: `CAL_STATE_FILE` (defaults to `./calendar-state.json`).
/// Horizon: `CAL_HORIZON_HOURS` (default 48).
/// Refresh: `CAL_REFRESH_MINUTES` (default 5).
async fn spawn_calendar(
    source: Arc<dyn CalendarSource>,
    args: &Args,
    secret: &str,
    state: CoreState,
) -> Result<CalendarService> {
    let horizon_hours: i64 = env::var("CAL_HORIZON_HOURS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(48);
    let refresh_minutes: i64 = env::var("CAL_REFRESH_MINUTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let state_file =
        env::var("CAL_STATE_FILE").unwrap_or_else(|_| "calendar-state.json".to_string());

    let store = Arc::new(JsonStore::new(state_file.clone()));
    // The server always listens on 0.0.0.0; remote daemons will POST
    // actions back here via their public URL, so the base URL needs to
    // include whatever hostname the operator uses. Let them override it
    // via CAL_ACTION_BASE_URL; otherwise assume localhost (good for
    // single-box deployments).
    let action_base_url = env::var("CAL_ACTION_BASE_URL")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{}/calendar/action", args.port));

    let notifier = Arc::new(CoreNotifier::new(
        state,
        CoreNotifierConfig {
            action_base_url,
            action_auth: Some(secret.to_string()),
            snooze_hours: 4,
        },
    ));

    // Daily summary: set CAL_SUMMARY_AT="HH:MM" to enable. Omitting the
    // env var disables the feature. Invalid values skip summary with a
    // warning — silently ignoring would be worse.
    let daily_summary = env::var("CAL_SUMMARY_AT").ok().and_then(|v| {
        let (h_s, m_s) = v.split_once(':')?;
        let h: u32 = h_s.parse().ok()?;
        let m: u32 = m_s.parse().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        Some(cross_notifier_calendar::DailySummaryConfig { hour: h, minute: m })
    });
    if env::var("CAL_SUMMARY_AT").is_ok() && daily_summary.is_none() {
        tracing::warn!("calendar: CAL_SUMMARY_AT is not HH:MM; summary disabled");
    }

    let cfg = CalendarServiceConfig {
        horizon: chrono::Duration::hours(horizon_hours),
        refresh_interval: chrono::Duration::minutes(refresh_minutes),
        daily_summary,
    };

    let svc = CalendarService::spawn(source.clone(), notifier, store, cfg)
        .await
        .context("spawning calendar service")?;

    tracing::info!(
        source = source.label(),
        horizon_hours,
        refresh_minutes,
        state_file,
        "calendar: enabled",
    );

    Ok(svc)
}
