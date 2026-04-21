// Local HTTP server for receiving notifications and querying status.
// Listens on :9876 by default. The /notify + /notify/{id}/wait + /ws
// endpoints are delegated to cross-notifier-core so local and remote
// share the same exclusive/wait/long-poll logic; /status and /center/*
// are daemon-specific.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get},
    Json, Router,
};
use cross_notifier_core::{
    CoreState, OutboundMessage, Subscriber, router as core_router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::notification::NotificationPayload;
use crate::protocol::{ExpiredMessage, ResolvedMessage};
use crate::store::SharedStore;

/// Live connection state for a single server, exposed to the UI.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ConnectionState {
    pub connected: bool,
    /// Most recent connection error. Cleared on successful connect.
    /// Retained while disconnected so the UI can show *why*.
    pub last_error: Option<String>,
}

pub type ConnectionMap = Arc<RwLock<HashMap<String, ConnectionState>>>;

#[derive(Clone)]
struct AppState {
    event_proxy: Option<EventLoopProxy<AppEvent>>,
    connections: ConnectionMap,
    store: SharedStore,
}

impl AppState {
    fn send_event(&self, event: AppEvent) {
        if let Some(proxy) = &self.event_proxy {
            let _ = proxy.send_event(event);
        }
    }
}

pub async fn run_server(
    port: u16,
    event_proxy: EventLoopProxy<AppEvent>,
    connections: ConnectionMap,
    store: SharedStore,
) {
    let state = AppState {
        event_proxy: Some(event_proxy.clone()),
        connections,
        store,
    };

    // Core with auth disabled (localhost only) — gives us the same
    // /notify, /notify/{id}/wait, /ws, /health, /openapi.* endpoints
    // the remote server exposes, with identical wait/maxWait behavior.
    let core = CoreState::new("");
    spawn_local_bridge(core.clone(), event_proxy).await;

    let daemon_routes = Router::new()
        .route("/status", get(handle_status))
        .route("/center", get(handle_center_list))
        .route("/center", delete(handle_center_clear))
        .route("/center/{id}", delete(handle_center_dismiss))
        .route("/center/count", get(handle_center_count))
        .with_state(state);

    let app = core_router(core).merge(daemon_routes);

    let addr = format!("0.0.0.0:{}", port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind HTTP server to {}: {}", addr, e);
            return;
        }
    };
    info!("HTTP server listening on {}", addr);

    if let Err(e) = axum::serve(listener, app).await {
        error!("HTTP server error: {}", e);
    }
}

/// Registers an in-process subscriber that forwards core's outbound
/// messages into the winit event loop as the appropriate AppEvent. The
/// daemon then reacts exactly as it would for a WebSocket-delivered
/// notification from a remote server.
async fn spawn_local_bridge(core: CoreState, event_proxy: EventLoopProxy<AppEvent>) {
    let (sub, mut rx) = Subscriber::new("local");
    core.add_subscriber(sub).await;
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let ev = match msg {
                OutboundMessage::Notification(n) => AppEvent::IncomingNotification {
                    server_label: "local".to_string(),
                    payload: core_notification_to_payload(n),
                },
                OutboundMessage::Resolved(r) => {
                    AppEvent::NotificationResolved(ResolvedMessage {
                        notification_id: r.notification_id,
                        resolved_by: r.resolved_by,
                        action_label: r.action_label,
                        success: r.success,
                        error: r.error,
                    })
                }
                OutboundMessage::Expired(e) => {
                    AppEvent::NotificationExpired(ExpiredMessage {
                        notification_id: e.notification_id,
                    })
                }
            };
            if event_proxy.send_event(ev).is_err() {
                break;
            }
        }
    });
}

fn core_notification_to_payload(n: cross_notifier_core::Notification) -> NotificationPayload {
    NotificationPayload {
        id: n.id,
        source: n.source,
        title: n.title,
        message: n.message,
        status: n.status,
        icon_data: n.icon_data,
        icon_href: n.icon_href,
        icon_path: n.icon_path,
        duration: n.duration as i32,
        actions: n
            .actions
            .into_iter()
            .map(|a| crate::notification::Action {
                label: a.label,
                url: a.url,
                method: a.method,
                headers: a.headers,
                body: a.body,
                open: a.open,
            })
            .collect(),
        exclusive: n.exclusive,
        store_on_expire: n.store_on_expire,
    }
}

async fn handle_status(
    State(state): State<AppState>,
) -> Json<HashMap<String, bool>> {
    let connections = state.connections.read().await;
    Json(
        connections
            .iter()
            .map(|(k, v)| (k.clone(), v.connected))
            .collect(),
    )
}

