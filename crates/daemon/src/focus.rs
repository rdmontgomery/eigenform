//! Deep-link focus: `POST /api/focus` asks the open app windows to bring one session to
//! the front. This is the contract Stead's notification click depends on — keep it stable:
//!
//! - body `{"session": "<uuid>"}` or `{"pty": "<id>"}` (exactly one) with
//!   `content-type: application/json`. `pty` takes the id `POST /api/pty` returns, so a
//!   tool that just launched a session can focus it before claude has written a
//!   transcript (and so before it has a uuid); a number is accepted too.
//! - 200: a `focus` event went out over `/api/events/stream` to at least one window
//! - 202: no window was connected; the request is held for [`PENDING_TTL`] and replayed
//!   as the first frame to the next window that connects, marked `"replayed": true`
//!   (the click often opens the app cold, so the POST lands before any window exists)
//! - 400: the body is not JSON, names both or neither, or `session` is not a uuid
//! - 404: the uuid is neither a live pty (`/api/pty`) nor a session the forest knows,
//!   or no live pty has that id
//!
//! The `focus` event (`{"session"}` or `{"pty"}`, as asked) rides the existing bus (so it
//! shows in `/api/events` too); which window acts on it is the app's call (the most
//! recently focused one).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use tokio::sync::broadcast;

use crate::events::{Event, EventBus};
use crate::AppState;

/// How long an undelivered focus request waits for a window to connect.
pub const PENDING_TTL: Duration = Duration::from_secs(30);

/// Counts the windows listening on `/api/events/stream` and holds the one focus request
/// that arrived while none was. Both live under one lock with the bus subscription, so a
/// request and a connecting window can't miss each other: either the request lands first
/// (held, then replayed on connect) or the window subscribes first (it gets the broadcast).
#[derive(Default)]
pub struct FocusHub {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    clients: usize,
    pending: Option<(Event, Instant)>,
}

/// What became of a focus request.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Broadcast to this many connected stream clients.
    Delivered(usize),
    /// Nobody was listening; held for the next client to connect.
    Pending,
}

/// One connected stream client. Dropping it (the SSE stream ended) un-counts it.
pub struct ClientGuard {
    hub: Arc<FocusHub>,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        let mut inner = self.hub.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.clients = inner.clients.saturating_sub(1);
    }
}

impl FocusHub {
    /// Register a stream client: subscribe it to the bus, count it, and hand it the held
    /// focus request if one is still fresh (taken, so only the first client replays it).
    pub fn connect(
        self: &Arc<Self>,
        bus: &EventBus,
    ) -> (broadcast::Receiver<Event>, Option<Event>, ClientGuard) {
        self.connect_at(bus, Instant::now())
    }

    fn connect_at(
        self: &Arc<Self>,
        bus: &EventBus,
        now: Instant,
    ) -> (broadcast::Receiver<Event>, Option<Event>, ClientGuard) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let rx = bus.subscribe();
        inner.clients += 1;
        let replay = inner
            .pending
            .take()
            .filter(|(_, at)| now.duration_since(*at) < PENDING_TTL)
            .map(|(ev, _)| ev);
        let guard = ClientGuard {
            hub: Arc::clone(self),
        };
        (rx, replay, guard)
    }

    /// Record a `focus` event for `target` on the bus. With no client connected, hold it
    /// (replacing any older held request: only the most recent click matters).
    pub fn request(&self, bus: &EventBus, target: &Target) -> Delivery {
        self.request_at(bus, target, Instant::now())
    }

    fn request_at(&self, bus: &EventBus, target: &Target, now: Instant) -> Delivery {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let event = bus.record("focus", target.to_json());
        if inner.clients > 0 {
            inner.pending = None;
            Delivery::Delivered(inner.clients)
        } else {
            inner.pending = Some((event, now));
            Delivery::Pending
        }
    }
}

