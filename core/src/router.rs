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
    routing::{delete, get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::device::{Device, Platform};
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
        .route("/devices", get(handle_list_devices).post(handle_register_device))
        .route("/devices/{token}", delete(handle_unregister_device))
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

    // Fan out to mobile push targets on a background task so the HTTP
    // caller doesn't wait on APNS. A slow APNS call would otherwise
    // balloon our /notify latency and potentially tip over upstream
    // proxies.
    if state.apns().is_some() && state.devices().is_some() {
        let state_for_push = state.clone();
        let notif_for_push = n.clone();
        tokio::spawn(async move {
            state_for_push.dispatch_push(&notif_for_push).await;
        });
    }

    let Some(rx) = pending_rx else {
        return StatusCode::ACCEPTED.into_response();
    };

    wait_and_respond(&n.id, rx, Duration::from_secs(wait_secs)).await
}

// --- /devices handlers ---

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterDeviceRequest {
    device_token: String,
    #[serde(default)]
    label: String,
    platform: Platform,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceListResponse {
    devices: Vec<Device>,
}

/// Returns 404 as JSON `{"error":"..."}` when push isn't configured on
/// this server. Cleaner than an empty 200 from the client's perspective.
fn no_registry_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": "device registry not configured on this server",
        })),
    )
        .into_response()
}

async fn handle_register_device(
    State(state): State<CoreState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !check_auth(&headers, state.secret()) {
        return unauthorized();
    }
    let Some(registry) = state.devices() else {
        return no_registry_response();
    };

    let req: RegisterDeviceRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    if req.device_token.is_empty() {
        return (StatusCode::BAD_REQUEST, "deviceToken is required").into_response();
    }

    let device = registry.register(req.device_token, req.label, req.platform).await;
    info!(
        platform = ?device.platform,
        label = %device.label,
        "registered push device",
    );
    (StatusCode::OK, axum::Json(device)).into_response()
}

