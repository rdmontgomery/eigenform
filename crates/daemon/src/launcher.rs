//! New-session launcher support: directory candidates and the path probe.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use std::path::PathBuf;

use crate::paths::{expand_tilde, normalize_path};
use crate::AppState;

/// `GET /api/candidates` — launcher directory list: recent session cwds merged with the
/// immediate subdirs of the configured workspace root. Response: JSON array
/// `[{"path": string, "recent": bool}]`. If neither `workspace_root` nor `projects_dir`
/// is set, returns an empty array. Non-UTF-8 paths are serialized via `display()` (same
/// approach as `/api/pty`'s `cwd` field).
pub(crate) async fn candidates_route(State(state): State<AppState>) -> Response {
    let cfg = &state.config;

    // Recents: deduplicated cwds from recent sessions, in recency order.
    // Dedup delegated to eigenform_projects::unique_cwds (shared with CLI mirror).
    let recents: Vec<PathBuf> = if let Some(dir) = &cfg.projects_dir {
        match eigenform_forest::list(
            dir,
            eigenform_forest::Scope::AllProjects,
            None,
            chrono::Utc::now(),
        ) {
            Ok(sessions) => eigenform_projects::existing_dirs(eigenform_projects::unique_cwds(
                sessions.into_iter().map(|s| s.cwd),
            )),
            Err(_) => vec![],
        }
    } else {
        vec![]
    };

    // Subdirs: immediate children of the workspace root. Missing root → empty (not a 500).
    let subdirs: Vec<PathBuf> = if let Some(root) = &cfg.workspace_root {
        eigenform_projects::immediate_subdirs(root).unwrap_or_default()
    } else {
        vec![]
    };

    // Short-circuit: nothing configured → empty array.
    if recents.is_empty() && subdirs.is_empty() {
        return axum::Json(serde_json::json!([])).into_response();
    }

    let candidates = eigenform_projects::merge_candidates(&recents, &subdirs);
    let items: Vec<serde_json::Value> = candidates
        .iter()
        .map(|c| {
            serde_json::json!({
                "path": c.path.display().to_string(),
                "recent": c.recent,
            })
        })
        .collect();
    axum::Json(items).into_response()
}

#[derive(serde::Deserialize)]
pub(crate) struct PathProbeQuery {
    path: String,
}

/// `GET /api/path?path=<abs>` — does this path exist, and is it a directory?
/// Response: `{"exists": bool, "isDir": bool}`. The launcher uses this to decide
/// whether opening a typed path means "attach to an existing dir" (no prompt) or
/// "make a new one" (confirm first). Stat-only, no traversal; the daemon is
/// localhost-bound, so this leaks nothing a local shell couldn't already see.
pub(crate) async fn path_probe_route(
    axum::extract::Query(query): axum::extract::Query<PathProbeQuery>,
) -> Response {
    let p = normalize_path(&expand_tilde(&query.path));
    let meta = std::fs::metadata(&p);
    let exists = meta.is_ok();
    let is_dir = meta.map(|m| m.is_dir()).unwrap_or(false);
    axum::Json(serde_json::json!({ "exists": exists, "isDir": is_dir })).into_response()
}
