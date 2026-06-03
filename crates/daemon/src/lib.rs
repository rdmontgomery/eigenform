//! eigen-daemon: the woland backend — pty manager + http/ws server.
//!
//! Slice 1: the pty bridge. Spawn an arbitrary command in a pty and stream its stdio.
//! The bridge drives ANY command; real `claude --resume` is launched only by the user,
//! never by tests or the agent. See `docs/plans/2026-06-03-woland-design.md`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
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
        .route("/api/sessions", get(sessions_route))
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

/// Resolve, read, parse, and render a session's transcript fragment.
fn session_fragment(cfg: &Config, uuid: &str) -> Result<String, (StatusCode, &'static str)> {
    let dir = cfg
        .projects_dir
        .as_ref()
        .ok_or((StatusCode::NOT_FOUND, "no projects dir configured"))?;
    let path = eigen_forest::resolve(dir, uuid).map_err(|_| (StatusCode::NOT_FOUND, "no such session"))?;
    let contents =
        std::fs::read_to_string(&path).map_err(|_| (StatusCode::NOT_FOUND, "could not read session"))?;
    // parse_str is currently infallible (ParseError is uninhabited).
    let session = eigen_surgery::Session::parse_str(&contents).unwrap_or_else(|e| match e {});
    Ok(eigen_render::session_html(&session))
}

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
    /// Resume this session in the pty (spawns `claude --resume`). Absent = the default
    /// command (a shell). Only a real connection spawns anything.
    session: Option<String>,
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
    let command = pty_command(&cfg, query.session.as_deref());
    ws.on_upgrade(move |socket| bridge(socket, command))
}

/// Resolve which command a pty connection should run. `session` → `claude --resume
/// <full-uuid>` in that session's cwd (the user-initiated, token-spending path). No
/// session → the configured default (a shell in dev, a dummy in tests). Pure: spawns
/// nothing.
fn pty_command(cfg: &Config, session: Option<&str>) -> PtyCommand {
    if let (Some(uuid), Some(dir)) = (session, &cfg.projects_dir) {
        if let Ok(stub) = eigen_forest::resolve_stub(dir, uuid) {
            return PtyCommand {
                program: "claude".to_string(),
                args: vec!["--resume".to_string(), stub.uuid],
                cwd: Some(stub.cwd),
            };
        }
    }
    PtyCommand {
        program: cfg.program.clone(),
        args: cfg.args.clone(),
        cwd: cfg.cwd.clone(),
    }
}

#[derive(Clone)]
struct PtyCommand {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
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

    // Blocking pty reads live on a dedicated thread, forwarded over a channel.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let send_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if sink.send(Message::Binary(bytes)).await.is_err() {
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
            dev: false,
        };

        let resumed = pty_command(&cfg, Some("abcdef00"));
        assert_eq!(resumed.program, "claude");
        assert_eq!(resumed.args, vec!["--resume".to_string(), uuid.to_string()]);
        assert_eq!(resumed.cwd.as_deref(), Some(std::path::Path::new("/home/me/proj")));

        // No session → the configured default, never claude.
        let default = pty_command(&cfg, None);
        assert_eq!(default.program, "bash");
    }
}
