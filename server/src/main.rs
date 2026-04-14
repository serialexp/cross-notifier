//! Headless notification server. Thin entry point around `cross_notifier_core::router`.

use std::env;
use std::net::SocketAddr;

use anyhow::{Context, Result};
use cross_notifier_core::{CoreState, router};
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
}

fn parse_args() -> Result<Args> {
    let mut port: Option<u16> = None;
    let mut secret: Option<String> = None;

    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-port" | "--port" => {
                port = Some(it.next().context("missing value for -port")?.parse()?);
            }
            "-secret" | "--secret" => {
                secret = Some(it.next().context("missing value for -secret")?);
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

    Ok(Args { port, secret })
}

fn print_usage() {
    eprintln!(
        "cross-notifier-server [options]\n\n\
         -port N          Listen port (default 9876, env CROSS_NOTIFIER_PORT)\n\
         -secret STR      Shared secret (env CROSS_NOTIFIER_SECRET)\n\
         -h, --help       Show this help\n"
    );
}

async fn run(args: Args) -> Result<()> {
    let state = CoreState::new(args.secret);
    let app = router(state);

    let addr: SocketAddr = ([0, 0, 0, 0], args.port).into();
    tracing::info!("Notification server listening on {addr}");
    tracing::info!("  POST /notify          - send notifications (requires auth)");
    tracing::info!("  GET  /notify/:id/wait - long-poll response (requires auth)");
    tracing::info!("  GET  /ws              - WebSocket for clients (requires auth)");
    tracing::info!("  GET  /health          - health check (no auth)");
    tracing::info!("  GET  /openapi.yaml    - OpenAPI spec, YAML (no auth)");
    tracing::info!("  GET  /openapi.json    - OpenAPI spec, JSON (no auth)");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
