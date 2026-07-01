//! eigenform-daemon: the woland backend — pty manager + http/ws server.
//!
//! Slice 1: the pty bridge. Spawn an arbitrary command in a pty and stream its stdio.
//! The bridge drives ANY command; real `claude --resume` is launched only by the user,
//! never by tests or the agent. See `docs/plans/2026-06-03-woland-design.md`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;

pub mod host;

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
#[derive(Clone, Debug)]
pub struct Config {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Directory of the legacy woland workbench, served (paused, dev-only) at `/woland`.
    /// None = not mounted.
    pub web_dir: Option<PathBuf>,
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
    /// Code root for the new-session launcher (`~/projects` or similar).
    /// `immediate_subdirs` of this path become `recent: false` candidates.
    /// None = no subdirectory suggestions (only recents from projects_dir).
    pub workspace_root: Option<PathBuf>,
    /// Dev mode: inject the live-reload hook and serve `/api/dev/reload`.
    pub dev: bool,
}

/// Shared router state: pure [`Config`] plus the runtime [`host::SessionHost`]. `Config`
/// holds no runtime state; the host owns the live pty registry the handlers reach for.
/// Both fields are `Arc`, so `Clone` is cheap (axum requires `State: Clone`).
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub host: Arc<host::SessionHost>,
}

/// Build the eigenform HTTP/WS router. `GET /pty` upgrades to a websocket bridged to a
/// pty. The eigenform terminal app is the root (`/`): served from `term_dir` when given,
/// otherwise from the embedded build (feature `embed-assets`). The legacy woland
/// workbench, when `web_dir` is set, mounts at `/woland` (paused, dev-only).
pub fn app(config: Config) -> Router {
    let mut router = Router::new()
        .route("/pty", get(pty_ws))
        .route("/api/pty", get(pty_list_route))
        .route("/api/pty/:id", axum::routing::delete(pty_delete_route))
        .route("/session/:uuid", get(session_route))
        .route("/api/session/:uuid", get(session_fragment_route))
        .route("/api/session/:uuid/json", get(session_json_route))
        .route("/api/session/:uuid/fork", post(fork_route))
        .route("/api/sessions", get(sessions_route))
        .route("/api/forest", get(forest_route))
        .route("/api/watch/forest", get(forest_watch_route))
        .route("/api/projects", get(projects_route))
        .route("/api/recent", get(recent_route))
        .route("/api/candidates", get(candidates_route))
        .route("/api/path", get(path_probe_route))
        .route("/api/health", get(health_route))
        .route("/api/watch/:uuid", get(watch_route));

    // Legacy woland, paused: mounts at /woland only when a build dir is given.
    if let Some(web_dir) = &config.web_dir {
        let index = web_dir.join("index.html");
        router = router.nest_service(
            "/woland",
            tower_http::services::ServeDir::new(web_dir)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    }

    // eigenform (the terminal app) is the front door at `/`.
    // Dev routes take precedence over the static fallback so the reload hook injects.
    if config.dev && config.term_dir.is_some() {
        router = router
            .route("/", get(dev_index))
            .route("/api/dev/reload", get(dev_reload));
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

    let state = AppState {
        config: Arc::new(config),
        host: Arc::new(host::SessionHost::default()),
    };
    router.with_state(state)
}

/// Bind `addr` and serve woland until the process is killed.
pub async fn serve(addr: std::net::SocketAddr, config: Config) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(config)).await?;
    Ok(())
}

/// `GET /session/:uuid` — the semantic transcript as a standalone HTML page.
async fn session_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let cfg = &state.config;
    match session_fragment(cfg, &uuid) {
        Ok(frag) => Html(transcript_page(&frag)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/session/:uuid` — just the transcript fragment, for in-page injection into
/// the Manuscript (no page chrome).
async fn session_fragment_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let cfg = &state.config;
    match session_fragment(cfg, &uuid) {
        Ok(frag) => Html(frag).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/session/:uuid/json` — the transcript as structured JSON for the Manuscript
/// (exchanges + a trailing leaf), so woland can fold/annotate per turn rather than inject
/// opaque HTML.
async fn session_json_route(
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
    match SESSION_CACHE.get_or_render(&path, || {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        let session = eigenform_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {});
        eigenform_render::session_json(&session)
    }) {
        Ok(json) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json.to_string(),
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "could not read session").into_response(),
    }
}

/// Resolve, read, parse, and render a session's transcript fragment.
fn session_fragment(cfg: &Config, uuid: &str) -> Result<String, (StatusCode, &'static str)> {
    Ok(eigenform_render::session_html(&load_session(cfg, uuid)?))
}

/// `POST /api/session/:uuid/fork` — edit-then-fork at a turn. Body `{turn, text}`:
/// re-author the turn `turn` (a turn uuid) with `text`, drop everything after it, and
/// write a NEW session beside the source (copy-on-fork — the source is never touched).
/// Returns `{uuid}` of the new branch, which the client then resumes in the Furnace.
async fn fork_route(
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
        Ok(new_uuid) => Json(serde_json::json!({ "uuid": new_uuid })).into_response(),
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
    let src_path =
        eigenform_forest::resolve(dir, src_uuid).map_err(|_| (StatusCode::NOT_FOUND, "no such session"))?;
    let contents = std::fs::read_to_string(&src_path)
        .map_err(|_| (StatusCode::NOT_FOUND, "could not read session"))?;
    let session = eigenform_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {});
    let forked = eigenform_surgery::fork_before(&session, turn)
        .map_err(|_| (StatusCode::UNPROCESSABLE_ENTITY, "cannot fork before that turn"))?;
    let project_dir = src_path
        .parent()
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "session path has no parent"))?;
    eigenform_surgery::write(&forked, project_dir)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "could not write fork"))
}

/// Resolve, read, and parse a session's JSONL into a [`Session`].
fn load_session(cfg: &Config, uuid: &str) -> Result<eigenform_surgery::Session, (StatusCode, &'static str)> {
    let dir = cfg
        .projects_dir
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "no projects dir configured"))?;
    let path = eigenform_forest::resolve(dir, uuid).map_err(|_| (StatusCode::NOT_FOUND, "no such session"))?;
    let contents =
        std::fs::read_to_string(&path).map_err(|_| (StatusCode::NOT_FOUND, "could not read session"))?;
    // parse_str is currently infallible (ParseError is uninhabited).
    Ok(eigenform_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {}))
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

