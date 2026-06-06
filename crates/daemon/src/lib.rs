//! eigen-daemon: the woland backend — pty manager + http/ws server.
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

/// What the daemon runs when a terminal connects. For slice 1 this is a fixed command
/// (a shell for the demo, a dummy in tests) — NOT arbitrary exec from the request.
#[derive(Clone, Debug)]
pub struct Config {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Directory of static frontend assets to serve at `/`. None = API only.
    pub web_dir: Option<PathBuf>,
    /// `~/.claude/projects` (or a test dir) for session resolution. None = no transcript.
    pub projects_dir: Option<PathBuf>,
    /// `~/.claude/sessions` (or a test dir): `<pid>.json` files for liveness. None = no
    /// live Forest.
    pub sessions_dir: Option<PathBuf>,
    /// `~/.eigen/state`: persisted per-session metrics (the activity spark). None = no spark.
    pub state_dir: Option<PathBuf>,
    /// Dev mode: inject the live-reload hook and serve `/api/dev/reload`.
    pub dev: bool,
}

/// Build the woland HTTP/WS router. `GET /pty` upgrades to a websocket bridged to a pty;
/// `web_dir`, if set, is served as static files at `/`.
pub fn app(config: Config) -> Router {
    let mut router = Router::new()
        .route("/pty", get(pty_ws))
        .route("/session/:uuid", get(session_route))
        .route("/api/session/:uuid", get(session_fragment_route))
        .route("/api/session/:uuid/json", get(session_json_route))
        .route("/api/session/:uuid/fork", post(fork_route))
        .route("/api/sessions", get(sessions_route))
        .route("/api/forest", get(forest_route))
        .route("/api/watch/forest", get(forest_watch_route))
        .route("/api/projects", get(projects_route))
        .route("/api/recent", get(recent_route))
        .route("/api/watch/:uuid", get(watch_route));
    if let Some(web_dir) = &config.web_dir {
        // Dev routes take precedence over the static fallback.
        if config.dev {
            router = router
                .route("/", get(dev_index))
                .route("/api/dev/reload", get(dev_reload));
        }
        let index = web_dir.join("index.html");
        router = router.fallback_service(
            tower_http::services::ServeDir::new(web_dir)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    }
    router.with_state(Arc::new(config))
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
    State(cfg): State<Arc<Config>>,
) -> Response {
    match session_fragment(&cfg, &uuid) {
        Ok(frag) => Html(transcript_page(&frag)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/session/:uuid` — just the transcript fragment, for in-page injection into
/// the Manuscript (no page chrome).
async fn session_fragment_route(
    AxumPath(uuid): AxumPath<String>,
    State(cfg): State<Arc<Config>>,
) -> Response {
    match session_fragment(&cfg, &uuid) {
        Ok(frag) => Html(frag).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/session/:uuid/json` — the transcript as structured JSON for the Manuscript
/// (exchanges + a trailing leaf), so woland can fold/annotate per turn rather than inject
/// opaque HTML.
async fn session_json_route(
    AxumPath(uuid): AxumPath<String>,
    State(cfg): State<Arc<Config>>,
) -> Response {
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    let Ok(path) = eigen_forest::resolve(dir, &uuid) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    // Render once per (file, mtime, len); repeat views and forest-browsing skip the
    // multi-MB read+parse+serialize that dominates the manuscript load latency.
    match SESSION_CACHE.get_or_render(&path, || {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        let session = eigen_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {});
        eigen_render::session_json(&session)
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
    Ok(eigen_render::session_html(&load_session(cfg, uuid)?))
}

/// `POST /api/session/:uuid/fork` — edit-then-fork at a turn. Body `{turn, text}`:
/// re-author the turn `turn` (a turn uuid) with `text`, drop everything after it, and
/// write a NEW session beside the source (copy-on-fork — the source is never touched).
/// Returns `{uuid}` of the new branch, which the client then resumes in the Furnace.
async fn fork_route(
    AxumPath(uuid): AxumPath<String>,
    State(cfg): State<Arc<Config>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let Some(turn) = body.get("turn").and_then(|v| v.as_str()) else {
        return (StatusCode::BAD_REQUEST, "missing `turn`").into_response();
    };
    // `text` (the edited prompt) is delivered live into the resumed branch by the client,
    // not written into the file — the fork must end on a completed turn to be resumable.
    match fork_session(&cfg, &uuid, turn) {
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
        eigen_forest::resolve(dir, src_uuid).map_err(|_| (StatusCode::NOT_FOUND, "no such session"))?;
    let contents = std::fs::read_to_string(&src_path)
        .map_err(|_| (StatusCode::NOT_FOUND, "could not read session"))?;
    let session = eigen_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {});
    let forked = eigen_surgery::fork_before(&session, turn)
        .map_err(|_| (StatusCode::UNPROCESSABLE_ENTITY, "cannot fork before that turn"))?;
    let project_dir = src_path
        .parent()
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "session path has no parent"))?;
    eigen_surgery::write(&forked, project_dir)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "could not write fork"))
}

/// Resolve, read, and parse a session's JSONL into a [`Session`].
fn load_session(cfg: &Config, uuid: &str) -> Result<eigen_surgery::Session, (StatusCode, &'static str)> {
    let dir = cfg
        .projects_dir
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "no projects dir configured"))?;
    let path = eigen_forest::resolve(dir, uuid).map_err(|_| (StatusCode::NOT_FOUND, "no such session"))?;
    let contents =
        std::fs::read_to_string(&path).map_err(|_| (StatusCode::NOT_FOUND, "could not read session"))?;
    // parse_str is currently infallible (ParseError is uninhabited).
    Ok(eigen_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {}))
}

