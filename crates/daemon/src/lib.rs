//! eigenform-daemon: the session host + http/ws server behind the eigenform app.
//!
//! Hosts pty sessions (a [`host::SessionHost`] that outlives any one browser socket),
//! serves the transcript / forest / claims / inspect APIs, and bridges each pty to a
//! websocket. The bridge drives ANY command; real `claude` is launched only by the user
//! from inside the app, never by the daemon, tests, or the agent.
//!
//! This file is the composition root — [`Config`], [`AppState`], and the router. Each
//! route family lives in its own module:
//!
//! - `pty` — `/pty` websocket (spawn / resume / attach), `/api/pty` list + kill
//! - `session` — `/api/session/:uuid/json` (cached transcript) and `/fork`
//! - `forest` — `/api/forest` snapshot and its `/api/watch/forest` push stream
//! - `claims` — `/api/claims`, the active-sessions audit
//! - `watch` — `/api/watch/:uuid` change pings and the dev live-reload stream
//! - `launcher` — `/api/candidates` and the `/api/path` probe
//! - `inspect` — `/api/inspect`, the skills + memory inventory
//! - `snooze` — `/api/snoozes`, tabs closed until a wake time (persisted)
//! - [`events`] — the observability bus and `/api/events` (+ `/stream`)
//! - [`host`] — the [`host::SessionHost`] pty registry; `paths` — tilde/normalize helpers

use std::path::PathBuf;
use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;

/// The eigenform version: `EIGENFORM_VERSION` when the release workflow stamps it at
/// build time (one per merge to main), else the crate version with a `-dev` suffix for
/// local builds. `/api/health` reports it so the launcher can spot a stale daemon.
pub const VERSION: &str = match option_env!("EIGENFORM_VERSION") {
    Some(v) => v,
    None => concat!(env!("CARGO_PKG_VERSION"), "-dev"),
};

pub mod artifacts;
mod claims;
pub mod events;
mod forest;
pub mod host;
mod inspect;
mod launcher;
mod paths;
pub mod plan_gate;
mod pty;
mod session;
pub mod snooze;
mod watch;

pub use pty::Pty;

/// Webterm assets baked into the binary so an installed `eigenform` is self-contained
/// (no Node, no build, no `--term` flag). Only compiled with `--features embed-assets`,
/// which release/`cargo install` builds turn on after `webterm/dist` is built; a plain
/// `cargo build`/`cargo test` leaves it off, so the daemon needs no built frontend to
/// compile. rust-embed bakes the bytes in release and reads them from disk in debug.
#[cfg(feature = "embed-assets")]
mod embedded {
    use axum::http::{header, StatusCode, Uri};
    use axum::response::{Html, IntoResponse, Response};
    use rust_embed::RustEmbed;

    #[derive(RustEmbed)]
    #[folder = "../../webterm"]
    #[include = "index.html"]
    #[include = "dist/**"]
    #[include = "static/**"]
    struct Assets;

    /// Root fallback: serve an embedded asset by its URL path, else the index (SPA
    /// fallback). API/pty routes are registered before this fallback, so they always win.
    pub async fn serve(uri: Uri) -> Response {
        let rel = uri.path().trim_start_matches('/');
        if !rel.is_empty() {
            if let Some(file) = Assets::get(rel) {
                let mime = mime_guess::from_path(rel).first_or_octet_stream();
                return ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response();
            }
        }
        match Assets::get("index.html") {
            Some(f) => Html(String::from_utf8_lossy(&f.data).into_owned()).into_response(),
            None => (StatusCode::NOT_FOUND, "no embedded index").into_response(),
        }
    }
}

/// What the daemon runs when a terminal connects. For slice 1 this is a fixed command
/// (a shell for the demo, a dummy in tests) — NOT arbitrary exec from the request.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Directory of the eigenform terminal app served at `/` (the front door).
    /// None = serve the embedded build (feature `embed-assets`) or API only.
    pub term_dir: Option<PathBuf>,
    /// `~/.claude/projects` (or a test dir) for session resolution. None = no transcript.
    pub projects_dir: Option<PathBuf>,
    /// `~/.claude/sessions` (or a test dir): `<pid>.json` files for liveness. None = no
    /// live Forest.
    pub sessions_dir: Option<PathBuf>,
    /// `~/.eigenform/state`: persisted per-session metrics (the activity spark). None = no spark.
    pub state_dir: Option<PathBuf>,
    /// `$CODEX_HOME` (default `~/.codex`): Codex CLI threads join the forest as rows, the
    /// drawer renders their rollouts, and `session=<thread id>` resumes them with
    /// `codex resume`. None = Claude sessions only.
    pub codex_home: Option<PathBuf>,
    /// Code root for the new-session launcher (`~/projects` or similar).
    /// `immediate_subdirs` of this path become `recent: false` candidates.
    /// None = no subdirectory suggestions (only recents from projects_dir).
    pub workspace_root: Option<PathBuf>,
    /// Dev mode: inject the live-reload hook and serve `/api/dev/reload`.
    pub dev: bool,
    /// Optional JSONL sink for the structured event stream (`--log-file <path>`).
    /// Each recorded event is appended as one JSON line, best-effort; None = no file.
    pub log_file: Option<PathBuf>,
    /// How long the plan gate holds an `ExitPlanMode` hook awaiting a decision before
    /// answering "no decision" (the terminal prompt takes over). 0 = the default
    /// ([`plan_gate::DEFAULT_HOLD`]).
    pub plan_review_hold_secs: u64,
}

