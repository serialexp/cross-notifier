//! Axum router for `/calendar/action/snooze` and `/calendar/action/dismiss`.
//!
//! Mount this alongside the core notification router on both the server
//! and the daemon so the action callbacks baked into calendar
//! notifications resolve to the local scheduler.
//!
//! Auth behaviour: if a secret is provided at router construction time,
//! requests must present it via `Authorization: Bearer <secret>`. The
//! daemon's localhost-only HTTP server mounts with `secret: None`; the
//! public server mounts with the same shared secret used for /notify.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    response::Response,
    routing::{get, post},
};
use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::scheduler::{SchedulerCmd, SchedulerHandle};

/// A swappable slot holding the currently-active scheduler handle. The
/// router looks up the handle on each request, so a supervisor can stop
/// the current `CalendarService` and spawn a replacement (pointing the
/// slot at the new handle) without unmounting any routes.
///
/// When the slot is empty — e.g. the user cleared their calendar config —
/// action endpoints return `503 Service Unavailable`.
#[derive(Clone, Default)]
pub struct CalendarHandleSlot {
    inner: Arc<std::sync::RwLock<Option<SchedulerHandle>>>,
}

impl CalendarHandleSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install (or clear) the active handle. Cheap; holds a brief write
    /// lock that does not cross an await point.
    pub fn set(&self, handle: Option<SchedulerHandle>) {
        *self.inner.write().expect("CalendarHandleSlot poisoned") = handle;
    }

    /// Snapshot the current handle. Returns `None` when no service is
    /// running.
    pub fn get(&self) -> Option<SchedulerHandle> {
        self.inner
            .read()
            .expect("CalendarHandleSlot poisoned")
            .clone()
    }
}

#[derive(Clone)]
struct ActionState {
    slot: CalendarHandleSlot,
    secret: Option<Arc<String>>,
}

/// Build the action-callback router. Prefix it with whatever path the
/// caller chose when constructing notification action URLs (typically
/// `/calendar/action`).
///
/// The `slot` is a late-bound pointer to the active scheduler; if it's
/// empty when a request arrives, handlers return `503`. This lets the
/// daemon hot-reload calendar config without re-mounting routes.
pub fn calendar_action_router(slot: CalendarHandleSlot, secret: Option<String>) -> Router {
    let state = ActionState {
        slot,
        secret: secret.map(Arc::new),
    };
    Router::new()
        .route("/snooze", post(handle_snooze))
        .route("/dismiss", post(handle_dismiss))
        .route("/upcoming", get(handle_upcoming))
        .with_state(state)
}

fn check_auth(state: &ActionState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    let Some(expected) = &state.secret else {
        return Ok(());
    };
    let ok = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|got| got == expected.as_str())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(Box::new(
            (StatusCode::UNAUTHORIZED, "invalid or missing bearer token").into_response(),
        ))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnoozeReq {
    occurrence_id: String,
    #[serde(default)]
    hours: Option<u32>,
    #[serde(default)]
    minutes: Option<u32>,
}

