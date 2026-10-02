//! Session transcript JSON (cached) and edit-then-fork.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use crate::{AppState, Config};

/// `GET /api/session/:uuid/json` — the transcript as structured JSON (exchanges + a
/// trailing leaf) for the drawer, reach map, and forest preview.
pub(crate) async fn session_json_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let cfg = &state.config;
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    let Ok(path) = eigenform_forest::resolve(dir, &uuid) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    // Render once per (file, mtime, len); repeat views and forest-browsing skip the
    // multi-MB read+parse+serialize that dominates the manuscript load latency.
    // NB: the cache key is the parent file only — a subagent still writing its own jsonl
    // while the parent is quiet won't invalidate this cache until the parent changes too.
    match SESSION_CACHE.get_or_render(&path, || {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        let session =
            eigenform_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {});

        let subagents: std::collections::HashMap<String, eigenform_render::ResolvedSubagent> =
            eigenform_forest::enumerate_subagents(&path)
                .into_iter()
                .filter_map(|stub| {
                    let contents = std::fs::read_to_string(&stub.path).ok()?;
                    let sub_session = eigenform_surgery::Session::parse_str(&contents)
                        .unwrap_or_else(|e| match e {});
                    Some((
                        stub.agent_id,
                        eigenform_render::ResolvedSubagent {
                            session: sub_session,
                            agent_type: stub.agent_type,
                            description: stub.description,
                        },
                    ))
                })
                .collect();

        eigenform_render::session_json_with_subagents(&session, &subagents)
    }) {
        Ok(json) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json.to_string(),
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "could not read session").into_response(),
    }
}

/// `POST /api/session/:uuid/fork` — edit-then-fork at a turn. Body `{turn, text}`:
/// re-author the turn `turn` (a turn uuid) with `text`, drop everything after it, and
/// write a NEW session beside the source (copy-on-fork — the source is never touched).
/// Returns `{uuid}` of the new branch, which the client then resumes in the Furnace.
pub(crate) async fn fork_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let cfg = &state.config;
    let Some(turn) = body.get("turn").and_then(|v| v.as_str()) else {
        return (StatusCode::BAD_REQUEST, "missing `turn`").into_response();
    };
    // `text` (the edited prompt) is delivered live into the resumed branch by the client,
    // not written into the file — the fork must end on a completed turn to be resumable.
    match fork_session(cfg, &uuid, turn) {
        Ok(new_uuid) => {
            state.events.record(
                "fork-created",
                serde_json::json!({
                    "srcUuid": uuid,
                    "branchUuid": new_uuid,
                    "turn": turn,
                }),
            );
            Json(serde_json::json!({ "uuid": new_uuid })).into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// Fork `src_uuid` to the completed-turn boundary before `turn`. The new session is written
/// into the SAME project directory as the source (so `claude --resume` and the Forest find
/// it under the project's cwd), never the projects root. Returns the new session uuid.
fn fork_session(
    cfg: &Config,
    src_uuid: &str,
    turn: &str,
) -> Result<String, (StatusCode, &'static str)> {
    let dir = cfg
        .projects_dir
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "no projects dir configured"))?;
    let src_path = eigenform_forest::resolve(dir, src_uuid)
        .map_err(|_| (StatusCode::NOT_FOUND, "no such session"))?;
    let contents = std::fs::read_to_string(&src_path)
        .map_err(|_| (StatusCode::NOT_FOUND, "could not read session"))?;
    let session = eigenform_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {});
    let forked = eigenform_surgery::fork_before(&session, turn).map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "cannot fork before that turn",
        )
    })?;
    let project_dir = src_path.parent().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "session path has no parent",
    ))?;
    eigenform_surgery::write(&forked, project_dir)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "could not write fork"))
}

/// Cache of rendered session JSON, keyed by file path and invalidated by the file's
/// (modified-time, length) stamp. A static transcript is parsed once; the live session
/// (whose file grows each turn) re-renders only when it actually changes.
#[derive(Default)]
struct SessionJsonCache {
    #[allow(clippy::type_complexity)]
    map: Mutex<HashMap<PathBuf, (SystemTime, u64, Arc<str>)>>,
}

impl SessionJsonCache {
    fn get_or_render(
        &self,
        path: &Path,
        render: impl FnOnce() -> String,
    ) -> std::io::Result<Arc<str>> {
        let meta = std::fs::metadata(path)?;
        let stamp = (meta.modified()?, meta.len());
        if let Some((mtime, len, json)) = self.map.lock().unwrap().get(path) {
            if (*mtime, *len) == stamp {
                return Ok(Arc::clone(json));
            }
        }
        let json: Arc<str> = Arc::from(render());
        self.map
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), (stamp.0, stamp.1, Arc::clone(&json)));
        Ok(json)
    }
}