/// Shared router state: pure [`Config`] plus the runtime [`host::SessionHost`]. `Config`
/// holds no runtime state; the host owns the live pty registry the handlers reach for.
/// Both fields are `Arc`, so `Clone` is cheap (axum requires `State: Clone`).
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub host: Arc<host::SessionHost>,
    /// Structured observability event bus (ring buffer + SSE + optional log file).
    pub events: Arc<events::EventBus>,
    /// Plan reviews parked by the `ExitPlanMode` hook, awaiting a human decision.
    pub plan_gate: Arc<plan_gate::PlanGate>,
    /// Snoozed tabs awaiting their wake time (persisted under `state_dir`).
    pub snoozes: Arc<snooze::SnoozeStore>,
}

/// Build the eigenform HTTP/WS router. `GET /pty` upgrades to a websocket bridged to a
/// pty. The eigenform terminal app is the root (`/`): served from `term_dir` when given,
/// otherwise from the embedded build (feature `embed-assets`).
pub fn app(config: Config) -> Router {
    let mut router = Router::new()
        .route("/pty", get(pty::pty_ws))
        .route("/api/pty", get(pty::pty_list_route))
        .route(
            "/api/pty/{id}",
            axum::routing::delete(pty::pty_delete_route),
        )
        .route("/api/session/{uuid}/json", get(session::session_json_route))
        .route("/api/session/{uuid}/fork", post(session::fork_route))
        .route(
            "/api/session/{uuid}/artifacts",
            get(artifacts::artifacts_route),
        )
        .route(
            "/artifact/{uuid}/{*path}",
            get(artifacts::artifact_file_route),
        )
        .route(
            "/api/hooks/plan-review",
            post(plan_gate::plan_review_hook_route),
        )
        .route("/api/plan-reviews", get(plan_gate::plan_reviews_route))
        .route(
            "/api/plan-reviews/{id}/plan",
            get(plan_gate::plan_review_text_route),
        )
        .route(
            "/api/plan-reviews/{id}/decision",
            post(plan_gate::plan_review_decision_route),
        )
        .route("/api/forest", get(forest::forest_route))
        .route("/api/watch/forest", get(forest::forest_watch_route))
        .route("/api/claims", get(claims::claims_route))
        .route(
            "/api/claims/{pid}",
            axum::routing::delete(claims::claim_delete_route),
        )
        .route("/api/inspect", get(inspect::inspect_route))
        .route(
            "/api/snoozes",
            get(snooze::snoozes_route).post(snooze::snooze_create_route),
        )
        .route(
            "/api/snoozes/{id}",
            axum::routing::delete(snooze::snooze_delete_route),
        )
        .route("/api/candidates", get(launcher::candidates_route))
        .route("/api/path", get(launcher::path_probe_route))
        .route("/api/health", get(health_route))
        .route("/api/events", get(events::events_route))
        .route("/api/events/stream", get(events::events_stream_route))
        .route("/api/watch/{uuid}", get(watch::watch_route));

    // eigenform (the terminal app) is the front door at `/`.
    // Dev routes take precedence over the static fallback so the reload hook injects.
    if config.dev && config.term_dir.is_some() {
        router = router
            .route("/", get(watch::dev_index))
            .route("/api/dev/reload", get(watch::dev_reload));
    }
    if let Some(term_dir) = &config.term_dir {
        let index = term_dir.join("index.html");
        router = router.fallback_service(
            tower_http::services::ServeDir::new(term_dir)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    } else {
        // No on-disk build: serve the assets baked into the binary, if present.
        #[cfg(feature = "embed-assets")]
        {
            router = router.fallback(embedded::serve);
        }
    }

    // The event bus is shared between the host (which records pty spawn/exit and
    // uuid adoption) and the route handlers (which record fork + spawn/resume refusals).
    let events = Arc::new(events::EventBus::new(config.log_file.as_deref()));
    let state = AppState {
        host: Arc::new(host::SessionHost::with_events(Arc::clone(&events))),
        snoozes: Arc::new(snooze::SnoozeStore::open(config.state_dir.as_deref())),
        config: Arc::new(config),
        events,
        plan_gate: Arc::new(plan_gate::PlanGate::default()),
    };
    router.with_state(state)
}

/// Bind `addr` and serve the app until the process is killed.
pub async fn serve(addr: std::net::SocketAddr, config: Config) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(config)).await?;
    Ok(())
}

/// `GET /api/health` — liveness + identity marker. The `eigenform` launcher probes this
/// to decide whether a daemon is already up (reuse it) vs. the port being held by some
/// other process, and `eigenform stop` reads `pid` to terminate the running daemon.
async fn health_route() -> Response {
    axum::Json(serde_json::json!({
        "app": "eigenform",
        "version": VERSION,
        "pid": std::process::id(),
    }))
    .into_response()
}
