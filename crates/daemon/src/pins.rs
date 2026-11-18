//! Pinned sessions: the uuids the rail keeps at its top, oldest pin first.
//!
//! Persisted to `<state_dir>/pins.json` so a pin outlives a reload and a daemon restart;
//! with no `state_dir` they live in memory only. A pin is just a uuid, so it survives the
//! session exiting and comes back with it when it is resumed.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Upper bound on stored pins, so a runaway client can't grow the file without limit.
const MAX_PINS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    /// Lowercased session uuid.
    pub uuid: String,
    /// Epoch milliseconds.
    pub pinned_at: i64,
}

pub struct PinStore {
    path: Option<PathBuf>,
    items: Mutex<Vec<Pin>>,
}

impl PinStore {
    /// Load from `<state_dir>/pins.json` (missing or unreadable → none pinned).
    pub fn open(state_dir: Option<&Path>) -> Self {
        let path = state_dir.map(|d| d.join("pins.json"));
        let items = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Vec<Pin>>(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            items: Mutex::new(items),
        }
    }

    /// Every pin, oldest first.
    pub fn list(&self) -> Vec<Pin> {
        self.lock().clone()
    }

    /// Pin `uuid` (a no-op when already pinned, keeping its place).
    pub fn pin(&self, uuid: &str, now_ms: i64) -> Result<(), &'static str> {
        let uuid = uuid.to_ascii_lowercase();
        let mut items = self.lock();
        if items.iter().any(|p| p.uuid == uuid) {
            return Ok(());
        }
        if items.len() >= MAX_PINS {
            return Err("too many pins");
        }
        items.push(Pin {
            uuid,
            pinned_at: now_ms,
        });
        self.persist(&items);
        Ok(())
    }

    /// Unpin `uuid`; false when it wasn't pinned.
    pub fn unpin(&self, uuid: &str) -> bool {
        let uuid = uuid.to_ascii_lowercase();
        let mut items = self.lock();
        let before = items.len();
        items.retain(|p| p.uuid != uuid);
        let changed = items.len() != before;
        if changed {
            self.persist(&items);
        }
        changed
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Pin>> {
        self.items.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Best-effort atomic write (tmp + rename); a failed write keeps the in-memory list.
    fn persist(&self, items: &[Pin]) {
        let Some(path) = &self.path else { return };
        let Ok(bytes) = serde_json::to_vec_pretty(items) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

#[derive(Deserialize)]
struct NewPin {
    session: String,
}

/// `GET /api/pins` — every pinned session, oldest pin first.
pub(crate) async fn pins_route(State(state): State<AppState>) -> Response {
    Json(state.pins.list()).into_response()
}

/// `POST /api/pins` — body `{"session": "<uuid>"}`. Local origin and a JSON content type
/// are required (the content type forces a CORS preflight this daemon never grants).
/// Returns the full list.
pub(crate) async fn pin_create_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let json_ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    if !crate::pty::origin_is_local(&headers) || !json_ct {
        return (StatusCode::FORBIDDEN, "local JSON requests only").into_response();
    }
    let Some(b) = serde_json::from_slice::<NewPin>(&body)
        .ok()
        .filter(|b| crate::focus::is_uuid(&b.session))
    else {
        return (StatusCode::BAD_REQUEST, "need {\"session\": \"<uuid>\"}").into_response();
    };
    match state
        .pins
        .pin(&b.session, chrono::Utc::now().timestamp_millis())
    {
        Ok(()) => Json(state.pins.list()).into_response(),
        Err(e) => (StatusCode::INSUFFICIENT_STORAGE, e).into_response(),
    }
}

/// `DELETE /api/pins/{uuid}` — unpin; 404 when it wasn't pinned. Returns the full list.
pub(crate) async fn pin_delete_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !crate::pty::origin_is_local(&headers) {
        return (StatusCode::FORBIDDEN, "local requests only").into_response();
    }
    if state.pins.unpin(&uuid) {
        Json(state.pins.list()).into_response()
    } else {
        (StatusCode::NOT_FOUND, "not pinned").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaa1111-0000-4000-8000-000000000001";
    const B: &str = "bbbb2222-0000-4000-8000-000000000002";

    fn uuids(s: &PinStore) -> Vec<String> {
        s.list().into_iter().map(|p| p.uuid).collect()
    }

    #[test]
    fn pins_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let s = PinStore::open(Some(dir.path()));
        s.pin(B, 1).unwrap();
        s.pin(&A.to_uppercase(), 2).unwrap();
        drop(s);

        let s = PinStore::open(Some(dir.path()));
        assert_eq!(uuids(&s), [B, A], "oldest pin first, uuids lowercased");
        assert_eq!(s.list()[0].pinned_at, 1);

        assert!(s.unpin(B));
        drop(s);
        assert_eq!(uuids(&PinStore::open(Some(dir.path()))), [A]);
    }

    #[test]
    fn pinning_twice_keeps_the_first_place() {
        let s = PinStore::open(None);
        s.pin(A, 1).unwrap();
        s.pin(B, 2).unwrap();
        s.pin(A, 3).unwrap();
        assert_eq!(uuids(&s), [A, B]);
        assert_eq!(s.list()[0].pinned_at, 1);
    }

    #[test]
    fn unpinning_what_isnt_pinned_says_so() {
        let s = PinStore::open(None);
        assert!(!s.unpin(A));
    }

    #[test]
    fn a_corrupt_file_reads_as_no_pins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pins.json"), "not json").unwrap();
        let s = PinStore::open(Some(dir.path()));
        assert!(s.list().is_empty());
        s.pin(A, 1).unwrap();
        assert_eq!(uuids(&PinStore::open(Some(dir.path()))), [A]);
    }

    #[test]
    fn the_store_is_bounded() {
        let s = PinStore::open(None);
        for i in 0..MAX_PINS {
            s.pin(&format!("aaaa1111-0000-4000-8000-{i:012}"), 0)
                .unwrap();
        }
        assert!(s.pin(B, 0).is_err());
    }
}