/// A canonical 8-4-4-4-12 hex uuid (Claude session ids, Codex thread ids).
pub(crate) fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// What a focus request names: a session by uuid, or a live pty by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A lowercased session uuid.
    Session(String),
    Pty(crate::host::PtyId),
}

impl Target {
    /// The event data and the response's echo: `{"session": uuid}` or `{"pty": "<id>"}`
    /// (a string, as `/api/pty` serves ids: JS can't hold every u64).
    fn to_json(&self) -> serde_json::Value {
        match self {
            Target::Session(uuid) => serde_json::json!({ "session": uuid }),
            Target::Pty(id) => serde_json::json!({ "pty": id.to_string() }),
        }
    }
}

#[derive(serde::Deserialize)]
struct FocusBody {
    session: Option<String>,
    pty: Option<serde_json::Value>,
}

/// Validate a body into its target, or the 400 message.
fn parse_target(body: &[u8]) -> Result<Target, &'static str> {
    const NEED: &str = "need {\"session\": \"<uuid>\"} or {\"pty\": \"<id>\"}";
    let Ok(body) = serde_json::from_slice::<FocusBody>(body) else {
        return Err(NEED);
    };
    match (body.session, body.pty) {
        (Some(session), None) if is_uuid(&session) => {
            Ok(Target::Session(session.to_ascii_lowercase()))
        }
        (Some(_), None) => Err("`session` is not a uuid"),
        (None, Some(pty)) => pty
            .as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| pty.as_u64())
            .map(Target::Pty)
            .ok_or("`pty` is not a pty id"),
        _ => Err(NEED),
    }
}

/// `POST /api/focus` — see the module docs for the contract.
pub(crate) async fn focus_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // Same CSRF posture as the other POST routes: a JSON content type can't be sent
    // cross-origin without a preflight, and a browser Origin must be local.
    if !crate::pty::origin_is_local(&headers) {
        return (StatusCode::FORBIDDEN, "local requests only").into_response();
    }
    let json_ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if !json_ct {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "send content-type: application/json",
        )
            .into_response();
    }
    let target = match parse_target(&body) {
        Ok(target) => target,
        Err(why) => return (StatusCode::BAD_REQUEST, why).into_response(),
    };

    let known = match &target {
        Target::Session(uuid) => session_known(&state, uuid).await,
        Target::Pty(id) => state.host.get(*id).is_some(),
    };
    if !known {
        let mut body = target.to_json();
        body["error"] = match target {
            Target::Session(_) => "unknown session",
            Target::Pty(_) => "no live pty with that id",
        }
        .into();
        return (StatusCode::NOT_FOUND, Json(body)).into_response();
    }

    let mut body = target.to_json();
    match state.focus.request(&state.events, &target) {
        Delivery::Delivered(clients) => {
            body["delivered"] = clients.into();
            (StatusCode::OK, Json(body)).into_response()
        }
        Delivery::Pending => {
            body["pending"] = true.into();
            body["ttlSecs"] = PENDING_TTL.as_secs().into();
            (StatusCode::ACCEPTED, Json(body)).into_response()
        }
    }
}