/// `GET /api/sessions` — recent sessions across all projects, for the sidebar.
async fn sessions_route(State(state): State<AppState>) -> Response {
    let cfg = &state.config;
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    match eigenform_forest::list(dir, eigenform_forest::Scope::AllProjects, None, chrono::Utc::now()) {
        Ok(sessions) => {
            let items: Vec<_> = sessions
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "uuid": s.uuid,
                        "title": s.title.clone().unwrap_or_else(|| "(untitled)".to_string()),
                        "cwd": s.cwd.display().to_string(),
                        "recency": s.recency.to_rfc3339(),
                    })
                })
                .collect();
            axum::Json(items).into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response(),
    }
}

/// `GET /api/projects` — distinct project cwds (recent-first), for the new-session
/// directory datalist.
async fn projects_route(State(state): State<AppState>) -> Response {
    let cfg = &state.config;
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    match eigenform_forest::list(dir, eigenform_forest::Scope::AllProjects, None, chrono::Utc::now()) {
        Ok(sessions) => {
            let mut seen = std::collections::HashSet::new();
            let cwds: Vec<String> = sessions
                .iter()
                .map(|s| s.cwd.display().to_string())
                .filter(|c| seen.insert(c.clone()))
                .collect();
            axum::Json(cwds).into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response(),
    }
}

/// `GET /api/forest` — the corroborated live-Forest snapshot (liveness × JSONL state ×
/// activity spark). Mirrors what `eigenform forest --live` prints.
async fn forest_route(State(state): State<AppState>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        forest_json(&state.config),
    )
        .into_response()
}

/// `GET /api/watch/forest` — SSE that pushes the snapshot whenever it changes.
async fn forest_watch_route(State(state): State<AppState>) -> Response {
    forest_sse(state.config)
}

/// Compute the live-Forest snapshot as a JSON string. Empty array if the dirs aren't set.
fn forest_json(cfg: &Config) -> String {
    let (Some(projects), Some(sessions), Some(state)) =
        (&cfg.projects_dir, &cfg.sessions_dir, &cfg.state_dir)
    else {
        return "[]".to_string();
    };
    let rows: Vec<serde_json::Value> =
        eigenform_forest::live_forest(projects, sessions, state, chrono::Utc::now())
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "uuid": s.uuid,
                    "title": s.title,
                    "cwd": s.cwd.display().to_string(),
                    "recency": s.recency.to_rfc3339(),
                    "live": s.live,
                    "state": s.state.as_str(),
                    "spark": s.spark,
                })
            })
            .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// SSE pushing the live-Forest snapshot on change. Triggers: filesystem events on the
/// sessions + projects dirs (snappy: activity, new sessions) ∪ a coarse 3s tick (catches
/// pid exits, which aren't filesystem events). Emits only when the snapshot's hash changes,
/// so the tick is silent when nothing moved. The payload travels in the event (no refetch).
fn forest_sse(cfg: Arc<Config>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
    tokio::spawn(async move {
        // A dedicated thread owns the notify watcher; it pings on any write under the
        // watched dirs. Lives until the event channel closes (SSE gone).
        let (evt_tx, mut evt_rx) = tokio::sync::mpsc::channel::<()>(8);
        let watch_dirs: Vec<PathBuf> = [cfg.sessions_dir.clone(), cfg.projects_dir.clone()]
            .into_iter()
            .flatten()
            .collect();
        std::thread::spawn(move || {
            let (raw_tx, raw_rx) = std::sync::mpsc::channel();
            let Ok(mut watcher) = notify::recommended_watcher(move |res| {
                let _ = raw_tx.send(res);
            }) else {
                return;
            };
            for d in &watch_dirs {
                let _ = notify::Watcher::watch(&mut watcher, d, notify::RecursiveMode::Recursive);
            }
            for _event in raw_rx {
                if evt_tx.blocking_send(()).is_err() {
                    break; // SSE gone; drop the watcher
                }
            }
        });

        let mut last_hash: u64 = 0;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3));
        loop {
            let cfg2 = Arc::clone(&cfg);
            let json = tokio::task::spawn_blocking(move || forest_json(&cfg2))
                .await
                .unwrap_or_else(|_| "[]".to_string());
            let h = hash_str(&json);
            if h != last_hash {
                last_hash = h;
                if tx.send(json).await.is_err() {
                    break; // client gone
                }
            }
            tokio::select! {
                _ = tick.tick() => {}
                r = evt_rx.recv() => { if r.is_none() { break; } }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|json| Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data(json)));
    // Keep-alive comments force a periodic write so a disconnected client is detected
    // (the write fails) and the stream + connection are dropped promptly. Without it, a
    // dead SSE connection to a quiet endpoint lingers until the next real event — which
    // may never come — leaking ESTABLISHED sockets against the browser's per-origin cap.
    axum::response::sse::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(10)),
        )
        .into_response()
}

/// `GET /api/recent` — the most recent session uuid across all projects.
async fn recent_route(State(state): State<AppState>) -> Response {
    let cfg = &state.config;
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    match eigenform_forest::list(dir, eigenform_forest::Scope::AllProjects, None, chrono::Utc::now()) {
        Ok(mut sessions) => match sessions.drain(..).next() {
            Some(s) => s.uuid.into_response(),
            None => (StatusCode::NOT_FOUND, "no sessions").into_response(),
        },
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response(),
    }
}