async fn handle_unregister_device(
    State(state): State<CoreState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !check_auth(&headers, state.secret()) {
        return unauthorized();
    }
    let Some(registry) = state.devices() else {
        return no_registry_response();
    };
    let removed = registry.unregister(&token).await;
    if removed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn handle_list_devices(
    State(state): State<CoreState>,
    headers: HeaderMap,
) -> Response {
    if !check_auth(&headers, state.secret()) {
        return unauthorized();
    }
    let Some(registry) = state.devices() else {
        return no_registry_response();
    };
    let devices = registry.list().await;
    (StatusCode::OK, axum::Json(DeviceListResponse { devices })).into_response()
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

#[cfg(test)]
mod push_integration_tests {
    //! End-to-end: hit the real router with a real mock-APNS server in
    //! front of it, and prove the /notify path fans out to registered
    //! devices (and prunes them when APNS says they're dead).

    use super::*;
    use crate::device::DeviceRegistry;
    use crate::push::apns::{ApnsClient, ApnsConfig, ApnsKey};
    use crate::push::mock::MockApns;
    use axum::http::StatusCode as HttpStatus;
    use tokio::net::TcpListener;

    const SECRET: &str = "itest-secret";

    /// Test-only ES256 key; matches the fixture in apns.rs tests.
    const TEST_P8: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgevZzL1gdAFr88hb2\nOF/2NxApJCzGCEDdfSp6VQO30hyhRANCAAQRWz+jn65BtOMvdyHKcvjBeBSDZH2r\n1RTwjmYSi9R/zpBnuQ4EiMnCqfMPWiZqB4QdbAd0E7oH50VpuZ1P087G\n-----END PRIVATE KEY-----\n";

    struct Harness {
        base: String,
        state: CoreState,
        mock: MockApns,
    }

    async fn spawn_harness() -> Harness {
        let mock = MockApns::spawn().await;
        let cfg = ApnsConfig {
            base_url: mock.base_url.clone(),
            team_id: "TEAM123456".into(),
            key_id: "KEYID67890".into(),
            bundle_id: "com.example.test".into(),
            key: ApnsKey::Pem(TEST_P8.as_bytes().to_vec()),
        };
        let state = CoreState::new(SECRET)
            .with_device_registry(DeviceRegistry::in_memory())
            .with_apns(ApnsClient::new(cfg));

        let app = router(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Harness {
            base: format!("http://{addr}"),
            state,
            mock,
        }
    }

    async fn register(base: &str, http: &reqwest::Client, token: &str, label: &str) {
        let resp = http
            .post(format!("{base}/devices"))
            .bearer_auth(SECRET)
            .json(&serde_json::json!({
                "deviceToken": token,
                "label": label,
                "platform": "ios",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), HttpStatus::OK);
    }

    async fn post_notify(base: &str, http: &reqwest::Client, title: &str) {
        let resp = http
            .post(format!("{base}/notify"))
            .bearer_auth(SECRET)
            .json(&serde_json::json!({
                "source": "itest",
                "title": title,
                "message": "body",
            }))
            .send()
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "notify failed: {}",
            resp.status()
        );
    }

    /// Spin until the mock has recorded at least `n` pushes, or fail the
    /// test after ~2s. Background dispatch means /notify returns before
    /// APNS sees the request.
    async fn wait_for_pushes(mock: &MockApns, n: usize) {
        for _ in 0..40 {
            if mock.requests().len() >= n {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!(
            "expected >= {n} pushes to mock APNS, got {}",
            mock.requests().len()
        );
    }

    #[tokio::test]
    async fn notify_fans_out_to_registered_devices() {
        let h = spawn_harness().await;
        let http = reqwest::Client::new();

        register(&h.base, &http, "tok-a", "A").await;
        register(&h.base, &http, "tok-b", "B").await;
        post_notify(&h.base, &http, "hello").await;

        wait_for_pushes(&h.mock, 2).await;
        let tokens: std::collections::HashSet<String> = h
            .mock
            .requests()
            .into_iter()
            .map(|r| r.token)
            .collect();
        assert_eq!(tokens.len(), 2);
        assert!(tokens.contains("tok-a"));
        assert!(tokens.contains("tok-b"));
        assert_eq!(h.state.devices().unwrap().count().await, 2);
    }

    #[tokio::test]
    async fn dead_tokens_get_pruned_from_registry() {
        let h = spawn_harness().await;
        let http = reqwest::Client::new();

        register(&h.base, &http, "alive", "alive").await;
        register(&h.base, &http, "dead", "dead").await;
        h.mock
            .set_response("dead", HttpStatus::GONE, "Unregistered");
        post_notify(&h.base, &http, "hi").await;

        // Wait for both pushes to be attempted.
        wait_for_pushes(&h.mock, 2).await;
        // Registry prune runs after the dispatch loop finishes; poll.
        for _ in 0..40 {
            if h.state.devices().unwrap().count().await == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let remaining: Vec<String> = h
            .state
            .devices()
            .unwrap()
            .list()
            .await
            .into_iter()
            .map(|d| d.token)
            .collect();
        assert_eq!(remaining, vec!["alive".to_string()]);
    }

    #[tokio::test]
    async fn exclusive_notifications_skip_push() {
        let h = spawn_harness().await;
        let http = reqwest::Client::new();

        register(&h.base, &http, "tok", "x").await;
        // Exclusive via `wait` — fires the skip-for-mobile branch.
        let resp = http
            .post(format!("{}/notify", h.base))
            .bearer_auth(SECRET)
            .json(&serde_json::json!({
                "source": "itest",
                "title": "ex",
                "message": "m",
                "wait": 1,
                "maxWait": 1,
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success() || resp.status() == HttpStatus::GONE);

        // Give the dispatch task a beat to possibly (incorrectly) fire.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            h.mock.requests().len(),
            0,
            "exclusive notifications must not push to mobile",
        );
    }

    #[tokio::test]
    async fn register_and_unregister_device() {
        let h = spawn_harness().await;
        let http = reqwest::Client::new();

        register(&h.base, &http, "tok", "Phone").await;
        let listing: serde_json::Value = http
            .get(format!("{}/devices", h.base))
            .bearer_auth(SECRET)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let devices = listing["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["label"], "Phone");

        let del = http
            .delete(format!("{}/devices/tok", h.base))
            .bearer_auth(SECRET)
            .send()
            .await
            .unwrap();
        assert_eq!(del.status(), HttpStatus::NO_CONTENT);
        assert_eq!(h.state.devices().unwrap().count().await, 0);
    }

    #[tokio::test]
    async fn devices_endpoint_404s_when_registry_disabled() {
        let state = CoreState::new(SECRET);
        let app = router(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base = format!("http://{addr}");

        let resp = reqwest::Client::new()
            .get(format!("{base}/devices"))
            .bearer_auth(SECRET)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), HttpStatus::NOT_FOUND);
    }
}
