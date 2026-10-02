//! Tab snoozes: a tab closed "until later" and the wake time it should come back at.
//!
//! The daemon only stores them — it never opens a tab. Each snooze is the client's tab
//! descriptor (opaque JSON) plus a wake time; the web UI polls `GET /api/snoozes`, and
//! when one is due it claims it with `DELETE /api/snoozes/:id` and reopens the tab. The
//! delete is the claim: with two browser windows open, only the one whose delete
//! succeeds reopens it. Persisted to `<state_dir>/snoozes.json` so a snooze outlives a
//! reload and a daemon restart; with no `state_dir` they live in memory only.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Upper bound on stored snoozes — a runaway client can't grow the file without limit.
const MAX_SNOOZES: usize = 500;

/// One snoozed tab. Times are epoch milliseconds (the browser's `Date.now()` clock).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snooze {
    pub id: String,
    /// When the tab should come back.
    pub until: i64,
    /// When it was snoozed (for "snoozed 2h ago" and ordering ties).
    pub snoozed_at: i64,
    /// The client's tab descriptor, stored verbatim (must carry a string `label`).
    pub tab: serde_json::Value,
}

#[derive(Deserialize)]
struct NewSnooze {
    until: i64,
    tab: serde_json::Value,
}

/// The snooze list plus where it persists.
pub struct SnoozeStore {
    path: Option<PathBuf>,
    items: Mutex<Vec<Snooze>>,
    seq: std::sync::atomic::AtomicU64,
}

impl SnoozeStore {
    /// Load from `<state_dir>/snoozes.json` (missing or unreadable → empty).
    pub fn open(state_dir: Option<&Path>) -> Self {
        let path = state_dir.map(|d| d.join("snoozes.json"));
        let items = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Vec<Snooze>>(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            items: Mutex::new(items),
            seq: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Every snooze, soonest wake first.
    pub fn list(&self) -> Vec<Snooze> {
        let mut v = self.items.lock().unwrap().clone();
        v.sort_by_key(|s| (s.until, s.snoozed_at));
        v
    }

    fn add(&self, until: i64, tab: serde_json::Value) -> Result<Snooze, &'static str> {
        let now = chrono::Utc::now().timestamp_millis();
        let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let snooze = Snooze {
            id: format!("{now:x}-{n}"),
            until,
            snoozed_at: now,
            tab,
        };
        let mut items = self.items.lock().unwrap();
        if items.len() >= MAX_SNOOZES {
            return Err("too many snoozes");
        }
        items.push(snooze.clone());
        self.persist(&items);
        Ok(snooze)
    }

    fn remove(&self, id: &str) -> Option<Snooze> {
        let mut items = self.items.lock().unwrap();
        let idx = items.iter().position(|s| s.id == id)?;
        let gone = items.remove(idx);
        self.persist(&items);
        Some(gone)
    }

    /// Best-effort atomic write (tmp + rename); a failed write keeps the in-memory list.
    fn persist(&self, items: &[Snooze]) {
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

/// `GET /api/snoozes` — every snoozed tab, soonest wake first.
pub(crate) async fn snoozes_route(State(state): State<AppState>) -> Response {
    Json(state.snoozes.list()).into_response()
}

/// `POST /api/snoozes` — body `{until, tab}`. Local origin and a JSON content type are
/// required (the content type forces a CORS preflight this daemon never grants).
pub(crate) async fn snooze_create_route(
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
    let Ok(b) = serde_json::from_slice::<NewSnooze>(&body) else {
        return (StatusCode::BAD_REQUEST, "bad snooze body").into_response();
    };
    if b.until <= 0 || !b.tab.get("label").is_some_and(|l| l.is_string()) {
        return (
            StatusCode::BAD_REQUEST,
            "need a positive `until` and a labelled `tab`",
        )
            .into_response();
    }
    match state.snoozes.add(b.until, b.tab) {
        Ok(s) => (StatusCode::CREATED, Json(s)).into_response(),
        Err(e) => (StatusCode::INSUFFICIENT_STORAGE, e).into_response(),
    }
}

/// `DELETE /api/snoozes/:id` — cancel a snooze, or claim a due one for waking. Returns
/// the removed snooze; 404 when it is already gone (another window claimed it).
pub(crate) async fn snooze_delete_route(
    AxumPath(id): AxumPath<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !crate::pty::origin_is_local(&headers) {
        return (StatusCode::FORBIDDEN, "local requests only").into_response();
    }
    match state.snoozes.remove(&id) {
        Some(s) => Json(s).into_response(),
        None => (StatusCode::NOT_FOUND, "no such snooze").into_response(),
    }
}