/// `GET /api/candidates` — launcher directory list: recent session cwds merged with the
/// immediate subdirs of the configured workspace root. Response: JSON array
/// `[{"path": string, "recent": bool}]`. If neither `workspace_root` nor `projects_dir`
/// is set, returns an empty array. Non-UTF-8 paths are serialized via `display()` (same
/// approach as `/api/pty`'s `cwd` field).
async fn candidates_route(State(state): State<AppState>) -> Response {
    let cfg = &state.config;

    // Recents: deduplicated cwds from recent sessions, in recency order.
    // Dedup delegated to eigenform_projects::unique_cwds (shared with CLI mirror).
    let recents: Vec<PathBuf> = if let Some(dir) = &cfg.projects_dir {
        match eigenform_forest::list(dir, eigenform_forest::Scope::AllProjects, None, chrono::Utc::now()) {
            Ok(sessions) => {
                eigenform_projects::unique_cwds(sessions.into_iter().map(|s| s.cwd))
            }
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

/// `GET /api/health` — liveness + identity marker. The `eigenform` launcher probes this
/// to decide whether a daemon is already up (reuse it) vs. the port being held by some
/// other process, and `eigenform stop` reads `pid` to terminate the running daemon.
async fn health_route() -> Response {
    axum::Json(serde_json::json!({
        "app": "eigenform",
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct PathProbeQuery {
    path: String,
}

/// `GET /api/path?path=<abs>` — does this path exist, and is it a directory?
/// Response: `{"exists": bool, "isDir": bool}`. The launcher uses this to decide
/// whether opening a typed path means "attach to an existing dir" (no prompt) or
/// "make a new one" (confirm first). Stat-only, no traversal; the daemon is
/// localhost-bound, so this leaks nothing a local shell couldn't already see.
async fn path_probe_route(
    axum::extract::Query(query): axum::extract::Query<PathProbeQuery>,
) -> Response {
    let p = normalize_path(&expand_tilde(&query.path));
    let meta = std::fs::metadata(&p);
    let exists = meta.is_ok();
    let is_dir = meta.map(|m| m.is_dir()).unwrap_or(false);
    axum::Json(serde_json::json!({ "exists": exists, "isDir": is_dir })).into_response()
}

/// `GET /api/watch/:uuid` — Server-Sent Events: a `change` event each time the session's
/// JSONL is written (the live-follow signal for the right pane).
async fn watch_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let cfg = &state.config;
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    let path = match eigenform_forest::resolve(dir, &uuid) {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "no such session").into_response(),
    };
    let watch_dir = path.parent().unwrap_or(&path).to_path_buf();
    let target = path.file_name().map(|n| n.to_os_string());
    watch_sse(watch_dir, target)
}

/// `GET /` in dev mode: the eigenform index with a live-reload hook injected.
async fn dev_index(State(state): State<AppState>) -> Response {
    let cfg = &state.config;
    let Some(term_dir) = &cfg.term_dir else {
        return (StatusCode::NOT_FOUND, "no term dir").into_response();
    };
    let Ok(html) = std::fs::read_to_string(term_dir.join("index.html")) else {
        return (StatusCode::NOT_FOUND, "no index.html").into_response();
    };
    let injected = html.replacen(
        "<head>",
        "<head>\n    <meta name=\"eigenform-dev\" content=\"1\" />",
        1,
    );
    Html(injected).into_response()
}

/// `GET /api/dev/reload` — SSE that fires whenever the built frontend bundle changes.
async fn dev_reload(State(state): State<AppState>) -> Response {
    let cfg = &state.config;
    let Some(term_dir) = &cfg.term_dir else {
        return (StatusCode::NOT_FOUND, "no term dir").into_response();
    };
    watch_sse(term_dir.join("dist"), None)
}

/// Spawn a filesystem watcher on `watch_dir`, returning a receiver that yields `()` each time a
/// file (matching `target`, if set) is written, plus the watcher thread's handle.
///
/// The thread exits promptly when the receiver is dropped: it polls with a 1s timeout and checks
/// whether the consumer is gone (`tx.is_closed()`) on every tick. This matters because the old
/// implementation only noticed a disconnected client *after the next filesystem event* — so an
/// EventSource that reconnected while the session's directory was quiet stranded its thread +
/// inotify instance indefinitely. Those leaked watchers accumulated toward the per-user inotify
/// cap (e.g. 128); once near it, `recommended_watcher()` starts failing and new SSE subscriptions
/// stream nothing forever, silently freezing the reach map and transcript.
fn watch_channel(
    watch_dir: PathBuf,
    target: Option<std::ffi::OsString>,
) -> (tokio::sync::mpsc::Receiver<()>, std::thread::JoinHandle<()>) {
    let (tx, rx) = tokio::sync::mpsc::channel::<()>(8);
    let handle = std::thread::spawn(move || {
        let (raw_tx, raw_rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        }) {
            Ok(w) => w,
            Err(_) => return,
        };
        if notify::Watcher::watch(&mut watcher, &watch_dir, notify::RecursiveMode::NonRecursive)
            .is_err()
        {
            return;
        }
        loop {
            match raw_rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(Ok(event)) => {
                    let touches = match &target {
                        Some(name) => {
                            event.paths.iter().any(|p| p.file_name() == Some(name.as_os_str()))
                        }
                        None => true,
                    };
                    if touches && tx.blocking_send(()).is_err() {
                        break; // SSE connection gone; drop the watcher
                    }
                }
                // A watcher-level error: ignore this one and keep watching.
                Ok(Err(_)) => {}
                // No fs events this tick — still check whether the client left, so a
                // disconnected consumer is reaped within ~1s instead of leaking until the
                // next (possibly never) filesystem event.
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if tx.is_closed() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (rx, handle)
}

/// SSE that emits a `change` event when files in `watch_dir` are written. If `target` is
/// set, only that filename triggers; otherwise any change in the dir does. Backed by
/// [`watch_channel`], whose thread self-terminates when this stream (and thus the receiver)
/// is dropped — including when the client disconnects while the directory is quiet.
fn watch_sse(watch_dir: PathBuf, target: Option<std::ffi::OsString>) -> Response {
    let (rx, _handle) = watch_channel(watch_dir, target);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|_| Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data("change")));
    // See the forest watch above: keep-alive comments let the daemon notice and reap a
    // disconnected client within the interval instead of leaking the connection until the
    // session's next filesystem write (which, for an idle session, may never arrive).
    axum::response::sse::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(10)),
        )
        .into_response()
}

/// Wrap a transcript fragment in a standalone dark page with collapsible styling.
fn transcript_page(fragment: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>transcript</title>\
         <style>{TRANSCRIPT_CSS}</style></head><body>{fragment}</body></html>"
    )
}

const TRANSCRIPT_CSS: &str = "\
body{margin:0;padding:12px;background:#0b0b0e;color:#e6e6e6;\
font-family:ui-monospace,Menlo,Consolas,monospace;font-size:13px;line-height:1.5}\
.session header{color:#8a8a99;margin-bottom:10px}\
details.exchange{margin:0 0 6px;border-left:2px solid #23232b;padding-left:8px}\
summary{cursor:pointer;list-style:none}summary::-webkit-details-marker{display:none}\
.reply{margin:4px 0 4px 16px;white-space:pre-wrap;word-break:break-word}\
.glyph{display:inline-block;width:1em}.glyph.user{color:#7aa2f7}\
.glyph.assistant{color:#9ece6a}.glyph.system{color:#565666}\
.role{color:#565666;margin-right:6px}.content{white-space:pre-wrap}\
.leaf{color:#e0af68}";

#[derive(serde::Deserialize)]
struct PtyQuery {
    /// Re-attach to an already-registered pty by id; spawns nothing. Highest precedence.
    attach: Option<host::PtyId>,
    /// Resume this session in the pty (spawns `claude --resume`).
    session: Option<String>,
    /// Start a fresh session: spawn `claude` in this cwd. Takes precedence over `session`.
    new: Option<String>,
    /// When `new` is set, `create=1` tells the daemon to `fs::create_dir_all` the cwd
    /// before spawning. Only allowed when the path is under `config.workspace_root`;
    /// outside paths close the socket with a POLICY frame.
    ///
    /// Wire form: `&create=1` (non-zero = true, `0` or absent = false).
    /// Parsed as `Option<u8>` so the query-string value `"1"` deserialises cleanly
    /// without a custom deserialiser (axum uses serde's query deserialiser; booleans
    /// from query strings require a custom handler because `"true"` ≠ `true`).
    #[serde(default)]
    create: u8,
    // Absent all = the default command (a shell). Only a real connection spawns anything.
}

async fn pty_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<PtyQuery>,
    headers: HeaderMap,
) -> Response {
    // Defend against CSRF-to-localhost: a page you visit must not be able to open a
    // shell on this daemon. Browsers always send Origin; reject any that isn't local.
    // A missing Origin means a non-browser client (curl, our tests) — allowed.
    if !origin_is_local(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin websocket rejected").into_response();
    }

    // Re-attach: no spawn. Resolve the live pty before upgrading so a missing id can
    // close the socket with a clear reason rather than spawn anything.
    if let Some(id) = query.attach {
        let host = Arc::clone(&state.host);
        return ws.on_upgrade(move |socket| async move {
            match host.get(id) {
                Some(live) => attach_socket(socket, live).await,
                None => {
                    let _ = close_with_reason(socket, "no live pty with that id").await;
                }
            }
        });
    }

    // Otherwise spawn-and-register through the host (uniform model: even bare `/pty`
    // registers a pty that outlives this socket).

    // `new=<cwd>` directory policy (full-freedom + confirm; the launcher gates `create=1`
    // behind a user "Create <path>?" prompt, and `origin_is_local` above is the CSRF guard).
    //   - create=1 + missing → mkdir_all it (anywhere), then spawn.
    //   - create=0 + missing → refuse: don't spawn claude in a cwd that isn't there.
    //   - existing dir        → spawn as-is, no mkdir, regardless of the create flag.
    // Resolved here (before on_upgrade) so a rejection closes the socket with a clear reason.
    if let Some(cwd_str) = query.new.as_deref() {
        let dir = normalize_path(&expand_tilde(cwd_str));
        let exists = dir.is_dir();
        if !exists {
            if query.create != 0 {
                if std::fs::create_dir_all(&dir).is_err() {
                    return ws.on_upgrade(move |socket| async move {
                        let _ = close_with_reason(socket, "failed to create directory").await;
                    });
                }
            } else {
                return ws.on_upgrade(move |socket| async move {
                    let _ = close_with_reason(socket, "no such directory").await;
                });
            }
        }
    }

    let command = pty_command(&state.config, &query);

    // Resume guard, mirroring the `new=` "no such directory" policy above: a session
    // records the cwd it was born in. If the user renamed/moved/deleted that project, the
    // recorded cwd is gone — spawning `claude` there silently lands in `$HOME` and
    // `--resume` then can't find the session. Refuse up front with a clear reason instead.
    if query.session.is_some() && resume_cwd_missing(&command) {
        return ws.on_upgrade(move |socket| async move {
            let _ = close_with_reason(socket, "session's project directory no longer exists").await;
        });
    }

    let host = Arc::clone(&state.host);
    ws.on_upgrade(move |socket| async move {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        let live = match host.spawn(&command.program, &args, command.cwd.as_deref(), (80, 24)) {
            Ok(live) => live,
            Err(_) => {
                let _ = close_with_reason(socket, "failed to spawn pty").await;
                return;
            }
        };

        // For a fresh session: watch for its new JSONL, then record the uuid on the
        // LivePty and broadcast it to attached clients. The watcher holds a `Weak`
        // (mirroring the pump) so an abandoned connection can't keep the pty alive.
        if let Some((projects, dir_name)) = command.watch.clone() {
            let weak = Arc::downgrade(&live);
            std::thread::spawn(move || {
                if let Some(uuid) = watch_new_session(projects, dir_name, weak.clone()) {
                    if let Some(live) = weak.upgrade() {
                        live.set_uuid(uuid.clone());
                        live.broadcast_text(
                            serde_json::json!({"type": "session", "uuid": uuid}).to_string(),
                        );
                    }
                }
            });
        }

        attach_socket(socket, live).await;
    })
}

/// Close a websocket with a human-readable reason (used when an attach target is gone
/// or a spawn fails). Best-effort: a send failure means the client already left.
async fn close_with_reason(socket: WebSocket, reason: &'static str) -> Result<(), axum::Error> {
    use axum::extract::ws::{close_code, CloseFrame};
    let mut socket = socket;
    socket
        .send(Message::Close(Some(CloseFrame {
            code: close_code::POLICY,
            reason: reason.into(),
        })))
        .await
}

/// `GET /api/pty` — the live-pty roster. Sweeps (via `host.list()`) then serializes one
/// row per registered pty. `id` is a string (JS Number can't hold a u64 exactly);
/// timestamps are ISO-8601 (matching `/api/forest`'s recency). `state` is the
/// classifier's `"working" | "waiting" | "idle" | "exited"` (Task 1.9).
async fn pty_list_route(State(state): State<AppState>) -> Response {
    use chrono::{DateTime, Utc};
    // Backfill uuids from claude's pid authority (`sessions/<pid>.json`) before listing,
    // so the roster reflects sessions whose JSONL watcher hasn't fired yet. Cheap (a
    // dozen small files); only when a sessions_dir is configured.
    if let Some(sessions_dir) = &state.config.sessions_dir {
        state.host.reconcile(sessions_dir);
    }
    let rows: Vec<serde_json::Value> = state
        .host
        .list()
        .iter()
        .map(|live| {
            let (uuid, last_activity) = {
                let meta = live.meta_snapshot();
                (meta.uuid, meta.last_activity)
            };
            // The classifier owns the exited short-circuit now (precedence: exited →
            // working → waiting → idle); `state()` takes shared then meta sequentially.
            let state = live.state().as_str();
            let to_iso = |t: SystemTime| DateTime::<Utc>::from(t).to_rfc3339();
            serde_json::json!({
                "id": live.id.to_string(),
                "cwd": live.cwd.as_ref().map(|c| c.display().to_string()),
                "uuid": uuid,
                "state": state,
                "spawnedAt": to_iso(live.spawned_at),
                "lastActivity": to_iso(last_activity),
            })
        })
        .collect();
    axum::Json(rows).into_response()
}

/// `DELETE /api/pty/:id` — kill the child and unlist. 204 on success, 404 if unknown.
async fn pty_delete_route(
    AxumPath(id): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let Ok(id) = id.parse::<host::PtyId>() else {
        return (StatusCode::NOT_FOUND, "no live pty with that id").into_response();
    };
    match state.host.kill(id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(host::KillError::NotFound) => {
            (StatusCode::NOT_FOUND, "no live pty with that id").into_response()
        }
    }
}

/// Resolve which command a pty connection should run, spawning nothing.
/// - `new=<cwd>` → `claude` in that dir (fresh session); watch for its new JSONL.
/// - `session=<uuid>` → `claude --resume <full-uuid>` in that session's cwd.
/// - neither → the configured default (a shell in dev, a dummy in tests).
fn pty_command(cfg: &Config, query: &PtyQuery) -> PtyCommand {
    if let (Some(cwd), Some(projects)) = (&query.new, &cfg.projects_dir) {
        // Expand ~ first: the browser sends the path literally (no shell), and both the
        // spawn cwd and the JSONL-watch project name must reflect the real directory.
        let expanded = expand_tilde(cwd);
        let escaped = escaped_cwd(&expanded.to_string_lossy());
        return PtyCommand {
            program: "claude".to_string(),
            args: vec![],
            cwd: Some(expanded),
            watch: Some((projects.clone(), escaped)),
        };
    }
    if let (Some(uuid), Some(dir)) = (&query.session, &cfg.projects_dir) {
        if let Ok(stub) = eigenform_forest::resolve_stub(dir, uuid) {
            return PtyCommand {
                program: "claude".to_string(),
                args: vec!["--resume".to_string(), stub.uuid],
                cwd: Some(stub.cwd),
                watch: None,
            };
        }
    }
    PtyCommand {
        program: cfg.program.clone(),
        args: cfg.args.clone(),
        cwd: cfg.cwd.clone(),
        watch: None,
    }
}

/// True when a resolved resume command points at a cwd that no longer exists on disk.
/// A session records the cwd it was created in; if that project dir was renamed, moved,
/// or deleted, spawning `claude` there chdir-fails into `$HOME` and `--resume` then can't
/// find the session — so the caller refuses with a clear reason instead of spawning.
/// Symlinks count as present (`is_dir` follows them), so a remapped project still resumes.
fn resume_cwd_missing(command: &PtyCommand) -> bool {
    command.cwd.as_deref().is_some_and(|c| !c.is_dir())
}

/// Claude Code's project dir name for a cwd: `/` → `-` (e.g. `/home/me/p` → `-home-me-p`).
fn escaped_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// Expand a leading `~` or `~/…` to `$HOME` — the launcher input comes from a browser,
/// so there's no shell to do it. Non-tilde paths (and a missing `$HOME`) pass through
/// unchanged. Only a leading `~` is special; `~user` is not expanded.
fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Normalize a path by resolving `.` and `..` components without touching the filesystem
/// (i.e. without canonicalizing — the path may not exist yet). This is used to check
/// whether a `new=<cwd>&create=1` request is under `workspace_root` even when the
/// requested directory hasn't been created yet.
///
/// Rules:
/// - `.` components are skipped.
/// - `..` pops the last accumulated component (if any; at the root it is a no-op).
/// - All other components are pushed.
///
/// The input is treated as an absolute path. If it is relative, it is used as-is
/// (the containment check will likely fail since the root is always absolute).
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// If `path` is a `<uuid>.jsonl` directly under `<projects>/<dir_name>/` and its uuid is
/// not in `baseline`, return that uuid — the freshly-created session.
fn new_session_uuid(
    path: &Path,
    projects: &Path,
    dir_name: &str,
    baseline: &std::collections::HashSet<String>,
) -> Option<String> {
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return None;
    }
    if path.parent() != Some(&projects.join(dir_name)) {
        return None;
    }
    let uuid = path.file_stem().and_then(|s| s.to_str())?;
    (!baseline.contains(uuid)).then(|| uuid.to_string())
}