/// True when `uuid` backs a live pty or appears in the forest snapshot. The pty check
/// runs first (cheap, and covers a fresh session the forest hasn't scanned yet).
async fn session_known(state: &AppState, uuid: &str) -> bool {
    if let Some(sessions_dir) = &state.config.sessions_dir {
        state.host.reconcile(sessions_dir);
    }
    if state
        .host
        .list()
        .iter()
        .any(|live| live.uuid().as_deref() == Some(uuid))
    {
        return true;
    }
    let cfg = Arc::clone(&state.config);
    let uuid = uuid.to_string();
    tokio::task::spawn_blocking(move || crate::forest::forest_knows(&cfg, &uuid))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "aaaa1111-0000-4000-8000-000000000001";

    fn session(uuid: &str) -> Target {
        Target::Session(uuid.to_string())
    }

    #[test]
    fn a_body_names_exactly_one_target() {
        let t = |b: &str| parse_target(b.as_bytes());
        assert_eq!(t(&format!("{{\"session\":\"{U}\"}}")), Ok(session(U)));
        assert_eq!(
            t(&format!("{{\"session\":\"{}\"}}", U.to_uppercase())),
            Ok(session(U))
        );
        assert_eq!(t(r#"{"pty":"7"}"#), Ok(Target::Pty(7)));
        assert_eq!(t(r#"{"pty":7}"#), Ok(Target::Pty(7)));
        assert!(t(r#"{"pty":"seven"}"#).is_err());
        assert!(t(r#"{"pty":-1}"#).is_err());
        assert!(t(r#"{"session":"nope"}"#).is_err());
        assert!(t(&format!("{{\"session\":\"{U}\",\"pty\":\"7\"}}")).is_err());
        assert!(t("{}").is_err());
        assert!(t("not json").is_err());
    }

    #[test]
    fn uuid_shape_is_strict() {
        assert!(is_uuid(U));
        assert!(is_uuid("AAAA1111-0000-4000-8000-00000000000F"));
        assert!(!is_uuid(""));
        assert!(!is_uuid("aaaa1111-0000-4000-8000-00000000000"));
        assert!(!is_uuid("aaaa1111-0000-4000-8000-0000000000001"));
        assert!(!is_uuid("aaaa1111000040008000000000000001"));
        assert!(!is_uuid("../../../etc/passwd-0000-4000-8000-000000000001"));
        assert!(!is_uuid("gggg1111-0000-4000-8000-000000000001"));
    }

    #[test]
    fn a_request_with_no_client_is_held_and_replayed_once() {
        let bus = EventBus::default();
        let hub = Arc::new(FocusHub::default());
        let t0 = Instant::now();
        assert_eq!(hub.request_at(&bus, &session(U), t0), Delivery::Pending);

        let (_rx, replay, _g) = hub.connect_at(&bus, t0 + Duration::from_secs(5));
        let ev = replay.expect("first client gets the held request");
        assert_eq!(ev.kind, "focus");
        assert_eq!(ev.data["session"], U);

        let (_rx2, replay2, _g2) = hub.connect_at(&bus, t0 + Duration::from_secs(6));
        assert!(replay2.is_none(), "only the first client replays it");
    }

    #[test]
    fn a_held_request_expires() {
        let bus = EventBus::default();
        let hub = Arc::new(FocusHub::default());
        let t0 = Instant::now();
        hub.request_at(&bus, &session(U), t0);
        let (_rx, replay, _g) = hub.connect_at(&bus, t0 + PENDING_TTL);
        assert!(replay.is_none(), "stale after {PENDING_TTL:?}");
    }

    #[test]
    fn only_the_most_recent_request_is_held() {
        let bus = EventBus::default();
        let hub = Arc::new(FocusHub::default());
        let t0 = Instant::now();
        hub.request_at(&bus, &session("aaaa1111-0000-4000-8000-00000000000a"), t0);
        hub.request_at(&bus, &session(U), t0);
        let (_rx, replay, _g) = hub.connect_at(&bus, t0);
        assert_eq!(replay.unwrap().data["session"], U);
    }

    #[test]
    fn connected_clients_get_the_broadcast_and_are_counted_until_dropped() {
        let bus = EventBus::default();
        let hub = Arc::new(FocusHub::default());
        let (mut rx, replay, guard) = hub.connect(&bus);
        assert!(replay.is_none());
        assert_eq!(hub.request(&bus, &session(U)), Delivery::Delivered(1));
        let ev = rx.try_recv().expect("broadcast reached the subscriber");
        assert_eq!(ev.kind, "focus");

        drop(guard);
        assert_eq!(
            hub.request(&bus, &session(U)),
            Delivery::Pending,
            "no clients left"
        );
    }
}