/// Cache of rendered session JSON, keyed by file path and invalidated by the file's
/// (modified-time, length) stamp. A static transcript is parsed once; the live session
/// (whose file grows each turn) re-renders only when it actually changes.
#[derive(Default)]
struct SessionJsonCache {
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
async fn sessions_route(State(cfg): State<Arc<Config>>) -> Response {
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    match eigen_forest::list(dir, eigen_forest::Scope::AllProjects, None, chrono::Utc::now()) {
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
async fn projects_route(State(cfg): State<Arc<Config>>) -> Response {
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    match eigen_forest::list(dir, eigen_forest::Scope::AllProjects, None, chrono::Utc::now()) {
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
/// activity spark). Mirrors what `eigen forest --live` prints.
async fn forest_route(State(cfg): State<Arc<Config>>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        forest_json(&cfg),
    )
        .into_response()
}

/// `GET /api/watch/forest` — SSE that pushes the snapshot whenever it changes.
async fn forest_watch_route(State(cfg): State<Arc<Config>>) -> Response {
    forest_sse(cfg)
}

/// Compute the live-Forest snapshot as a JSON string. Empty array if the dirs aren't set.
fn forest_json(cfg: &Config) -> String {
    let (Some(projects), Some(sessions), Some(state)) =
        (&cfg.projects_dir, &cfg.sessions_dir, &cfg.state_dir)
    else {
        return "[]".to_string();
    };
    let rows: Vec<serde_json::Value> =
        eigen_forest::live_forest(projects, sessions, state, chrono::Utc::now())
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
    axum::response::sse::Sse::new(stream).into_response()
}

/// `GET /api/recent` — the most recent session uuid across all projects.
async fn recent_route(State(cfg): State<Arc<Config>>) -> Response {
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    match eigen_forest::list(dir, eigen_forest::Scope::AllProjects, None, chrono::Utc::now()) {
        Ok(mut sessions) => match sessions.drain(..).next() {
            Some(s) => s.uuid.into_response(),
            None => (StatusCode::NOT_FOUND, "no sessions").into_response(),
        },
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response(),
    }
}

/// `GET /api/watch/:uuid` — Server-Sent Events: a `change` event each time the session's
/// JSONL is written (the live-follow signal for the right pane).
async fn watch_route(
    AxumPath(uuid): AxumPath<String>,
    State(cfg): State<Arc<Config>>,
) -> Response {
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    let path = match eigen_forest::resolve(dir, &uuid) {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "no such session").into_response(),
    };
    let watch_dir = path.parent().unwrap_or(&path).to_path_buf();
    let target = path.file_name().map(|n| n.to_os_string());
    watch_sse(watch_dir, target)
}

/// `GET /` in dev mode: the static index with a live-reload hook injected.
async fn dev_index(State(cfg): State<Arc<Config>>) -> Response {
    let Some(web_dir) = &cfg.web_dir else {
        return (StatusCode::NOT_FOUND, "no web dir").into_response();
    };
    let Ok(html) = std::fs::read_to_string(web_dir.join("index.html")) else {
        return (StatusCode::NOT_FOUND, "no index.html").into_response();
    };
    let injected = html.replacen(
        "<head>",
        "<head>\n    <meta name=\"eigen-dev\" content=\"1\" />",
        1,
    );
    Html(injected).into_response()
}

/// `GET /api/dev/reload` — SSE that fires whenever the built frontend bundle changes.
async fn dev_reload(State(cfg): State<Arc<Config>>) -> Response {
    let Some(web_dir) = &cfg.web_dir else {
        return (StatusCode::NOT_FOUND, "no web dir").into_response();
    };
    watch_sse(web_dir.join("dist"), None)
}

/// SSE that emits a `change` event when files in `watch_dir` are written. If `target` is
/// set, only that filename triggers; otherwise any change in the dir does. A dedicated
/// thread owns the watcher and lives until the SSE receiver is dropped.
fn watch_sse(watch_dir: PathBuf, target: Option<std::ffi::OsString>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<()>(8);
    std::thread::spawn(move || {
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
        for event in raw_rx {
            let Ok(event) = event else { continue };
            let touches = match &target {
                Some(name) => event.paths.iter().any(|p| p.file_name() == Some(name.as_os_str())),
                None => true,
            };
            if touches && tx.blocking_send(()).is_err() {
                break; // SSE connection gone; drop the watcher
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
        .map(|_| Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data("change")));
    axum::response::sse::Sse::new(stream).into_response()
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
    /// Resume this session in the pty (spawns `claude --resume`).
    session: Option<String>,
    /// Start a fresh session: spawn `claude` in this cwd. Takes precedence over `session`.
    new: Option<String>,
    // Absent both = the default command (a shell). Only a real connection spawns anything.
}

async fn pty_ws(
    ws: WebSocketUpgrade,
    State(cfg): State<Arc<Config>>,
    axum::extract::Query(query): axum::extract::Query<PtyQuery>,
    headers: HeaderMap,
) -> Response {
    // Defend against CSRF-to-localhost: a page you visit must not be able to open a
    // shell on this daemon. Browsers always send Origin; reject any that isn't local.
    // A missing Origin means a non-browser client (curl, our tests) — allowed.
    if !origin_is_local(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin websocket rejected").into_response();
    }
    let command = pty_command(&cfg, &query);
    ws.on_upgrade(move |socket| bridge(socket, command))
}

/// Resolve which command a pty connection should run, spawning nothing.
/// - `new=<cwd>` → `claude` in that dir (fresh session); watch for its new JSONL.
/// - `session=<uuid>` → `claude --resume <full-uuid>` in that session's cwd.
/// - neither → the configured default (a shell in dev, a dummy in tests).
fn pty_command(cfg: &Config, query: &PtyQuery) -> PtyCommand {
    if let (Some(cwd), Some(projects)) = (&query.new, &cfg.projects_dir) {
        return PtyCommand {
            program: "claude".to_string(),
            args: vec![],
            cwd: Some(PathBuf::from(cwd)),
            watch: Some((projects.clone(), escaped_cwd(cwd))),
        };
    }
    if let (Some(uuid), Some(dir)) = (&query.session, &cfg.projects_dir) {
        if let Ok(stub) = eigen_forest::resolve_stub(dir, uuid) {
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

/// Claude Code's project dir name for a cwd: `/` → `-` (e.g. `/home/me/p` → `-home-me-p`).
fn escaped_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
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
fn watch_new_session(projects: PathBuf, dir_name: String) -> Option<String> {
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        match raw_rx.recv_timeout(remaining) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if let Some(uuid) = new_session_uuid(path, &projects, &dir_name, &baseline) {
                        return Some(uuid);
                    }
                }
            }
            Ok(Err(_)) => continue,
            Err(_) => return None, // timeout or watcher gone
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

/// What the daemon pushes to the browser over the pty websocket.
enum Outbound {
    /// Raw pty output.
    Binary(Vec<u8>),
    /// A JSON control message (e.g. a new session's uuid).
    Text(String),
}

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

/// Bridge one websocket to a freshly-spawned pty: pty output → binary frames, client
/// control messages → stdin / resize.
async fn bridge(socket: WebSocket, command: PtyCommand) {
    let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
    let mut pty = match Pty::spawn(&command.program, &args, command.cwd.as_deref(), (80, 24)) {
        Ok(p) => p,
        Err(_) => return,
    };
    let reader = match pty.reader() {
        Ok(r) => r,
        Err(_) => return,
    };

    let (mut sink, mut stream) = socket.split();

    // pty output → binary frames; control messages (e.g. a new session's uuid) → text.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Outbound>(64);

    // Blocking pty reads live on a dedicated thread.
    let read_tx = tx.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if read_tx.blocking_send(Outbound::Binary(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // For a fresh session: watch for its new JSONL and report the uuid to the client.
    if let Some((projects, dir_name)) = command.watch.clone() {
        let detect_tx = tx.clone();
        std::thread::spawn(move || {
            if let Some(uuid) = watch_new_session(projects, dir_name) {
                let msg = format!(r#"{{"type":"session","uuid":"{uuid}"}}"#);
                let _ = detect_tx.blocking_send(Outbound::Text(msg));
            }
        });
    }
    drop(tx); // only the worker threads keep senders; rx closes when they're done

    let send_task = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let msg = match out {
                Outbound::Binary(b) => Message::Binary(b),
                Outbound::Text(t) => Message::Text(t),
            };
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => match serde_json::from_str::<Control>(&t) {
                Ok(Control::Stdin { data }) => {
                    let _ = pty.write_input(data.as_bytes());
                }
                Ok(Control::Resize { cols, rows }) => {
                    let _ = pty.resize(cols, rows);
                }
                Err(_) => {}
            },
            Message::Binary(b) => {
                let _ = pty.write_input(&b);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    send_task.abort();
}

/// A command running in a pseudo-terminal: stream its output via [`Pty::reader`], send
/// input via [`Pty::write_input`], and follow the terminal size via [`Pty::resize`].
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    #[allow(dead_code)]
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
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
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
        Ok(self.master.try_clone_reader()?)
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
            projects_dir: Some(dir.path().to_path_buf()),
            sessions_dir: None,
            state_dir: None,
            dev: false,
        };

        let resumed = pty_command(
            &cfg,
            &PtyQuery { session: Some("abcdef00".into()), new: None },
        );
        assert_eq!(resumed.program, "claude");
        assert_eq!(resumed.args, vec!["--resume".to_string(), uuid.to_string()]);
        assert_eq!(resumed.cwd.as_deref(), Some(std::path::Path::new("/home/me/proj")));

        // No session → the configured default, never claude.
        let default = pty_command(&cfg, &PtyQuery { session: None, new: None });
        assert_eq!(default.program, "bash");

        // new=<cwd> → fresh claude in that dir, with a watch target for its new JSONL.
        let fresh = pty_command(
            &cfg,
            &PtyQuery { session: None, new: Some("/home/me/fresh".into()) },
        );
        assert_eq!(fresh.program, "claude");
        assert!(fresh.args.is_empty());
        assert_eq!(fresh.cwd.as_deref(), Some(std::path::Path::new("/home/me/fresh")));
        assert_eq!(fresh.watch.as_ref().map(|(_, d)| d.as_str()), Some("-home-me-fresh"));
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
            projects_dir: Some(dir.path().to_path_buf()),
            sessions_dir: None,
            state_dir: None,
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
        let forked_session = eigen_surgery::Session::parse_str(&body).unwrap_or_else(|e| match e {});
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
