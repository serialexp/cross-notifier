// WebSocket client with automatic reconnection.
// Connects to a remote notification server, authenticates via Bearer token,
// and forwards notifications to the main thread via EventLoopProxy.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;
use tracing::{error, info, warn};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::notification::NotificationPayload;
use crate::protocol::{Message, MessageType, ResolvedMessage};

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const PING_INTERVAL: Duration = Duration::from_secs(30);
const GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Spawns a WebSocket client task that reconnects automatically.
/// Returns a handle that can be used to shut down the client.
pub fn spawn_client(
    url: String,
    secret: String,
    client_name: String,
    server_label: String,
    event_proxy: EventLoopProxy<AppEvent>,
) -> ClientHandle {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let handle = tokio::spawn(client_loop(
        url,
        secret,
        client_name,
        server_label,
        event_proxy,
        shutdown_rx,
    ));

    ClientHandle {
        _shutdown: shutdown_tx,
        _task: handle,
    }
}

pub struct ClientHandle {
    _shutdown: tokio::sync::oneshot::Sender<()>,
    _task: tokio::task::JoinHandle<()>,
}

async fn client_loop(
    url: String,
    secret: String,
    client_name: String,
    server_label: String,
    event_proxy: EventLoopProxy<AppEvent>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut backoff = MIN_BACKOFF;
    let mut was_connected = false;

    loop {
        if shutdown.try_recv().is_ok() {
            info!("Client for {} shutting down", server_label);
            return;
        }

        match connect_and_run(&url, &secret, &client_name, &server_label, &event_proxy).await {
            Ok(()) => {
                info!("Disconnected from {}", server_label);
                backoff = MIN_BACKOFF;
            }
            Err(e) => {
                if was_connected {
                    tokio::time::sleep(GRACE_PERIOD).await;
                    warn!("Connection to {} lost: {}", server_label, e);
                } else {
                    warn!("Failed to connect to {}: {}", server_label, e);
                }
            }
        }

        was_connected = true;
        let _ = event_proxy.send_event(AppEvent::ConnectionStatus {
            server_url: url.clone(),
            connected: false,
        });

        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = &mut shutdown => {
                info!("Client for {} shutting down during backoff", server_label);
                return;
            }
        }

        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn connect_and_run(
    url: &str,
    secret: &str,
    client_name: &str,
    server_label: &str,
    event_proxy: &EventLoopProxy<AppEvent>,
) -> anyhow::Result<()> {
    let request = tungstenite::http::Request::builder()
        .uri(url)
        .header("Authorization", format!("Bearer {}", secret))
        .header("X-Client-Name", client_name)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .body(())?;

    let (ws_stream, _) = tokio_tungstenite::connect_async(request).await?;
    let (mut write, mut read) = ws_stream.split();

    info!("Connected to {}", server_label);
    let _ = event_proxy.send_event(AppEvent::ConnectionStatus {
        server_url: url.to_string(),
        connected: true,
    });

    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await; // consume immediate tick

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        handle_message(&text, server_label, event_proxy);
                    }
                    Some(Ok(tungstenite::Message::Ping(data))) => {
                        let _ = write.send(tungstenite::Message::Pong(data)).await;
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        return Ok(());
                    }
                    Some(Err(e)) => {
                        return Err(e.into());
                    }
                    _ => {}
                }
            }
            _ = ping_interval.tick() => {
                if let Err(e) = write.send(tungstenite::Message::Ping(vec![].into())).await {
                    return Err(e.into());
                }
            }
        }
    }
}

fn handle_message(
    text: &str,
    server_label: &str,
    event_proxy: &EventLoopProxy<AppEvent>,
) {
    let msg = match Message::decode(text) {
        Ok(m) => m,
        Err(e) => {
            error!("Failed to decode message from {}: {}", server_label, e);
            return;
        }
    };

    match msg.msg_type {
        MessageType::Notification => {
            match serde_json::from_value::<NotificationPayload>(msg.data) {
                Ok(payload) => {
                    let _ = event_proxy.send_event(AppEvent::IncomingNotification {
                        server_label: server_label.to_string(),
                        payload,
                    });
                }
                Err(e) => {
                    error!("Failed to parse notification from {}: {}", server_label, e);
                }
            }
        }
        MessageType::Resolved => {
            match serde_json::from_value::<ResolvedMessage>(msg.data) {
                Ok(resolved) => {
                    let _ = event_proxy.send_event(AppEvent::NotificationResolved(resolved));
                }
                Err(e) => {
                    error!("Failed to parse resolved message from {}: {}", server_label, e);
                }
            }
        }
        MessageType::Action => {
            warn!("Unexpected action message from server {}", server_label);
        }
    }
}
