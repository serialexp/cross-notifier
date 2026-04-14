//! HTTP + WebSocket router factory. Consumers mount this under whatever
//! prefix they want; in practice both the server binary and the daemon
//! mount it at `/`.

use std::time::Duration;

use axum::{
    Router,
    body::Bytes,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::openapi;
use crate::protocol::{
    ActionMessage, ExpiredMessage, Message, MessageType, Notification, PendingResponse,
};
use crate::state::{CoreState, DEFAULT_WAIT_SECS, PendingState};
use crate::subscriber::{OutboundMessage, Subscriber};

/// Build the axum router. The caller is responsible for adding any extra
/// routes and for calling `into_make_service()` / binding to an address.
pub fn router(state: CoreState) -> Router {
    Router::new()
        .route("/notify", post(handle_notify))
        .route("/notify/{id}/wait", get(handle_wait))
        .route("/ws", get(handle_ws))
        .route("/health", get(|| async { "ok" }))
        .route(
            "/openapi.yaml",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/yaml; charset=utf-8")],
                    openapi::YAML,
                )
            }),
        )
        .route(
            "/openapi.json",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                    openapi::json(),
                )
            }),
        )
        .with_state(state)
}

fn check_auth(headers: &HeaderMap, secret: &str) -> bool {
    // An empty configured secret means "auth disabled" — useful for the
    // daemon's localhost-only /notify endpoint.
    if secret.is_empty() {
        return true;
    }
    let Some(val) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(s) = val.to_str() else {
        return false;
    };
    s.strip_prefix("Bearer ").map(|t| t == secret).unwrap_or(false)
}

/// Builds a 401 response used by every handler when auth fails.
fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