/// Process-wide session-JSON cache (one daemon serves one user; keying by path is fine).
static SESSION_CACHE: LazyLock<SessionJsonCache> = LazyLock::new(SessionJsonCache::default);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_json_cache_renders_once_until_the_file_changes() {
        // The Manuscript re-fetches a session's JSON on every Forest click and SSE tick;
        // re-parsing a multi-MB transcript each time is the load latency. The cache renders
        // once and serves the stored JSON until the file's (mtime, len) stamp changes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, "one").unwrap();

        let cache = SessionJsonCache::default();
        let calls = std::cell::Cell::new(0);
        let render = |tag: &str| {
            calls.set(calls.get() + 1);
            tag.to_string()
        };

        let r1 = cache.get_or_render(&path, || render("JSON-A")).unwrap();
        let r2 = cache.get_or_render(&path, || render("JSON-B")).unwrap();
        assert_eq!(&*r1, "JSON-A");
        assert_eq!(
            &*r2, "JSON-A",
            "second view must be served from cache, not re-rendered"
        );
        assert_eq!(
            calls.get(),
            1,
            "render must run only once for an unchanged file"
        );

        // Mutating the file changes its (mtime, len) stamp → the cache re-renders.
        std::fs::write(&path, "three!").unwrap();
        let r3 = cache.get_or_render(&path, || render("JSON-C")).unwrap();
        assert_eq!(&*r3, "JSON-C");
        assert_eq!(calls.get(), 2, "a changed file must invalidate the cache");
    }

    #[test]
    fn fork_session_writes_a_resumable_branch_beside_the_source_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("-home-me-proj");
        std::fs::create_dir_all(&pdir).unwrap();
        let uuid = "abcdef00-0000-4000-8000-000000000000";
        let src = pdir.join(format!("{uuid}.jsonl"));
        // two complete exchanges: u1→a1→s1, u2→a2→s2
        let jsonl = [
            format!(r#"{{"type":"user","uuid":"u1","cwd":"/home/me/proj","sessionId":"{uuid}","message":{{"role":"user","content":"first prompt"}}}}"#),
            format!(r#"{{"type":"assistant","uuid":"a1","sessionId":"{uuid}","message":{{"role":"assistant","content":[{{"type":"text","text":"reply one"}}]}}}}"#),
            format!(r#"{{"type":"system","uuid":"s1","subtype":"turn_duration","sessionId":"{uuid}"}}"#),
            format!(r#"{{"type":"user","uuid":"u2","cwd":"/home/me/proj","sessionId":"{uuid}","message":{{"role":"user","content":"second prompt"}}}}"#),
            format!(r#"{{"type":"assistant","uuid":"a2","sessionId":"{uuid}","message":{{"role":"assistant","content":[{{"type":"text","text":"reply two"}}]}}}}"#),
            format!(r#"{{"type":"system","uuid":"s2","subtype":"turn_duration","sessionId":"{uuid}"}}"#),
        ]
        .join("\n") + "\n";
        std::fs::write(&src, &jsonl).unwrap();

        let cfg = Config {
            program: "bash".into(),
            projects_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        // fork "before" u2 → rewind to the s1 boundary; u2 and its tail drop.
        let new_uuid = fork_session(&cfg, "abcdef00", "u2").expect("fork ok");
        assert_ne!(new_uuid, uuid, "fork mints a fresh id");

        // the branch lands in the SAME project dir (so resume/Forest find it under the cwd)
        let forked = pdir.join(format!("{new_uuid}.jsonl"));
        let body = std::fs::read_to_string(&forked).expect("fork file written beside source");
        assert!(body.contains("first prompt"), "the kept prefix survives");
        assert!(
            !body.contains("second prompt"),
            "the edited turn is dropped (delivered live)"
        );
        assert!(
            !body.contains("reply two"),
            "the downstream reply is dropped"
        );
        // resumable: the new resume head is the completed-turn system row, not a user turn
        let forked_session =
            eigenform_surgery::Session::parse_str(&body).unwrap_or_else(|e| match e {});
        assert_eq!(forked_session.resume_leaf().as_deref(), Some("s1"));

        // copy-on-fork: the source is byte-for-byte untouched
        assert_eq!(std::fs::read_to_string(&src).unwrap(), jsonl);
    }
}
