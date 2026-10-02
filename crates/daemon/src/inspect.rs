//! `GET /api/inspect`: the skills + memory config inventory.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::path::Path;

use crate::AppState;

#[derive(serde::Deserialize)]
pub(crate) struct InspectQuery {
    /// Resolution context (an absolute cwd). Present → single-context inventory
    /// with shadowing. Absent (or `all=true`) → machine-wide inventory.
    cwd: Option<String>,
    /// Force the machine-wide inventory even if a `cwd` is supplied.
    all: Option<bool>,
}

/// `GET /api/inspect` — the unified config inventory (skills + memory across
/// resolution layers, every entry token-budgeted) as JSON, the same shape
/// `eigenform inspect --render json` prints. `?cwd=<abs>` gives one resolution
/// context (shadowing computed); otherwise (or `?all=true`) a flat inventory of
/// every recorded project. This is the browser surface for skills/memory that
/// previously lived only as a CLI stdout dump.
pub(crate) async fn inspect_route(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<InspectQuery>,
) -> Response {
    let cfg = &state.config;
    let Some(projects_dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    // projects_dir is `<home>/.claude/projects`; recover home for the skill stack.
    let Some(home) = projects_dir.parent().and_then(|p| p.parent()) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "cannot derive home from projects dir",
        )
            .into_response();
    };

    let all = query.all.unwrap_or(false);
    let data = match query.cwd.filter(|_| !all) {
        Some(cwd) => eigenform_inspect::collect(home, Path::new(&cwd)),
        None => eigenform_inspect::collect_all_projects(home),
    };
    match data {
        Ok(data) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            eigenform_render::inspect_json(&data),
        )
            .into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "inspect failed").into_response(),
    }
}