/// Block until a new `<uuid>.jsonl` appears under `<projects>/<dir_name>/` that wasn't
/// there at the start, returning its uuid. Runs on a dedicated thread.
///
/// `weak` is a downgraded reference to the owning [`host::LivePty`]; when it can no
/// longer be upgraded (pty killed/GC'd) the watch exits within one tick (~1 s) instead
/// of holding the thread for the full 60 s deadline.
fn watch_new_session(
    projects: PathBuf,
    dir_name: String,
    weak: std::sync::Weak<host::LivePty>,
) -> Option<String> {
    let project_dir = projects.join(&dir_name);
    let baseline: std::collections::HashSet<String> = std::fs::read_dir(&project_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
                .then(|| p.file_stem()?.to_str().map(str::to_string))
                .flatten()
        })
        .collect();

    let (raw_tx, raw_rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = raw_tx.send(res);
    })
    .ok()?;
    // Watch the projects root recursively so a brand-new project dir is covered too.
    notify::Watcher::watch(&mut watcher, &projects, notify::RecursiveMode::Recursive).ok()?;

    // Bound the wait so an abandoned "new session" connection can't leak this thread.
    // Tick every ~1 s so a dead/killed pty (detected via the Weak) ends the watch
    // promptly rather than holding the thread for the full 60 s budget.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let tick = std::time::Duration::from_secs(1);
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        // Drop early if the LivePty has already been released (pty killed/GC'd).
        weak.upgrade()?;
        match raw_rx.recv_timeout(remaining.min(tick)) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if let Some(uuid) = new_session_uuid(path, &projects, &dir_name, &baseline) {
                        return Some(uuid);
                    }
                }
            }
            Ok(Err(_)) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue, // tick: re-check weak
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None, // watcher gone
        }
    }
}

