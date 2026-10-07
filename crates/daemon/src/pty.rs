//! The pty websocket: spawn / resume / attach a session pty and bridge it to the browser.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::paths::{escaped_cwd, expand_tilde, normalize_path};
use crate::{host, AppState, Config};

#[derive(serde::Deserialize)]
pub(crate) struct PtyQuery {
    /// Re-attach to an already-registered pty by id; spawns nothing. Highest precedence.
    attach: Option<host::PtyId>,
    /// Resume this session in the pty (spawns `claude --resume`).
    session: Option<String>,
    /// Start a fresh session: spawn `claude` in this cwd. Takes precedence over `session`.
    new: Option<String>,
    /// Open a plain terminal in this cwd: the daemon's configured program (a shell), not
    /// `claude`. No JSONL watch — a terminal tab has no transcript, so nothing ever calls
    /// `set_uuid`; the drawer/reach map/rail Links section just show their empty state.
    /// Also takes precedence over `session`, same tier as `new`.
    term: Option<String>,
    /// When `new` or `term` is set, `create=1` tells the daemon to `fs::create_dir_all` the
    /// cwd before spawning. Only allowed when the path is under `config.workspace_root`;
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

pub(crate) async fn pty_ws(
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

    // `new=<cwd>` / `term=<cwd>` directory policy (full-freedom + confirm; the launcher
    // gates `create=1` behind a user "Create <path>?" prompt, and `origin_is_local` above
    // is the CSRF guard).
    //   - create=1 + missing → mkdir_all it (anywhere), then spawn.
    //   - create=0 + missing → refuse: don't spawn claude (or a shell) in a missing cwd.
    //   - existing dir        → spawn as-is, no mkdir, regardless of the create flag.
    // Resolved here (before on_upgrade) so a rejection closes the socket with a clear reason.
    if let Some(cwd_str) = query.new.as_deref().or(query.term.as_deref()) {
        let dir = normalize_path(&expand_tilde(cwd_str));
        let exists = dir.is_dir();
        if !exists {
            if query.create != 0 {
                if std::fs::create_dir_all(&dir).is_err() {
                    state.events.record(
                        "spawn-refused",
                        serde_json::json!({
                            "reason": "failed to create directory",
                            "cwd": dir.display().to_string(),
                        }),
                    );
                    return ws.on_upgrade(move |socket| async move {
                        let _ = close_with_reason(socket, "failed to create directory").await;
                    });
                }
            } else {
                state.events.record(
                    "spawn-refused",
                    serde_json::json!({
                        "reason": "no such directory",
                        "cwd": dir.display().to_string(),
                    }),
                );
                return ws.on_upgrade(move |socket| async move {
                    let _ = close_with_reason(socket, "no such directory").await;
                });
            }
        }
    }

    // Resume-resolution guard: a `session=<uuid>` that resolves to no on-disk session
    // (no projects_dir configured, or an unknown uuid) would fall through `pty_command`
    // to the default shell — silently launching a terminal instead of resuming Claude.
    // Refuse up front with a clear reason so the client surfaces it rather than dropping
    // the user into a bare prompt. Checked before pty_command so no shell is ever built.
    if session_resume_unresolved(&state.config, &query) {
        state.events.record(
            "resume-refused",
            serde_json::json!({
                "reason": "session could not be resolved",
                "session": query.session,
            }),
        );
        return ws.on_upgrade(move |socket| async move {
            let _ = close_with_reason(socket, "session could not be resolved").await;
        });
    }

    if let Some(pid) = codex_resume_leased(&state.config, &query) {
        let reason =
            format!("codex thread is held by a running worker (pid {pid}); wait for it to finish");
        state.events.record(
            "resume-refused",
            serde_json::json!({ "reason": reason, "session": query.session, "pid": pid }),
        );
        return ws.on_upgrade(move |socket| async move {
            let _ = close_with_reason(socket, reason).await;
        });
    }

    let command = pty_command(&state.config, &query);

    // Resume guard, mirroring the `new=` "no such directory" policy above: a session
    // records the cwd it was born in. If the user renamed/moved/deleted that project, the
    // recorded cwd is gone — spawning `claude` there silently lands in `$HOME` and
    // `--resume` then can't find the session. Refuse up front with a clear reason instead.
    if query.session.is_some() && resume_cwd_missing(&command) {
        state.events.record(
            "resume-refused",
            serde_json::json!({
                "reason": "session's project directory no longer exists",
                "session": query.session,
                "cwd": command.cwd.as_ref().map(|c| c.display().to_string()),
            }),
        );
        return ws.on_upgrade(move |socket| async move {
            let _ = close_with_reason(socket, "session's project directory no longer exists").await;
        });
    }

    // Every claude gets the turn hooks (spike 18), so a later close can tell whether
    // killing it would lose work. Added here, not in `pty_command`, because the token
    // is per spawn. A shell or codex gets none and is never killed by a close.
    let mut command = command;
    let hook_token = (command.program == "claude" && state.config.hook_port != 0).then(|| {
        let token = state.turns.new_token();
        command.args.push("--settings".to_string());
        command
            .args
            .push(crate::turns::settings_arg(state.config.hook_port, &token));
        token
    });

    let host = Arc::clone(&state.host);
    let events = Arc::clone(&state.events);
    let turns = Arc::clone(&state.turns);
    ws.on_upgrade(move |socket| async move {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        let live = match host.spawn(&command.program, &args, command.cwd.as_deref(), (80, 24)) {
            Ok(live) => live,
            Err(_) => {
                events.record(
                    "spawn-refused",
                    serde_json::json!({ "reason": "failed to spawn pty" }),
                );
                let _ = close_with_reason(socket, "failed to spawn pty").await;
                return;
            }
        };

        if let Some(token) = hook_token {
            turns.bind(live.id, token);
        }

        // A resume continues a known id: record it now so the drawer, artifact pane and
        // a later attach all know the session without waiting on a claim file.
        if let Some(uuid) = command.resumes.clone() {
            live.set_uuid(uuid.clone());
            events.record(
                "session-uuid-adopted",
                serde_json::json!({ "ptyId": live.id.to_string(), "uuid": uuid, "source": "resume" }),
            );
        }

        // For a fresh session: watch for its new JSONL, then record the uuid on the
        // LivePty and broadcast it to attached clients. The watcher holds a `Weak`
        // (mirroring the pump) so an abandoned connection can't keep the pty alive.
        if let Some((projects, dir_name)) = command.watch.clone() {
            let weak = Arc::downgrade(&live);
            let events = Arc::clone(&events);
            std::thread::spawn(move || {
                if let Some(uuid) = watch_new_session(projects, dir_name, weak.clone()) {
                    if let Some(live) = weak.upgrade() {
                        live.set_uuid(uuid.clone());
                        events.record(
                            "session-uuid-adopted",
                            serde_json::json!({
                                "ptyId": live.id.to_string(),
                                "uuid": uuid,
                                "source": "watcher",
                            }),
                        );
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
async fn close_with_reason(
    socket: WebSocket,
    reason: impl Into<axum::extract::ws::Utf8Bytes>,
) -> Result<(), axum::Error> {
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
pub(crate) async fn pty_list_route(State(state): State<AppState>) -> Response {
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
            let (uuid, last_activity, last_input) = {
                let meta = live.meta_snapshot();
                (meta.uuid, meta.last_activity, meta.last_input)
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
                "lastInput": to_iso(last_input),
            })
        })
        .collect();
    axum::Json(rows).into_response()
}

#[derive(serde::Deserialize)]
pub(crate) struct DeleteQuery {
    /// `if_safe=1`: kill only if nothing would be lost (`turns::SessionTurns::kill_safety`),
    /// else answer 409 with the reason and leave the pty running. This is what closing a
    /// tab sends; a bare DELETE is the explicit kill.
    #[serde(default)]
    if_safe: u8,
}

/// `DELETE /api/pty/:id[?if_safe=1]` — kill the child and unlist. 204 on success, 404
/// if unknown, 409 (`{"reason"}`) when `if_safe` is set and killing would lose work.
pub(crate) async fn pty_delete_route(
    AxumPath(id): AxumPath<String>,
    axum::extract::Query(query): axum::extract::Query<DeleteQuery>,
    State(state): State<AppState>,
) -> Response {
    let Ok(id) = id.parse::<host::PtyId>() else {
        return (StatusCode::NOT_FOUND, "no live pty with that id").into_response();
    };
    if query.if_safe != 0 {
        let Some(live) = state.host.get(id) else {
            return (StatusCode::NOT_FOUND, "no live pty with that id").into_response();
        };
        let (exited, child_pid) = {
            let meta = live
                .meta
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (meta.exited_at.is_some(), meta.child_pid)
        };
        // An exited child has nothing left to lose; anything else must prove it.
        let turns = Arc::clone(&state.turns);
        let verdict = if exited {
            Ok(())
        } else {
            // `ps` and a transcript read: off the async runtime.
            tokio::task::spawn_blocking(move || turns.kill_safety(id, child_pid))
                .await
                .unwrap_or(Err(crate::turns::Unsafe::NoProcessTree))
        };
        if let Err(why) = verdict {
            state.events.record(
                "pty-kept",
                serde_json::json!({ "id": id.to_string(), "reason": why.to_string() }),
            );
            return (
                StatusCode::CONFLICT,
                axum::Json(serde_json::json!({ "reason": why.to_string() })),
            )
                .into_response();
        }
    }
    match state.host.kill(id) {
        Ok(()) => {
            state.turns.forget(id);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(host::KillError::NotFound) => {
            (StatusCode::NOT_FOUND, "no live pty with that id").into_response()
        }
    }
}

/// Resolve which command a pty connection should run, spawning nothing.
/// - `new=<cwd>` → `claude` in that dir (fresh session); watch for its new JSONL.
/// - `term=<cwd>` → the configured program (a shell), never claude; no watch, no transcript.
/// - `session=<uuid>` → `claude --resume <full-uuid>` in that session's cwd; a Codex
///   thread id (no Claude session by that id) → `codex resume <id>` in the thread's cwd.
/// - none of the above → the configured default (a shell in dev, a dummy in tests).
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
            resumes: None,
        };
    }
    if let Some(cwd) = &query.term {
        return PtyCommand {
            program: cfg.program.clone(),
            args: cfg.args.clone(),
            cwd: Some(expand_tilde(cwd)),
            watch: None,
            resumes: None,
        };
    }
    if let (Some(uuid), Some(dir)) = (&query.session, &cfg.projects_dir) {
        if let Ok(stub) = eigenform_forest::resolve_stub(dir, uuid) {
            return PtyCommand {
                program: "claude".to_string(),
                args: vec!["--resume".to_string(), stub.uuid.clone()],
                cwd: Some(stub.cwd),
                watch: None,
                resumes: Some(stub.uuid),
            };
        }
    }
    if let Some(thread) = query.session.as_deref().and_then(|q| codex_thread(cfg, q)) {
        return PtyCommand {
            program: "codex".to_string(),
            args: vec!["resume".to_string(), thread.id.clone()],
            cwd: Some(thread.cwd),
            watch: None,
            resumes: Some(thread.id),
        };
    }
    PtyCommand {
        program: cfg.program.clone(),
        args: cfg.args.clone(),
        cwd: cfg.cwd.clone(),
        watch: None,
        resumes: None,
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

/// True when a `session=<uuid>` resume can't be resolved to a real on-disk session:
/// either no `projects_dir` is configured, or the uuid matches no session stub. Left
/// unguarded, such a request falls through `pty_command` to the default shell — silently
/// launching a plain terminal in place of the Claude session the user asked to resume
/// (the symptom after a reinstall drops the projects_dir config). The caller refuses with
/// a clear reason instead. Only meaningful when `query.session` is set; false otherwise,
/// so `new=`/`term=`/bare `/pty` requests are never affected.
fn session_resume_unresolved(cfg: &Config, query: &PtyQuery) -> bool {
    let Some(uuid) = query.session.as_deref() else {
        return false;
    };
    let claude = cfg
        .projects_dir
        .as_deref()
        .is_some_and(|dir| eigenform_forest::resolve_stub(dir, uuid).is_ok());
    !claude && codex_thread(cfg, uuid).is_none()
}

/// A `session=` query that names a Codex thread (and no Claude session), resolved to its
/// rail row — liveness included, so a resume can be refused while a worker holds it.
fn codex_thread(cfg: &Config, query: &str) -> Option<eigenform_codex::CodexThread> {
    let home = cfg.codex_home.as_deref()?;
    let stub = eigenform_codex::resolve(home, query).ok()?;
    Some(eigenform_codex::thread(
        home,
        &stub,
        &eigenform_codex::WriterLocks::probe(),
    ))
}

/// The pid of a live writer on the Codex thread a `session=` resume names. Codex lets
/// only one process write a thread (its writer lock); a running `codex exec` worker holds
/// it, so an interactive `codex resume` would fail inside the pty. Refuse up front with
/// the reason instead — the lock IS the lease. None for Claude sessions and idle threads.
fn codex_resume_leased(cfg: &Config, query: &PtyQuery) -> Option<u32> {
    let q = query.session.as_deref()?;
    if cfg
        .projects_dir
        .as_deref()
        .is_some_and(|dir| eigenform_forest::resolve_stub(dir, q).is_ok())
    {
        return None;
    }
    codex_thread(cfg, q)?.writer_pid
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
    /// For a resume: the session/thread id it continues. Resume keeps the id (spike 13;
    /// `codex resume` appends to the same thread), so it's recorded at spawn rather than
    /// waiting on a claim file — `codex` never writes one, and claude's is lazy.
    resumes: Option<String>,
}

// The bridge's outbound type now lives in `host` (Task 1.3), where Task 1.4's pump
// fans pty output out to subscribers as `Outbound`. Re-exported so this file's bridge
// keeps using it unchanged.
use host::Outbound;

/// Whether a request's Origin is local (or absent: a non-browser client). Shared with the
/// plan gate's routes.
pub(crate) fn origin_is_local(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return true; // no Origin → not a browser CSRF
    };
    let authority = origin
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("");
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
            serde_json::json!({"type": "pty", "id": live.id.to_string()})
                .to_string()
                .into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // 2. Subscribe + snapshot atomically.
    let (snapshot, mut rx) = live.attach();

    // 3a. Repaint. (Always send, even if empty — keeps the frame ordering uniform.)
    if sink.send(Message::Binary(snapshot.into())).await.is_err() {
        return;
    }

    // TOCTOU: if the child exited before we subscribed, the exit broadcast missed us.
    // Synthesize it to this socket only. (Done after `attach` so we never miss both.)
    if live.exited_at().is_some()
        && sink
            .send(Message::Text(r#"{"type":"exit"}"#.into()))
            .await
            .is_err()
    {
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
                            Outbound::Binary(b) => Message::Binary(b.into()),
                            Outbound::Text(t) => Message::Text(t.into()),
                        };
                        if sink.send(msg).await.is_err() {
                            break; // socket gone: stop pumping (and drop rx → detach).
                        }
                    }
                    None => break, // channel closed: nothing left to pump.
                },
                _ = keepalive.tick() => {
                    if sink.send(Message::Ping(Default::default())).await.is_err() {
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

    #[test]
    fn an_opaque_origin_is_not_local() {
        // Artifacts are served under a `sandbox` CSP, so their documents send
        // `Origin: null` — which must never pass the pty socket's guard.
        let mut h = HeaderMap::new();
        h.insert("origin", "null".parse().unwrap());
        assert!(!origin_is_local(&h));
        h.insert("origin", "http://127.0.0.1:4317".parse().unwrap());
        assert!(origin_is_local(&h));
    }

    #[test]
    fn codex_thread_resolves_to_codex_resume_and_a_held_lock_is_a_lease() {
        // A Codex thread (no Claude session by that id) resumes with `codex resume` in the
        // thread's cwd; while a worker holds its writer lock, the resume is leased away.
        let dir = tempfile::tempdir().unwrap();
        let id = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
        let day = dir.path().join("sessions/2026/10/02");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join(format!("rollout-2026-10-02T09-00-05-{id}.jsonl")),
            format!(r#"{{"timestamp":"2026-10-02T09:00:05.000Z","type":"session_meta","payload":{{"id":"{id}","cwd":"/w/repo","source":"exec"}}}}"#) + "\n",
        )
        .unwrap();
        let cfg = Config {
            program: "bash".into(),
            codex_home: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let q = PtyQuery {
            attach: None,
            session: Some("0199a1".into()),
            new: None,
            term: None,
            create: 0,
        };

        assert!(!session_resume_unresolved(&cfg, &q));
        let cmd = pty_command(&cfg, &q);
        assert_eq!(cmd.program, "codex");
        assert_eq!(cmd.args, vec!["resume".to_string(), id.to_string()]);
        assert_eq!(cmd.cwd.as_deref(), Some(std::path::Path::new("/w/repo")));
        assert_eq!(cmd.resumes.as_deref(), Some(id));
        assert_eq!(codex_resume_leased(&cfg, &q), None, "no writer → resumable");

        if std::process::Command::new("flock")
            .arg("--version")
            .output()
            .is_ok()
        {
            let locks = dir.path().join("thread-writer-locks");
            std::fs::create_dir_all(&locks).unwrap();
            let lock = locks.join(format!("{id}.lock"));
            std::fs::write(&lock, b"").unwrap();
            let mut holder = std::process::Command::new("flock")
                .args(["-x", lock.to_str().unwrap(), "sleep", "10"])
                .spawn()
                .unwrap();
            let mut leased = None;
            for _ in 0..50 {
                leased = codex_resume_leased(&cfg, &q);
                if leased.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            holder.kill().ok();
            holder.wait().ok();
            assert!(
                leased.is_some(),
                "a held writer lock must refuse the resume"
            );
        }

        // An unknown id is neither Claude nor Codex → refused, not shelled.
        let unknown = PtyQuery {
            attach: None,
            session: Some("ffff".into()),
            new: None,
            term: None,
            create: 0,
        };
        assert!(session_resume_unresolved(&cfg, &unknown));
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
            format!(r#"{{"type":"user","uuid":"u1","cwd":"/home/me/proj","sessionId":"{uuid}"}}"#)
                + "\n",
        )
        .unwrap();

        let cfg = Config {
            program: "bash".into(),
            projects_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let resumed = pty_command(
            &cfg,
            &PtyQuery {
                attach: None,
                session: Some("abcdef00".into()),
                new: None,
                term: None,
                create: 0,
            },
        );
        assert_eq!(resumed.program, "claude");
        assert_eq!(resumed.args, vec!["--resume".to_string(), uuid.to_string()]);
        // Resume keeps the id (spike 13), so the pty is bound to it at spawn.
        assert_eq!(resumed.resumes.as_deref(), Some(uuid));
        assert_eq!(
            resumed.cwd.as_deref(),
            Some(std::path::Path::new("/home/me/proj"))
        );

        // No session → the configured default, never claude.
        let default = pty_command(
            &cfg,
            &PtyQuery {
                attach: None,
                session: None,
                new: None,
                term: None,
                create: 0,
            },
        );
        assert_eq!(default.program, "bash");

        // new=<cwd> → fresh claude in that dir, with a watch target for its new JSONL.
        let fresh = pty_command(
            &cfg,
            &PtyQuery {
                attach: None,
                session: None,
                new: Some("/home/me/fresh".into()),
                term: None,
                create: 0,
            },
        );
        assert_eq!(fresh.program, "claude");
        assert!(fresh.args.is_empty());
        assert_eq!(
            fresh.cwd.as_deref(),
            Some(std::path::Path::new("/home/me/fresh"))
        );
        assert_eq!(
            fresh.watch.as_ref().map(|(_, d)| d.as_str()),
            Some("-home-me-fresh")
        );

        // term=<cwd> → the configured program (a shell), never claude, no watch.
        let term = pty_command(
            &cfg,
            &PtyQuery {
                attach: None,
                session: None,
                new: None,
                term: Some("/home/me/term".into()),
                create: 0,
            },
        );
        assert_eq!(term.program, "bash");
        assert!(term.args.is_empty());
        assert_eq!(
            term.cwd.as_deref(),
            Some(std::path::Path::new("/home/me/term"))
        );
        assert!(term.watch.is_none());

        // new= takes precedence over term= if a caller somehow sets both.
        let both = pty_command(
            &cfg,
            &PtyQuery {
                attach: None,
                session: None,
                new: Some("/home/me/fresh".into()),
                term: Some("/home/me/term".into()),
                create: 0,
            },
        );
        assert_eq!(both.program, "claude");
        assert_eq!(
            both.cwd.as_deref(),
            Some(std::path::Path::new("/home/me/fresh"))
        );
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
            projects_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        // The vanished-cwd resume still resolves to claude --resume in the recorded cwd...
        let vanished = pty_command(
            &cfg,
            &PtyQuery {
                attach: None,
                session: Some("abcdef00".into()),
                new: None,
                term: None,
                create: 0,
            },
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
            &PtyQuery {
                attach: None,
                session: Some("abcdef01".into()),
                new: None,
                term: None,
                create: 0,
            },
        );
        assert_eq!(present.cwd.as_deref(), Some(live_cwd.as_path()));
        assert!(
            !resume_cwd_missing(&present),
            "a resume whose cwd still exists must not be flagged"
        );
    }

    #[test]
    fn unresolvable_session_resume_is_flagged_not_silently_shelled() {
        // A session= resume that resolves to nothing must be refused, NOT fall through
        // to a plain shell (the "old tabs came back as terminals after a reinstall" bug).
        let dir = tempfile::tempdir().unwrap();
        let pdir = dir.path().join("-home-me-proj");
        std::fs::create_dir_all(&pdir).unwrap();
        let known = "abcdef00-0000-4000-8000-000000000000";
        std::fs::write(
            pdir.join(format!("{known}.jsonl")),
            format!(r#"{{"type":"user","uuid":"u1","cwd":"/home/me/proj","sessionId":"{known}"}}"#)
                + "\n",
        )
        .unwrap();

        let cfg = Config {
            program: "bash".into(),
            projects_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let q = |session: Option<&str>| PtyQuery {
            attach: None,
            session: session.map(str::to_string),
            new: None,
            term: None,
            create: 0,
        };

        // A uuid with no matching stub → unresolved → refuse.
        assert!(session_resume_unresolved(&cfg, &q(Some("ffffffff"))));
        // A resolvable uuid → not flagged; it resumes normally.
        assert!(!session_resume_unresolved(&cfg, &q(Some("abcdef00"))));
        // Non-resume requests are never flagged.
        assert!(!session_resume_unresolved(&cfg, &q(None)));

        // No projects_dir configured at all (e.g. a reinstall that lost the config):
        // every resume is unresolvable, so it's refused rather than shelled.
        let cfg_no_projects = Config {
            projects_dir: None,
            ..cfg
        };
        assert!(session_resume_unresolved(
            &cfg_no_projects,
            &q(Some("abcdef00"))
        ));
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
        assert_eq!(
            new_session_uuid(&other, projects, dir_name, &baseline),
            None
        );
    }
}