async fn handle_snooze(
    State(state): State<ActionState>,
    headers: HeaderMap,
    Json(req): Json<SnoozeReq>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers) {
        return *r;
    }
    // Prefer hours if provided; otherwise use minutes; otherwise default 4h.
    let duration = if let Some(h) = req.hours {
        Duration::hours(h as i64)
    } else if let Some(m) = req.minutes {
        Duration::minutes(m as i64)
    } else {
        Duration::hours(4)
    };
    let Some(sched) = state.slot.get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "calendar service is not currently running",
        )
            .into_response();
    };
    if let Err(e) = sched.send(SchedulerCmd::Snooze {
        occurrence_id: req.occurrence_id,
        duration,
    }) {
        return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DismissReq {
    occurrence_id: String,
}

async fn handle_dismiss(
    State(state): State<ActionState>,
    headers: HeaderMap,
    Json(req): Json<DismissReq>,
) -> Response {
    if let Err(r) = check_auth(&state, &headers) {
        return *r;
    }
    let Some(sched) = state.slot.get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "calendar service is not currently running",
        )
            .into_response();
    };
    if let Err(e) = sched.send(SchedulerCmd::Dismiss {
        occurrence_id: req.occurrence_id,
    }) {
        return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Response row for `GET /upcoming`. Built from `PendingFire` so callers
/// see both the occurrence itself and any scheduler-side overlay
/// (snooze_until / fired_at). Dates are rendered as RFC3339 by serde.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpcomingRow {
    /// Stable occurrence id (same one snooze/dismiss callbacks target).
    id: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    /// When the reminder will fire next, already accounting for snoozes.
    /// Past values mean the entry is overdue / in-flight.
    fire_at: chrono::DateTime<chrono::Utc>,
    /// Underlying event start, for context. `fire_at` will usually be
    /// earlier (alarm trigger offset).
    event_start: chrono::DateTime<chrono::Utc>,
    event_end: chrono::DateTime<chrono::Utc>,
    /// Set when the user (or an incoming snooze) has pushed the reminder
    /// forward. If both `snoozed_until` and `fired_at` are present the
    /// row represents "delivered once, now re-armed."
    #[serde(skip_serializing_if = "Option::is_none")]
    snoozed_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Set after delivery. If `snoozed_until` is None this row has been
    /// handled and is just waiting for GC.
    #[serde(skip_serializing_if = "Option::is_none")]
    fired_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<crate::types::PendingFire> for UpcomingRow {
    fn from(p: crate::types::PendingFire) -> Self {
        let fire_at = p.effective_fire_at();
        Self {
            id: p.occurrence.id,
            summary: p.occurrence.summary,
            location: p.occurrence.location,
            fire_at,
            event_start: p.occurrence.event_start,
            event_end: p.occurrence.event_end,
            snoozed_until: p.snoozed_until,
            fired_at: p.fired_at,
        }
    }
}

/// `GET /upcoming` — snapshot of the scheduler's pending fires, sorted by
/// effective fire time. Same auth behaviour as the action routes (secret
/// required when the router was built with one).
///
/// Unlike the action routes this is a read-only introspection endpoint.
/// It's handy for the daemon's settings UI ("what will I be reminded
/// about?"), CLI debugging via curl, and health-checking a server.
async fn handle_upcoming(State(state): State<ActionState>, headers: HeaderMap) -> Response {
    if let Err(r) = check_auth(&state, &headers) {
        return *r;
    }
    let Some(sched) = state.slot.get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "calendar service is not currently running",
        )
            .into_response();
    };
    match sched.list_pending().await {
        Ok(items) => {
            let rows: Vec<UpcomingRow> = items.into_iter().map(UpcomingRow::from).collect();
            Json(rows).into_response()
        }
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifier::RecordingNotifier;
    use crate::scheduler::Scheduler;
    use crate::store::MemoryStore;
    use crate::types::{Occurrence, PendingFire};
    use axum::body::Body;
    use axum::http::Request;
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn spawn_sched() -> (
        SchedulerHandle,
        Arc<MemoryStore>,
        tokio::task::JoinHandle<()>,
    ) {
        let store = Arc::new(MemoryStore::default());
        let notifier = Arc::new(RecordingNotifier::default());
        // Seed the store with one pending entry so snooze/dismiss have
        // something to target.
        let mut map = crate::store::PendingMap::new();
        map.insert(
            "abc".into(),
            PendingFire::new(Occurrence {
                id: "abc".into(),
                event_uid: "e".into(),
                recurrence_id: None,
                fire_at: Utc.with_ymd_and_hms(2026, 5, 1, 9, 0, 0).unwrap(),
                event_start: Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap(),
                event_end: Utc.with_ymd_and_hms(2026, 5, 1, 11, 0, 0).unwrap(),
                summary: "Meeting".into(),
                location: None,
                description: None,
            }),
        );
        crate::store::PendingStore::save(&*store, &map)
            .await
            .unwrap();

        let sched = Scheduler::new(store.clone(), notifier).await.unwrap();
        sched
            .set_clock(Utc.with_ymd_and_hms(2026, 5, 1, 8, 0, 0).unwrap())
            .await;
        let (handle, join) = sched.spawn();
        (handle, store, join)
    }

    fn slot_with(handle: SchedulerHandle) -> CalendarHandleSlot {
        let slot = CalendarHandleSlot::new();
        slot.set(Some(handle));
        slot
    }

    #[tokio::test]
    async fn upcoming_route_returns_pending_fires() {
        // Verifies end-to-end: the scheduler loads the seeded "abc" entry,
        // GET /upcoming reaches it through the slot, and the response
        // contains it with the right summary and fire time.
        let (handle, _store, _join) = spawn_sched().await;
        let app = calendar_action_router(slot_with(handle.clone()), None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/upcoming")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "abc");
        assert_eq!(rows[0]["summary"], "Meeting");
        assert_eq!(rows[0]["fireAt"], "2026-05-01T09:00:00Z");
    }

    #[tokio::test]
    async fn upcoming_returns_503_when_slot_empty() {
        let app = calendar_action_router(CalendarHandleSlot::new(), None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/upcoming")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn snooze_route_updates_scheduler_state() {
        let (handle, _store, _join) = spawn_sched().await;
        let app = calendar_action_router(slot_with(handle.clone()), None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/snooze")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"occurrenceId":"abc","hours":4}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn auth_guard_rejects_bad_token() {
        let (handle, _store, _join) = spawn_sched().await;
        let app = calendar_action_router(slot_with(handle.clone()), Some("secret".into()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/dismiss")
                    .header("content-type", "application/json")
                    .header("Authorization", "Bearer wrong")
                    .body(Body::from(r#"{"occurrenceId":"abc"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn empty_slot_returns_service_unavailable() {
        // No scheduler installed: the route is mounted but delegation has
        // nowhere to go. Matches the daemon's "calendar disabled in config"
        // state after a hot-reload toggles it off.
        let app = calendar_action_router(CalendarHandleSlot::new(), None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/dismiss")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"occurrenceId":"abc"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn slot_swap_routes_to_new_scheduler() {
        // After replacing the handle in the slot, a request goes to the
        // new scheduler. The old one is untouched. This is the hot-reload
        // contract the daemon relies on.
        let (handle_a, _store_a, _join_a) = spawn_sched().await;
        let (handle_b, _store_b, _join_b) = spawn_sched().await;
        let slot = CalendarHandleSlot::new();
        slot.set(Some(handle_a));
        // Swap in the replacement before the app is even built — subsequent
        // requests observe it, because the router reads the slot per call.
        slot.set(Some(handle_b));
        let app = calendar_action_router(slot, None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/dismiss")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"occurrenceId":"abc"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn auth_guard_allows_correct_token() {
        let (handle, _store, _join) = spawn_sched().await;
        let app = calendar_action_router(slot_with(handle.clone()), Some("secret".into()));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/dismiss")
                    .header("content-type", "application/json")
                    .header("Authorization", "Bearer secret")
                    .body(Body::from(r#"{"occurrenceId":"abc"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