#[derive(Clone)]
struct PtyCommand {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    /// For a fresh session: (projects_dir, escaped-cwd dir name) to watch for the new JSONL.
    watch: Option<(PathBuf, String)>,
}

// The bridge's outbound type now lives in `host` (Task 1.3), where Task 1.4's pump
// fans pty output out to subscribers as `Outbound`. Re-exported so this file's bridge
// keeps using it unchanged.
use host::Outbound;

fn origin_is_local(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return true; // no Origin → not a browser CSRF
    };
    let authority = origin.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
    matches!(host_of(authority), "127.0.0.1" | "localhost" | "::1")
}

/// The host portion of an `authority` (`host`, `host:port`, or `[ipv6]:port`).
fn host_of(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    }
}

/// Control messages from the browser. Output flows the other way as raw binary frames.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Control {
    Stdin { data: String },
    Resize { cols: u16, rows: u16 },
}

/// Bridge one websocket to an already-registered [`host::LivePty`]. The protocol:
///
/// 1. a text frame `{"type":"pty","id":"<id>"}` announcing the pty's id,
/// 2. the repaint snapshot as one binary frame (the current screen),
/// 3. the live stream (binary pty output + text control frames) until the socket closes.
///
/// Snapshot + subscription are atomic under `live.attach()`, so no byte is lost or
/// doubled across the seam. After subscribing we check `exited_at()`: if the child
/// already exited, the `{"type":"exit"}` broadcast fired *before* this subscriber
/// existed, so we synthesize one to THIS socket. Checking strictly after `attach`
/// subscribes means we either receive the live broadcast or observe `exited_at` — never
/// neither (the TOCTOU-safe ordering).
///
/// The socket read loop dispatches `Control` messages to `live.write_input` /
/// `live.resize`. When the socket closes, the read loop ends, the pump task is aborted,
/// and the receiver drops — which detaches us from the pty's subscriber set. The pty
/// itself lives on.
async fn attach_socket(socket: WebSocket, live: Arc<host::LivePty>) {
    let (mut sink, mut stream) = socket.split();

    // 1. Announce the id.
    if sink
        .send(Message::Text(
            serde_json::json!({"type": "pty", "id": live.id.to_string()}).to_string(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // 2. Subscribe + snapshot atomically.
    let (snapshot, mut rx) = live.attach();

    // 3a. Repaint. (Always send, even if empty — keeps the frame ordering uniform.)
    if sink.send(Message::Binary(snapshot)).await.is_err() {
        return;
    }

    // TOCTOU: if the child exited before we subscribed, the exit broadcast missed us.
    // Synthesize it to this socket only. (Done after `attach` so we never miss both.)
    if live.exited_at().is_some() && sink.send(Message::Text(r#"{"type":"exit"}"#.into())).await.is_err() {
        return;
    }

    // 3b. Pump fan-out → socket. Ends when the channel closes or the socket send fails.
    // Interleaved with a periodic Ping: a quiet pty (claude thinking, a long tool
    // call) otherwise leaves the socket idle, and idle WebSockets get reaped by
    // WSL2 localhost forwarding / NAT — which silently freezes the client tab.
    // The browser auto-replies Pong; it lands in the read loop below as `_ => {}`.
    let send_task = tokio::spawn(async move {
        let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(20));
        keepalive.tick().await; // first tick is immediate — skip it.
        loop {
            tokio::select! {
                out = rx.recv() => match out {
                    Some(out) => {
                        let msg = match out {
                            Outbound::Binary(b) => Message::Binary(b),
                            Outbound::Text(t) => Message::Text(t),
                        };
                        if sink.send(msg).await.is_err() {
                            break; // socket gone: stop pumping (and drop rx → detach).
                        }
                    }
                    None => break, // channel closed: nothing left to pump.
                },
                _ = keepalive.tick() => {
                    if sink.send(Message::Ping(Vec::new())).await.is_err() {
                        break; // socket gone.
                    }
                }
            }
        }
    });

    // 3c. Socket read loop: client control messages → the pty.
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => match serde_json::from_str::<Control>(&t) {
                // `Control::Resize { cols, rows }` maps to `live.resize(cols, rows)`:
                // LivePty/Pty take `(cols, rows)` (TermModel flips internally).
                Ok(Control::Stdin { data }) => {
                    // Input to a dead child is dropped silently; the {"type":"exit"} frame
                    // already informed the client that the process has ended.
                    let _ = live.write_input(data.as_bytes());
                }
                Ok(Control::Resize { cols, rows }) => {
                    let _ = live.resize(cols, rows);
                }
                Err(_) => {}
            },
            Message::Binary(b) => {
                // Input to a dead child is dropped silently; the {"type":"exit"} frame
                // already informed the client that the process has ended.
                let _ = live.write_input(&b);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    // Socket closed: abort the pump and drop `rx` (the task owns it), detaching us. The
    // pump's next `send` to our dead sender is pruned by `retain` on the host side.
    send_task.abort();
}

/// A command running in a pseudo-terminal: stream its output via [`Pty::reader`], send
/// input via [`Pty::write_input`], and follow the terminal size via [`Pty::resize`].
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
}

impl Pty {
    /// Spawn `program` with `args` in a pty of `(cols, rows)`, optionally in `cwd`.
    pub fn spawn(
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        size: (u16, u16),
    ) -> anyhow::Result<Pty> {
        let (cols, rows) = size;
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(program);
        for arg in args {
            cmd.arg(arg);
        }
        // Only set cwd if it still exists. A recorded session dir can move or be
        // deleted (e.g. a renamed checkout); setting cwd to a missing dir makes the
        // post-fork chdir fail and abort the whole daemon (SIGABRT) instead of just
        // this child. Falling through lets the child inherit the daemon's cwd.
        if let Some(cwd) = cwd {
            if cwd.is_dir() {
                cmd.cwd(cwd);
            }
        }

        let child = pair.slave.spawn_command(cmd)?;
        // Close the slave handle in the parent so EOF propagates when the child exits.
        drop(pair.slave);
        let writer = pair.master.take_writer()?;

        Ok(Pty {
            master: pair.master,
            child,
            writer,
        })
    }

    /// A fresh reader over the pty's output. Reads block until data or EOF.
    pub fn reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
        self.master.try_clone_reader()
    }

    /// OS pid of the child, for `sessions/<pid>.json` reconciliation. `None` once
    /// the child has been reaped.
    pub fn child_pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Block until the child exits, reaping the zombie. Called by the session host's
    /// pump at EOF (the reader saw the master close), so the child is already dying or
    /// dead and this returns promptly. Errors propagate from the underlying wait.
    pub fn wait_child(&mut self) -> std::io::Result<()> {
        self.child.wait().map(|_status| ())
    }

    /// Signal the child to terminate (SIGKILL via portable-pty's `kill`). The pump's
    /// EOF path then reaps it; an explicit `wait_child` here would block, so callers
    /// that want a synchronous reap call `wait_child` after.
    pub fn kill_child(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    /// Send bytes to the child's stdin.
    pub fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Tell the pty the terminal was resized to `(cols, rows)`.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{Duration, Instant};

    /// Poll `rx` for up to `dur`, returning true if a signal arrives.
    fn recv_within(rx: &mut tokio::sync::mpsc::Receiver<()>, dur: Duration) -> bool {
        let deadline = Instant::now() + dur;
        loop {
            match rx.try_recv() {
                Ok(()) => return true,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Join `handle`, returning true if it finishes within `dur` (false = still running/hung).
    fn join_within(handle: std::thread::JoinHandle<()>, dur: Duration) -> bool {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = handle.join();
            let _ = done_tx.send(());
        });
        done_rx.recv_timeout(dur).is_ok()
    }

    #[test]
    fn watch_channel_emits_on_matching_file_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, b"start\n").unwrap();
        let (mut rx, _h) =
            watch_channel(dir.path().to_path_buf(), Some("session.jsonl".into()));
        // Let the watcher arm (the SSE response returns before watch() completes).
        std::thread::sleep(Duration::from_millis(300));

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"more\n").unwrap();
        f.flush().unwrap();

        assert!(
            recv_within(&mut rx, Duration::from_secs(2)),
            "appending to the watched file should signal a change",
        );
    }

    #[test]
    fn watch_channel_ignores_non_target_files() {
        let dir = tempfile::tempdir().unwrap();
        let (mut rx, _h) =
            watch_channel(dir.path().to_path_buf(), Some("session.jsonl".into()));
        std::thread::sleep(Duration::from_millis(300));

        std::fs::write(dir.path().join("other.txt"), b"noise\n").unwrap();

        assert!(
            !recv_within(&mut rx, Duration::from_millis(800)),
            "writes to non-target files must not signal",
        );
    }

    #[test]
    fn watch_thread_exits_when_consumer_drops_even_if_dir_is_quiet() {
        // Regression test for the inotify/thread leak: when an SSE client disconnects, the
        // watcher must clean up promptly WITHOUT waiting for a filesystem event. The old
        // implementation blocked on the event channel and only noticed the dead client after
        // the next write, stranding the thread + inotify instance for quiet sessions.
        let dir = tempfile::tempdir().unwrap();
        let (rx, handle) =
            watch_channel(dir.path().to_path_buf(), Some("session.jsonl".into()));
        std::thread::sleep(Duration::from_millis(200)); // let it arm

        drop(rx); // client gone; directory stays quiet (no writes follow)

        assert!(
            join_within(handle, Duration::from_secs(3)),
            "watcher thread must exit within ~1s of the consumer dropping, with no fs events",
        );
    }

    #[test]
    fn normalize_path_resolves_dotdot() {
        // Normal path — unchanged.
        assert_eq!(normalize_path(Path::new("/a/b/c")), PathBuf::from("/a/b/c"));
        // Single `..` — pops one component.
        assert_eq!(normalize_path(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        // Double `..` — escapes the workspace.
        assert_eq!(
            normalize_path(Path::new("/workspace/child/../../outside")),
            PathBuf::from("/outside")
        );
        // `.` is skipped.
        assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
        // `..` at root is a no-op (no component to pop).
        assert_eq!(normalize_path(Path::new("/../x")), PathBuf::from("/x"));
    }

    #[test]
    fn expand_tilde_uses_home_for_leading_tilde_only() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        // Absolute and relative non-tilde paths pass through untouched.
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_tilde("relative/x"), PathBuf::from("relative/x"));
        // `~user` is NOT expanded (only a bare ~ or ~/).
        assert_eq!(expand_tilde("~bob/x"), PathBuf::from("~bob/x"));
        if let Some(home) = home {
            assert_eq!(expand_tilde("~"), home);
            assert_eq!(expand_tilde("~/src/proj"), home.join("src/proj"));
        }
    }

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
        assert_eq!(&*r2, "JSON-A", "second view must be served from cache, not re-rendered");
        assert_eq!(calls.get(), 1, "render must run only once for an unchanged file");

        // Mutating the file changes its (mtime, len) stamp → the cache re-renders.
        std::fs::write(&path, "three!").unwrap();
        let r3 = cache.get_or_render(&path, || render("JSON-C")).unwrap();
        assert_eq!(&*r3, "JSON-C");
        assert_eq!(calls.get(), 2, "a changed file must invalidate the cache");
    }

    #[test]
    fn session_query_resolves_to_claude_resume_without_spawning() {
        // A temp projects dir with one session; pty_command must map it to claude --resume
        // in the session's cwd — and spawn nothing.
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("-home-me-proj");
        std::fs::create_dir_all(&pdir).unwrap();
        let uuid = "abcdef00-0000-4000-8000-000000000000";
        std::fs::write(
            pdir.join(format!("{uuid}.jsonl")),
            format!(r#"{{"type":"user","uuid":"u1","cwd":"/home/me/proj","sessionId":"{uuid}"}}"#) + "\n",
        )
        .unwrap();

        let cfg = Config {
            program: "bash".into(),
            args: vec![],
            cwd: None,
            web_dir: None,
            term_dir: None,
            projects_dir: Some(dir.path().to_path_buf()),
            sessions_dir: None,
            state_dir: None,
            workspace_root: None,
            dev: false,
        };

        let resumed = pty_command(
            &cfg,
            &PtyQuery { attach: None, session: Some("abcdef00".into()), new: None, create: 0 },
        );
        assert_eq!(resumed.program, "claude");
        assert_eq!(resumed.args, vec!["--resume".to_string(), uuid.to_string()]);
        assert_eq!(resumed.cwd.as_deref(), Some(std::path::Path::new("/home/me/proj")));

        // No session → the configured default, never claude.
        let default = pty_command(&cfg, &PtyQuery { attach: None, session: None, new: None, create: 0 });
        assert_eq!(default.program, "bash");

        // new=<cwd> → fresh claude in that dir, with a watch target for its new JSONL.
        let fresh = pty_command(
            &cfg,
            &PtyQuery { attach: None, session: None, new: Some("/home/me/fresh".into()), create: 0 },
        );
        assert_eq!(fresh.program, "claude");
        assert!(fresh.args.is_empty());
        assert_eq!(fresh.cwd.as_deref(), Some(std::path::Path::new("/home/me/fresh")));
        assert_eq!(fresh.watch.as_ref().map(|(_, d)| d.as_str()), Some("-home-me-fresh"));
    }

    #[test]
    fn resume_into_a_vanished_project_dir_is_refused_not_spawned_in_home() {
        // A session records the cwd it was created in. If the user renames/moves/deletes
        // that project, the recorded cwd is gone — but pty_command still happily resolves
        // the resume to it. Spawning there chdir-fails into $HOME and `--resume` then can't
        // find the session (the baffling bug). The guard must flag the vanished cwd so the
        // caller refuses instead of spawning.
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("-home-me-gone");
        std::fs::create_dir_all(&pdir).unwrap();
        let gone = "abcdef00-0000-4000-8000-000000000000";
        std::fs::write(
            pdir.join(format!("{gone}.jsonl")),
            format!(r#"{{"type":"user","uuid":"u1","cwd":"/home/me/gone-forever","sessionId":"{gone}"}}"#) + "\n",
        )
        .unwrap();

        // A second session whose recorded cwd DOES still exist (a real dir we create).
        let live_cwd = dir.path().join("still-here");
        std::fs::create_dir_all(&live_cwd).unwrap();
        let pdir2 = dir.path().join("-still-here");
        std::fs::create_dir_all(&pdir2).unwrap();
        let live = "abcdef01-0000-4000-8000-000000000000";
        std::fs::write(
            pdir2.join(format!("{live}.jsonl")),
            format!(
                r#"{{"type":"user","uuid":"u1","cwd":"{}","sessionId":"{live}"}}"#,
                live_cwd.display()
            ) + "\n",
        )
        .unwrap();

        let cfg = Config {
            program: "bash".into(),
            args: vec![],
            cwd: None,
            web_dir: None,
            term_dir: None,
            projects_dir: Some(dir.path().to_path_buf()),
            sessions_dir: None,
            state_dir: None,
            workspace_root: None,
            dev: false,
        };

        // The vanished-cwd resume still resolves to claude --resume in the recorded cwd...
        let vanished = pty_command(
            &cfg,
            &PtyQuery { attach: None, session: Some("abcdef00".into()), new: None, create: 0 },
        );
        assert_eq!(
            vanished.cwd.as_deref(),
            Some(std::path::Path::new("/home/me/gone-forever"))
        );
        // ...but the guard flags it, so pty_ws refuses rather than spawning in $HOME.
        assert!(
            resume_cwd_missing(&vanished),
            "a resume whose recorded cwd no longer exists must be flagged"
        );

        // A resume whose cwd is still present is NOT flagged — it spawns normally.
        let present = pty_command(
            &cfg,
            &PtyQuery { attach: None, session: Some("abcdef01".into()), new: None, create: 0 },
        );
        assert_eq!(present.cwd.as_deref(), Some(live_cwd.as_path()));
        assert!(
            !resume_cwd_missing(&present),
            "a resume whose cwd still exists must not be flagged"
        );
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
            args: vec![],
            cwd: None,
            web_dir: None,
            term_dir: None,
            projects_dir: Some(dir.path().to_path_buf()),
            sessions_dir: None,
            state_dir: None,
            workspace_root: None,
            dev: false,
        };

        // fork "before" u2 → rewind to the s1 boundary; u2 and its tail drop.
        let new_uuid = fork_session(&cfg, "abcdef00", "u2").expect("fork ok");
        assert_ne!(new_uuid, uuid, "fork mints a fresh id");

        // the branch lands in the SAME project dir (so resume/Forest find it under the cwd)
        let forked = pdir.join(format!("{new_uuid}.jsonl"));
        let body = std::fs::read_to_string(&forked).expect("fork file written beside source");
        assert!(body.contains("first prompt"), "the kept prefix survives");
        assert!(!body.contains("second prompt"), "the edited turn is dropped (delivered live)");
        assert!(!body.contains("reply two"), "the downstream reply is dropped");
        // resumable: the new resume head is the completed-turn system row, not a user turn
        let forked_session = eigenform_surgery::Session::parse_str(&body).unwrap_or_else(|e| match e {});
        assert_eq!(forked_session.resume_leaf().as_deref(), Some("s1"));

        // copy-on-fork: the source is byte-for-byte untouched
        assert_eq!(std::fs::read_to_string(&src).unwrap(), jsonl);
    }

    #[test]
    fn detects_a_new_session_jsonl_under_the_project_dir() {
        let baseline: std::collections::HashSet<String> =
            ["old1".to_string()].into_iter().collect();
        let projects = std::path::Path::new("/x/.claude/projects");
        let dir_name = "-home-me-fresh";

        // a brand-new jsonl under the matching project dir → its uuid
        let fresh = projects.join(dir_name).join("new-uuid-123.jsonl");
        assert_eq!(
            new_session_uuid(&fresh, projects, dir_name, &baseline).as_deref(),
            Some("new-uuid-123")
        );
        // a pre-existing one (in baseline) → ignored
        let old = projects.join(dir_name).join("old1.jsonl");
        assert_eq!(new_session_uuid(&old, projects, dir_name, &baseline), None);
        // a file under a different project → ignored
        let other = projects.join("-other").join("x.jsonl");
        assert_eq!(new_session_uuid(&other, projects, dir_name, &baseline), None);
    }
}
