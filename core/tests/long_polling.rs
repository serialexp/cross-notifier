//! End-to-end tests: drive the router over real TCP + real WebSockets and
//! assert the wait / long-poll / expiry behaviour.

use std::time::Duration;

use cross_notifier_core::{
    Action, ActionMessage, ExpiredMessage, Message, MessageType, Notification, PendingResponse,
    ResolvedMessage, router, state::CoreState,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{
    Message as WsMessage,
    client::IntoClientRequest,
    http::{HeaderValue, StatusCode as WsStatus},
};

const SECRET: &str = "test-secret";

struct Harness {
    base: String,
    state: CoreState,
}

async fn spawn() -> Harness {
    let state = CoreState::new(SECRET);
    let app = router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Harness {
        base: format!("http://{addr}"),
        state,
    }
}

/// Small-body echo target so action-URL HTTP calls succeed.
async fn spawn_echo() -> String {
    use axum::{Router, routing::any};
    let app: Router = Router::new().route(
        "/",
        any(|| async { axum::http::StatusCode::OK }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn connect_ws(
    base: &str,
    name: &str,
) -> (
    futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WsMessage,
    >,
    futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) {
    let url = format!("{}/ws", base.replace("http", "ws"));
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {SECRET}")).unwrap(),
    );
    req.headers_mut()
        .insert("x-client-name", HeaderValue::from_str(name).unwrap());
    let (stream, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    stream.split()
}

async fn read_notification_id<S>(stream: &mut S) -> String
where
    S: futures_util::Stream<
            Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), stream.next())
            .await
            .expect("ws read timeout")
            .expect("stream ended")
            .expect("ws error");
        let text = match msg {
            WsMessage::Text(t) => t,
            _ => continue,
        };
        let env: Message = Message::decode(&text).unwrap();
        if env.msg_type != MessageType::Notification {
            continue;
        }
        let n: Notification = serde_json::from_value(env.data).unwrap();
        return n.id;
    }
}

async fn wait_for_expired<S>(stream: &mut S) -> ExpiredMessage
where
    S: futures_util::Stream<
            Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), stream.next())
            .await
            .expect("ws read timeout")
            .expect("stream ended")
            .expect("ws error");
        let text = match msg {
            WsMessage::Text(t) => t,
            _ => continue,
        };
        let env: Message = Message::decode(&text).unwrap();
        if env.msg_type == MessageType::Expired {
            return serde_json::from_value(env.data).unwrap();
        }
    }
}

fn bearer(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.header("authorization", format!("Bearer {SECRET}"))
}

#[tokio::test]
async fn post_blocks_until_resolved() {
    let h = spawn().await;
    let echo = spawn_echo().await;
    let (mut sink, mut stream) = connect_ws(&h.base, "client-a").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let post = tokio::spawn({
        let url = h.base.clone();
        let echo = echo.clone();
        async move {
            let client = reqwest::Client::new();
            bearer(client.post(format!("{url}/notify")))
                .json(&json!({
                    "source": "test",
                    "title": "approve?",
                    "wait": 5,
                    "actions": [{"label": "yes", "url": echo}],
                }))
                .send()
                .await
                .unwrap()
        }
    });

    let id = read_notification_id(&mut stream).await;
    let action = Message::encode(
        MessageType::Action,
        &ActionMessage {
            notification_id: id.clone(),
            action_index: 0,
        },
    )
    .unwrap();
    sink.send(WsMessage::Text(action.into())).await.unwrap();

    let resp = post.await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let got: ResolvedMessage = resp.json().await.unwrap();
    assert_eq!(got.notification_id, id);
    assert_eq!(got.action_label, "yes");
    assert!(got.success);
}

