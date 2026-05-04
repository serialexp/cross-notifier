//! In-process fake APNS server for tests.
//!
//! Real APNS speaks HTTP/2 over TLS; we speak plain HTTP/1.1 here and rely
//! on reqwest negotiating whichever protocol the server offers. That means
//! this mock doesn't exercise our HTTP/2 negotiation path — everything
//! else (URL, headers, JWT, payload, status → outcome mapping) is
//! under test.
//!
//! Gated behind `test-util` so downstream integration tests can drive it;
//! unit tests inside this crate see it unconditionally via `cfg(test)`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::json;
use tokio::net::TcpListener;

/// A single request captured by the mock.
#[derive(Debug, Clone)]
pub struct MockRequest {
    pub token: String,
    pub authorization: Option<String>,
    pub apns_topic: Option<String>,
    pub apns_push_type: Option<String>,
    pub body: serde_json::Value,
}

/// Canned response keyed by exact token match, with a wildcard fallback.
#[derive(Debug, Clone, Default)]
pub struct MockResponses {
    /// Per-token responses. Matches win over the default.
    pub by_token: HashMap<String, (StatusCode, serde_json::Value)>,
    /// Used when no per-token override matches. `None` ⇒ `200 OK {}`.
    pub default: Option<(StatusCode, serde_json::Value)>,
}

#[derive(Clone)]
struct MockState {
    requests: Arc<Mutex<Vec<MockRequest>>>,
    responses: Arc<Mutex<MockResponses>>,
}

/// Handle returned by [`spawn`]. Drop it to stop recording, though the
/// underlying server task will happily outlive it — the axum task exits
/// when the test binary does.
pub struct MockApns {
    /// Base URL to plug into `ApnsConfig::base_url`.
    pub base_url: String,
    requests: Arc<Mutex<Vec<MockRequest>>>,
    responses: Arc<Mutex<MockResponses>>,
}

impl MockApns {
    /// Start a mock on `127.0.0.1` at a kernel-assigned port.
    pub async fn spawn() -> Self {
        let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let responses = Arc::new(Mutex::new(MockResponses::default()));
        let state = MockState {
            requests: requests.clone(),
            responses: responses.clone(),
        };

        let app = Router::new()
            .route("/3/device/{token}", post(handle_push))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock apns");
        let addr: SocketAddr = listener.local_addr().expect("local_addr");

        tokio::spawn(async move {
            // If this panics we don't care — the test is going to fail
            // anyway and its assertion will be more informative than ours.
            let _ = axum::serve(listener, app).await;
        });

        Self {
            base_url: format!("http://{addr}"),
            requests,
            responses,
        }
    }

    /// Snapshot of every request received so far.
    pub fn requests(&self) -> Vec<MockRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Reset both the request log and the response table.
    pub fn reset(&self) {
        self.requests.lock().unwrap().clear();
        *self.responses.lock().unwrap() = MockResponses::default();
    }

    /// Return `(status, body)` for this exact token. Overrides `set_default`.
    pub fn set_response(&self, token: &str, status: StatusCode, reason: &str) {
        self.responses
            .lock()
            .unwrap()
            .by_token
            .insert(token.to_string(), (status, json!({ "reason": reason })));
    }

    /// Fallback response for any token not covered by `set_response`.
    pub fn set_default(&self, status: StatusCode, reason: &str) {
        self.responses.lock().unwrap().default = Some((status, json!({ "reason": reason })));
    }
}

async fn handle_push(
    State(state): State<MockState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let json_body: serde_json::Value = serde_json::from_slice(&body).unwrap_or(json!(null));

    let recorded = MockRequest {
        token: token.clone(),
        authorization: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        apns_topic: headers
            .get("apns-topic")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        apns_push_type: headers
            .get("apns-push-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        body: json_body,
    };
    state.requests.lock().unwrap().push(recorded);

    let (status, body) = {
        let resp = state.responses.lock().unwrap();
        if let Some((s, b)) = resp.by_token.get(&token) {
            (*s, b.clone())
        } else if let Some((s, b)) = resp.default.clone() {
            (s, b)
        } else {
            (StatusCode::OK, json!({}))
        }
    };

    // APNS returns an empty body on 200 in practice; we return `{}` for
    // easier decoding in clients that always try to parse JSON.
    (status, axum::Json(body))
}