// --- Center endpoints ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CenterNotificationResponse {
    id: i64,
    title: String,
    message: String,
    status: String,
    source: String,
    icon_data: String,
    server_label: String,
    actions: Vec<crate::notification::Action>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn handle_center_list(State(state): State<AppState>) -> Json<Vec<CenterNotificationResponse>> {
    let store = state.store.read().unwrap();
    let items: Vec<_> = store
        .list()
        .iter()
        .map(|n| CenterNotificationResponse {
            id: n.id,
            title: n.payload.title.clone(),
            message: n.payload.message.clone(),
            status: n.payload.status.clone(),
            source: n.payload.source.clone(),
            icon_data: n.payload.icon_data.clone(),
            server_label: n.server_label.clone(),
            actions: n.payload.actions.clone(),
            created_at: n.created_at,
        })
        .collect();
    Json(items)
}

#[derive(Serialize)]
struct CountResponse {
    count: usize,
}

async fn handle_center_count(State(state): State<AppState>) -> Json<CountResponse> {
    let store = state.store.read().unwrap();
    Json(CountResponse {
        count: store.count(),
    })
}

async fn handle_center_dismiss(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> StatusCode {
    let mut store = state.store.write().unwrap();
    if store.remove(id) {
        state.send_event(AppEvent::CenterDirty);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

#[derive(Deserialize)]
struct ClearQuery {
    #[serde(default)]
    confirm: Option<String>,
}

async fn handle_center_clear(
    State(state): State<AppState>,
    Query(query): Query<ClearQuery>,
) -> StatusCode {
    if query.confirm.as_deref() != Some("true") {
        return StatusCode::BAD_REQUEST;
    }

    let mut store = state.store.write().unwrap();
    store.clear();
    state.send_event(AppEvent::CenterDirty);
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn make_test_state() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::NotificationStore::load(dir.path().join("notifications.json"));
        let store = Arc::new(std::sync::RwLock::new(store));

        let state = AppState {
            event_proxy: None, // No event loop in tests
            connections: Arc::new(RwLock::new(HashMap::new())),
            store,
        };
        (state, dir)
    }

    fn make_router(state: AppState) -> Router {
        Router::new()
            .route("/center", get(handle_center_list))
            .route("/center", delete(handle_center_clear))
            .route("/center/{id}", delete(handle_center_dismiss))
            .route("/center/count", get(handle_center_count))
            .with_state(state)
    }

    fn make_test_payload(title: &str, message: &str) -> NotificationPayload {
        NotificationPayload {
            id: String::new(),
            source: String::new(),
            title: title.to_string(),
            message: message.to_string(),
            status: "info".to_string(),
            icon_data: String::new(),
            icon_href: String::new(),
            icon_path: String::new(),
            duration: 0,
            actions: Vec::new(),
            exclusive: false,
            store_on_expire: false,
        }
    }

    #[tokio::test]
    async fn test_center_list_empty() {
        let (state, _dir) = make_test_state();
        let app = make_router(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/center")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let items: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn test_center_list_with_notifications() {
        let (state, _dir) = make_test_state();
        {
            let mut store = state.store.write().unwrap();
            store.add(make_test_payload("Hello", "World"), "test".into());
        }
        let app = make_router(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/center")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let items: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "Hello");
    }

    #[tokio::test]
    async fn test_center_count() {
        let (state, _dir) = make_test_state();
        {
            let mut store = state.store.write().unwrap();
            store.add(make_test_payload("A", "a"), "s".into());
            store.add(make_test_payload("B", "b"), "s".into());
        }
        let app = make_router(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/center/count")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(result["count"], 2);
    }

    #[tokio::test]
    async fn test_center_dismiss_one() {
        let (state, _dir) = make_test_state();
        let id = {
            let mut store = state.store.write().unwrap();
            store.add(make_test_payload("A", "a"), "s".into())
        };
        let app = make_router(state.clone());

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/center/{}", id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.store.read().unwrap().count(), 0);
    }

    #[tokio::test]
    async fn test_center_dismiss_nonexistent() {
        let (state, _dir) = make_test_state();
        let app = make_router(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/center/999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_center_dismiss_all_requires_confirm() {
        let (state, _dir) = make_test_state();
        {
            let mut store = state.store.write().unwrap();
            store.add(make_test_payload("A", "a"), "s".into());
        }
        let app = make_router(state.clone());

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/center")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.store.read().unwrap().count(), 1); // not cleared
    }

    #[tokio::test]
    async fn test_center_dismiss_all() {
        let (state, _dir) = make_test_state();
        {
            let mut store = state.store.write().unwrap();
            store.add(make_test_payload("A", "a"), "s".into());
            store.add(make_test_payload("B", "b"), "s".into());
        }
        let app = make_router(state.clone());

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/center?confirm=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.store.read().unwrap().count(), 0);
    }
}
