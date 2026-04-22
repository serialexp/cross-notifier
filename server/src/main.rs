//! Headless notification server. Thin entry point around `cross_notifier_core::router`.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use cross_notifier_core::{
    CoreState,
    device::DeviceRegistry,
    push::{ApnsClient, ApnsConfig, ApnsEnvironment, ApnsKey},
    router,
};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
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
        self.team_id.get_or_insert_with(|| env::var("APNS_TEAM_ID").unwrap_or_default());
        self.key_id.get_or_insert_with(|| env::var("APNS_KEY_ID").unwrap_or_default());
        self.bundle_id.get_or_insert_with(|| env::var("APNS_BUNDLE_ID").unwrap_or_default());
        if self.key_path.is_none() {
            if let Ok(p) = env::var("APNS_P8_KEY_PATH") {
                if !p.is_empty() {
                    self.key_path = Some(PathBuf::from(p));
                }
            }
        }
        if self.key_base64.is_none() {
            if let Ok(b) = env::var("APNS_P8_KEY_BASE64") {
                if !b.is_empty() {
                    self.key_base64 = Some(b);
                }
            }
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
        let mut take =
            |flag: &str| it.next().with_context(|| format!("missing value for {flag}"));
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
        .or_else(|| env::var("CROSS_NOTIFIER_PORT").ok().and_then(|v| v.parse().ok()))
        .unwrap_or(9876);

    let secret = secret
        .or_else(|| env::var("CROSS_NOTIFIER_SECRET").ok())
        .filter(|s| !s.is_empty())
        .context("secret required: pass -secret or set CROSS_NOTIFIER_SECRET")?;

    if devices_file.is_none() {
        if let Ok(p) = env::var("CROSS_NOTIFIER_DEVICES_FILE") {
            if !p.is_empty() {
                devices_file = Some(PathBuf::from(p));
            }
        }
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
    let mut state = CoreState::new(args.secret);

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

    let app = router(state);

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

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
