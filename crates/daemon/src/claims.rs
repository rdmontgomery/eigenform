//! `sessions/<pid>.json` claims: the audit behind the active-sessions modal.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;

use crate::AppState;

/// `GET /api/claims` — every `sessions/<pid>.json` claim Claude Code has left, judged
/// against the process table (`alive` / `dead` / `reused`), enriched with the session's
/// title, whether it is headless, and the eigenform pty hosting it (if any). The
/// "what does Claude think is running" audit behind the active-sessions modal.
pub(crate) async fn claims_route(State(state): State<AppState>) -> Response {
    let Some(sessions) = state.config.sessions_dir.clone() else {
        return axum::Json(Vec::<serde_json::Value>::new()).into_response();
    };
    let projects = state.config.projects_dir.clone();
    // pid → eigenform pty id, so the modal can say which claims we host.
    let hosted: HashMap<u32, String> = state
        .host
        .list()
        .iter()
        .map(|live| (live.child_pid(), live.id.to_string()))
        .collect();
    let rows = tokio::task::spawn_blocking(move || {
        eigenform_forest::read_claims(&sessions)
            .into_iter()
            .map(|c| {
                let stub = projects
                    .as_deref()
                    .and_then(|dir| eigenform_forest::resolve_stub(dir, &c.session_id).ok());
                let title = stub
                    .as_ref()
                    .and_then(|st| eigenform_forest::session_ref(st).title);
                let entrypoint = c.entrypoint.clone().or_else(|| {
                    stub.as_ref()
                        .and_then(|st| eigenform_forest::session_entrypoint(&st.path))
                });
                let headless = entrypoint
                    .as_deref()
                    .is_some_and(eigenform_forest::is_headless);
                serde_json::json!({
                    "pid": c.pid,
                    "sessionId": c.session_id,
                    "cwd": c.cwd.map(|p| p.display().to_string()),
                    "startedAt": c.started_at,
                    "kind": c.kind,
                    "entrypoint": entrypoint,
                    "status": c.status,
                    "name": c.name,
                    "title": title,
                    "health": c.health.as_str(),
                    "headless": headless,
                    "ptyId": hosted.get(&c.pid),
                })
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    axum::Json(rows).into_response()
}

/// `DELETE /api/claims/:pid` — retire one claim. A stale claim (dead or reused pid) just
/// has its file removed; a live one gets SIGTERM, and Claude removes its own claim as it
/// exits. The claim is re-read and re-judged here, never trusted from the client, so a
/// pid that was reused between listing and clicking is never signalled.
pub(crate) async fn claim_delete_route(
    AxumPath(pid): AxumPath<u32>,
    State(state): State<AppState>,
) -> Response {
    let Some(sessions) = &state.config.sessions_dir else {
        return (StatusCode::NOT_FOUND, "no sessions dir configured").into_response();
    };
    if pid == 0 || pid == std::process::id() {
        return (StatusCode::BAD_REQUEST, "refusing to signal that pid").into_response();
    }
    let Some(claim) = eigenform_forest::read_claims(sessions)
        .into_iter()
        .find(|c| c.pid == pid)
    else {
        return (StatusCode::NOT_FOUND, "no claim for that pid").into_response();
    };
    let action = match claim.health {
        eigenform_forest::ClaimHealth::Alive => {
            let ok = std::process::Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                return (StatusCode::INTERNAL_SERVER_ERROR, "kill failed").into_response();
            }
            "terminated"
        }
        eigenform_forest::ClaimHealth::Dead | eigenform_forest::ClaimHealth::Reused => {
            if let Err(e) = std::fs::remove_file(&claim.path) {
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
            "cleared"
        }
    };
    axum::Json(serde_json::json!({ "pid": pid, "action": action })).into_response()
}