#[tokio::test]
async fn post_short_timeout_returns_pending() {
    let h = spawn().await;
    let _ws = connect_ws(&h.base, "client-a").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let resp = bearer(client.post(format!("{}/notify", h.base)))
        .json(&json!({
            "source": "test",
            "title": "hi",
            "wait": 1,
            "maxWait": 10,
            "actions": [{"label": "ok", "url": "http://127.0.0.1:1"}],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);
    let loc = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(loc.ends_with("/wait"), "unexpected Location: {loc}");
    let body: PendingResponse = resp.json().await.unwrap();
    assert_eq!(body.status, "pending");
    assert!(!body.id.is_empty());
}

#[tokio::test]
async fn maxwait_expiry_broadcasts_and_cleans_up() {
    let h = spawn().await;
    let (_sink, mut stream) = connect_ws(&h.base, "client-a").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let resp = bearer(client.post(format!("{}/notify", h.base)))
        .json(&json!({
            "source": "test",
            "title": "expiring",
            "wait": 1,
            "maxWait": 1,
            "actions": [{"label": "ok", "url": "http://127.0.0.1:1"}],
        }))
        .send()
        .await
        .unwrap();
    // With wait == maxWait the POST itself will see the expiry.
    assert!(matches!(
        resp.status(),
        reqwest::StatusCode::GONE | reqwest::StatusCode::ACCEPTED
    ));

    let _ = read_notification_id(&mut stream).await;
    let _ = wait_for_expired(&mut stream).await;

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.state.pending_count().await, 0);
}

#[tokio::test]
async fn long_poll_returns_resolved() {
    let h = spawn().await;
    let echo = spawn_echo().await;
    let (mut sink, mut stream) = connect_ws(&h.base, "client-a").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let resp = bearer(client.post(format!("{}/notify", h.base)))
        .json(&json!({
            "source": "test",
            "title": "poll me",
            "wait": 1,
            "maxWait": 10,
            "actions": [{"label": "yes", "url": echo}],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);
    let pending: PendingResponse = resp.json().await.unwrap();

    let id_for_post = pending.id.clone();
    let poll = tokio::spawn({
        let url = h.base.clone();
        let id = pending.id.clone();
        async move {
            let client = reqwest::Client::new();
            bearer(client.get(format!("{url}/notify/{id}/wait?timeout=5")))
                .send()
                .await
                .unwrap()
        }
    });

    let id = read_notification_id(&mut stream).await;
    assert_eq!(id, id_for_post);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let action = Message::encode(
        MessageType::Action,
        &ActionMessage {
            notification_id: id.clone(),
            action_index: 0,
        },
    )
    .unwrap();
    sink.send(WsMessage::Text(action.into())).await.unwrap();

    let resp = poll.await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let got: ResolvedMessage = resp.json().await.unwrap();
    assert_eq!(got.notification_id, id);
    assert!(got.success);
}

#[tokio::test]
async fn long_poll_unknown_id_returns_gone() {
    let h = spawn().await;
    let client = reqwest::Client::new();
    let resp = bearer(client.get(format!("{}/notify/nope/wait?timeout=1", h.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::GONE);
}

#[tokio::test]
async fn fire_and_forget_is_fast() {
    let h = spawn().await;
    let _ws = connect_ws(&h.base, "client-a").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = bearer(client.post(format!("{}/notify", h.base)))
        .json(&json!({"source": "test", "title": "fyi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);
    assert!(start.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn openapi_json_is_valid() {
    let h = spawn().await;
    let resp = reqwest::get(format!("{}/openapi.json", h.base)).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(v["paths"]["/notify"].is_object());
    assert!(v["paths"]["/notify/{id}/wait"].is_object());
}

#[tokio::test]
async fn missing_auth_is_unauthorized() {
    let h = spawn().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/notify", h.base))
        .json(&json!({"source": "test", "title": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

// Prevent unused-import warnings for the `Action` re-export when the list grows.
#[allow(dead_code)]
fn _uses() -> Action {
    Action::default()
}

// Make sure the WsStatus reference stays live even if axum upgrades its http types.
#[allow(dead_code)]
const _WS_STATUS_SANITY: WsStatus = WsStatus::SWITCHING_PROTOCOLS;