async fn handle_notify(
    State(state): State<CoreState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !check_auth(&headers, state.secret()) {
        return unauthorized();
    }

    let mut n: Notification = match serde_json::from_slice(&body) {
        Ok(n) => n,
        Err(e) => {
            warn!("failed to decode notification json: {e}");
            return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
    };

    if n.source.is_empty() {
        return (StatusCode::BAD_REQUEST, "source is required").into_response();
    }
    if n.title.is_empty() && n.message.is_empty() {
        return (StatusCode::BAD_REQUEST, "title or message required").into_response();
    }

    if !n.icon_href.is_empty() {
        match crate::icon::fetch_and_encode(&n.icon_href).await {
            Ok(b64) => {
                n.icon_data = b64;
                n.icon_href.clear();
            }
            Err(e) => warn!(url = %n.icon_href, "icon fetch failed: {e}"),
        }
    }

    if n.wait > 0 || n.max_wait > 0 {
        n.exclusive = true;
    }

    if n.exclusive && n.id.is_empty() {
        n.id = uuid::Uuid::new_v4().to_string();
    }

    let wait_secs = if n.wait == 0 { DEFAULT_WAIT_SECS } else { n.wait };
    let max_wait_secs = n.max_wait.max(wait_secs);

    let pending_rx = if n.exclusive {
        Some(
            state
                .register_pending(n.clone(), Duration::from_secs(max_wait_secs))
                .await,
        )
    } else {
        None
    };

    state.broadcast(OutboundMessage::Notification(n.clone())).await;

    let Some(rx) = pending_rx else {
        return StatusCode::ACCEPTED.into_response();
    };

    wait_and_respond(&n.id, rx, Duration::from_secs(wait_secs)).await
}

async fn wait_and_respond(
    id: &str,
    mut rx: watch::Receiver<PendingState>,
    wait_for: Duration,
) -> Response {
    // If it's already terminal (e.g. raced with a very short maxWait),
    // return immediately without sleeping.
    if !matches!(*rx.borrow(), PendingState::Live) {
        return terminal_response(id, rx.borrow().clone());
    }

    let res = tokio::time::timeout(wait_for, async {
        loop {
            if rx.changed().await.is_err() {
                // Sender dropped — shouldn't happen while the notification
                // is live, but treat it as expiry for safety.
                return PendingState::Expired;
            }
            let s = rx.borrow().clone();
            if !matches!(s, PendingState::Live) {
                return s;
            }
        }
    })
    .await;

    match res {
        Ok(state) => terminal_response(id, state),
        Err(_) => {
            // Short wait elapsed; tell the caller to long-poll.
            let body = PendingResponse::new(id.to_string());
            let mut resp = (StatusCode::ACCEPTED, axum::Json(body)).into_response();
            resp.headers_mut().insert(
                header::LOCATION,
                HeaderValue::from_str(&format!("/notify/{id}/wait"))
                    .expect("notification id is URL-safe (UUID)"),
            );
            resp
        }
    }
}

fn terminal_response(id: &str, state: PendingState) -> Response {
    match state {
        PendingState::Resolved(r) => (StatusCode::OK, axum::Json(r)).into_response(),
        PendingState::Expired => (
            StatusCode::GONE,
            axum::Json(ExpiredMessage {
                notification_id: id.to_string(),
            }),
        )
            .into_response(),
        PendingState::Live => unreachable!("terminal_response called with Live state"),
    }
}

#[derive(Deserialize)]
struct WaitQuery {
    timeout: Option<u64>,
}

async fn handle_wait(
    State(state): State<CoreState>,
    Path(id): Path<String>,
    Query(q): Query<WaitQuery>,
    headers: HeaderMap,
) -> Response {
    if !check_auth(&headers, state.secret()) {
        return unauthorized();
    }

    let Some(rx) = state.watch_pending(&id).await else {
        // Unknown or already cleaned up — treat as expired.
        return (
            StatusCode::GONE,
            axum::Json(ExpiredMessage {
                notification_id: id,
            }),
        )
            .into_response();
    };

    let wait_for = Duration::from_secs(q.timeout.filter(|s| *s > 0).unwrap_or(DEFAULT_WAIT_SECS));
    wait_and_respond(&id, rx, wait_for).await
}

async fn handle_ws(
    ws: WebSocketUpgrade,
    State(state): State<CoreState>,
    headers: HeaderMap,
) -> Response {
    if !check_auth(&headers, state.secret()) {
        return unauthorized();
    }

    let client_name = headers
        .get("x-client-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    ws.on_upgrade(move |socket| ws_client_loop(socket, state, client_name))
}

async fn ws_client_loop(socket: WebSocket, state: CoreState, client_name: String) {
    let (subscriber, mut outbound_rx) = Subscriber::new(client_name.clone());
    state.add_subscriber(subscriber).await;
    info!(client = %client_name, "client connected");

    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            out = outbound_rx.recv() => {
                let Some(msg) = out else { break };
                let encoded = match encode_outbound(&msg) {
                    Ok(s) => s,
                    Err(e) => { warn!("encode outbound: {e}"); continue; }
                };
                if sink.send(WsMessage::Text(encoded.into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => {
                        let msg = match Message::decode(&text) {
                            Ok(m) => m,
                            Err(e) => { warn!("decode inbound: {e}"); continue; }
                        };
                        if msg.msg_type == MessageType::Action {
                            match serde_json::from_value::<ActionMessage>(msg.data) {
                                Ok(am) => state.handle_action(&client_name, am).await,
                                Err(e) => warn!("decode action: {e}"),
                            }
                        }
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        if sink.send(WsMessage::Pong(data)).await.is_err() { break; }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Err(e)) => { debug!("ws error: {e}"); break; }
                    _ => {}
                }
            }
        }
    }

    info!(client = %client_name, "client disconnected");
}

fn encode_outbound(msg: &OutboundMessage) -> anyhow::Result<String> {
    match msg {
        OutboundMessage::Notification(n) => Message::encode(MessageType::Notification, n),
        OutboundMessage::Resolved(r) => Message::encode(MessageType::Resolved, r),
        OutboundMessage::Expired(e) => Message::encode(MessageType::Expired, e),
    }
}
