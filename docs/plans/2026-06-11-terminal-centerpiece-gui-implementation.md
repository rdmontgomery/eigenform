# Terminal-Centerpiece GUI — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the terminal-first GUI from `docs/plans/2026-06-11-eigen-terminal-centerpiece-gui-design.md`: a daemon-hosted persistent-pty session host, a new browser app beside woland (sidebar roster + terminal tabs), a launcher, and a transcript drawer.

**Architecture:** The daemon grows a `SessionHost` — a registry of live ptys decoupled from WebSocket lifetime, each holding a server-side `vt100` terminal model so re-attach is one clean repaint (spike 09: alt-screen, single viewport). A new vanilla-TS app (`webterm/`, served at `/term`) lifts woland's proven xterm↔`/pty` wiring and builds sidebar/tabs/launcher/drawer around it. Woland is untouched.

**Tech Stack:** Rust (axum 0.7, portable-pty 0.8, tokio, **vt100 0.16** — new dep), TypeScript (esbuild, @xterm/xterm + fit addon, `node --test`), no frameworks.

---

## Model recommendations (per phase)

The user asked which model should code each phase. Recommendations and reasoning:

| Phase | Model | Why |
|---|---|---|
| 0 — Scaffolding | **Sonnet** | Mechanical: a Config field, a `nest_service`, copied build config. No design freedom. |
| 1 — Session host | **Opus** | The load-bearing phase. Shared mutable state across std threads and tokio tasks (registry, fan-out, snapshot-vs-feed atomicity), lifecycle races (EOF vs detach vs kill), and escape sequences that straddle read-chunk boundaries. Failures here are *silent* — lost output, a deadlock under two viewers, a corrupt repaint — and everything later sits on top of it. |
| 2 — Shell + terminal | **Sonnet** | Mostly transcription: lift `connectPty`/resize/stdin framing from `web/src/main.ts`, build DOM/CSS shell, pure-function data layers with tests. The genuinely hard part (input fidelity) is settled by spike 09 + Phase 1; verification is manual. *Escalation note:* if attach/reload reconciliation misbehaves, the bug is almost certainly in Phase 1 — reopen that with Opus rather than patching the client. |
| 3 — Launcher | **Sonnet** | Small, fully specified: `merge_candidates`/`immediate_subdirs` already exist and are tested; the fuzzy scorer has a written design; the endpoint is a thin wrapper. |
| 4 — Transcript drawer | **Sonnet** | Patterns all exist in woland (fetch JSON, SSE re-render, auto-scroll-unless-scrolled, fork POST). The one backend task (tool detail in `session_json`) is additive TDD in a well-tested crate. |

General principle applied: Opus where there is concurrency, protocol design under partial failure, or cross-cutting state whose bugs don't show up in unit tests; Sonnet where this plan plus existing code reduce the task to faithful execution.

---

## Standing constraints (read before starting)

- **Never invoke `claude` yourself.** Live-claude tests are gated behind `EIGEN_ALLOW_LIVE_CLAUDE=1` and `--ignored`; manual fidelity checks are performed *by the user*. All automated pty tests spawn `sh`/`cat`/`printf`, never `claude`.
- **Woland stays intact.** No edits under `web/` except where a task says so explicitly (there are none). New frontend code goes in `webterm/`.
- **CLI mirror rule** (project standing rule): every new renderable view gets a CLI command — `eigen ptys` (Task 1.10) and `eigen candidates` (Task 3.2) are part of the plan, not optional.
- **Commit prefixes** follow repo convention: `daemon:`, `cli:`, `projects:`, and `term:` for the new app (codename TBD — `webterm`/`term:` are placeholders; renaming later is one find-replace).
- Run `cargo test -p eigen-daemon` (or `-p eigen-projects`, etc.) after every green step; `cd webterm && npm test` for frontend logic.

---

## Phase 0 — Scaffolding (Sonnet)

### Task 0.1: Serve a second app at `/term`

**Files:**
- Modify: `crates/daemon/src/lib.rs` (Config ~line 26, `app()` ~line 45)
- Modify: `crates/eigen-cli/src/main.rs` (daemon subcommand)
- Test: `crates/daemon/tests/term_app.rs`

**Step 1: Write the failing test**

```rust
// crates/daemon/tests/term_app.rs
use eigen_daemon::Config;

#[tokio::test]
async fn serves_term_app_at_term_prefix_and_woland_at_root() {
    let woland = tempfile::tempdir().unwrap();
    std::fs::write(woland.path().join("index.html"), "WOLAND").unwrap();
    let term = tempfile::tempdir().unwrap();
    std::fs::write(term.path().join("index.html"), "TERM").unwrap();

    let config = Config {
        web_dir: Some(woland.path().to_path_buf()),
        term_dir: Some(term.path().to_path_buf()),
        ..Config::default()
    };
    // follow the listener/request pattern used in tests/session_route.rs
    let body = get(&config, "/term/").await;
    assert!(body.contains("TERM"));
    let body = get(&config, "/").await;
    assert!(body.contains("WOLAND"));
}
```

(If `Config` has no `Default` impl, construct it field-by-field exactly as `tests/session_route.rs` does — copy that fixture helper.)

**Step 2: Run it, watch it fail**

Run: `cargo test -p eigen-daemon --test term_app`
Expected: compile error — `term_dir` field doesn't exist.

**Step 3: Implement**

In `Config`, add:

```rust
pub term_dir: Option<PathBuf>,
```

In `app()`, *before* the existing `web_dir` fallback block:

```rust
if let Some(term_dir) = &config.term_dir {
    let index = term_dir.join("index.html");
    router = router.nest_service(
        "/term",
        tower_http::services::ServeDir::new(term_dir)
            .fallback(tower_http::services::ServeFile::new(index)),
    );
}
```

Fix every `Config { .. }` construction site (CLI, tests) — add `term_dir: None`.

**Step 4: Run tests** — `cargo test -p eigen-daemon` → all PASS.

**Step 5: Wire the CLI flag**

In `crates/eigen-cli/src/main.rs`, the `Daemon` subcommand: add `--term <dir>` (`Option<PathBuf>`), pass into `Config`. Mirror how `--web` is wired.

**Step 6: Commit**

```bash
git add crates/daemon crates/eigen-cli
git commit -m "daemon: serve a second app at /term (term_dir config)"
```

### Task 0.2: Scaffold `webterm/`

**Files:**
- Create: `webterm/package.json`, `webterm/tsconfig.json`, `webterm/index.html`, `webterm/src/main.ts`, `webterm/src/style.css`

**Step 1: Copy woland's build setup.** Copy `web/package.json` → `webterm/package.json`; keep the same scripts verbatim (`esbuild src/main.ts --bundle --outdir=dist --format=esm --minify`, watch variant, `tsc --noEmit`, `node --test`) and the same deps (`@xterm/xterm`, `@xterm/addon-fit`, esbuild, typescript). Change `"name"` to `"eigen-term"`. Copy `web/tsconfig.json` unchanged.

**Step 2: Minimal index.html** — one `<div id="app">`, `<script type="module" src="dist/main.js">`, `<link rel="stylesheet" href="dist/main.css">`. **Important:** asset URLs must be relative (`dist/main.js`, not `/dist/main.js`) because the app is served under `/term/`.

**Step 3: Minimal main.ts** — `document.getElementById("app")!.textContent = "term";` plus `import "./style.css";`.

**Step 4: Verify build.** `cd webterm && npm install && npm run build` → `dist/main.js` exists. Then `eigen daemon --term webterm <existing flags>` and confirm `curl localhost:<port>/term/` returns the page. (No type errors: `npm run check` if woland has that script — match its name.)

**Step 5: Commit** — `term: scaffold webterm app beside woland (esbuild, served at /term)`.

---

## Phase 1 — Session host (Opus)

All new backend code goes in a new module `crates/daemon/src/host.rs` (`mod host;` + re-exports in `lib.rs`) — don't grow the 800-line `lib.rs` further. The existing `Pty` struct (`lib.rs:736`, spawn/reader/write_input/resize) is reused as-is.

### Task 1.1: Add vt100; `TermModel`

**Files:**
- Modify: `crates/daemon/Cargo.toml` (+ workspace `Cargo.toml` if deps are centralized): `vt100 = "0.16"`
- Create: `crates/daemon/src/host.rs`
- Test: inline `#[cfg(test)]` in `host.rs` (pure logic — follow the daemon's inline-test convention for non-IO units)

**Step 1: Read `docs.rs/vt100/0.16` for `Parser` and `Screen`.** The load-bearing API (verified): `Parser::new(rows, cols, scrollback)`, `parser.process(&[u8])`, `parser.screen_mut().set_size(rows, cols)` (resize lives on `Screen`, not `Parser` — discovered in Task 1.1), `screen.state_formatted() -> Vec<u8>` (contents **plus** input modes: mouse, bracketed paste, application keypad/cursor), `screen.contents() -> String`, `screen.alternate_screen() -> bool`.

**Step 2: Write failing tests**

```rust
#[test]
fn snapshot_reproduces_plain_output() {
    let mut m = TermModel::new(24, 80);
    m.feed(b"hello from the pty\r\n");
    let snap = String::from_utf8_lossy(&m.snapshot()).to_string();
    assert!(snap.contains("hello from the pty"));
}

#[test]
fn snapshot_carries_input_modes_set_by_the_app() {
    let mut m = TermModel::new(24, 80);
    // claude's startup mode-set (spike 09): mouse SGR + bracketed paste
    m.feed(b"\x1b[?1049h\x1b[?1006h\x1b[?1002h\x1b[?2004h");
    let snap = m.snapshot();
    let s = String::from_utf8_lossy(&snap);
    assert!(s.contains("\x1b[?2004h"), "bracketed paste must replay");
    assert!(s.contains("\x1b[?1006h"), "SGR mouse must replay");
}

#[test]
fn rows_text_exposes_the_grid_for_the_state_detector() {
    let mut m = TermModel::new(24, 80);
    m.feed(b"\x1b[?1049h\x1b[2J\x1b[H  \xe2\x9d\xaf 1. Yes\r\n    2. No");
    let rows = m.rows_text();
    assert!(rows[0].contains("❯ 1. Yes"));
    assert!(rows[1].contains("2. No"));
}
```

**Step 3: Run** `cargo test -p eigen-daemon host::` → FAIL (TermModel undefined).

**Step 4: Implement**

```rust
pub struct TermModel {
    parser: vt100::Parser,
    modes: ModeTracker, // Task 1.2; stub as a no-op unit struct for now
}

impl TermModel {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self { parser: vt100::Parser::new(rows, cols, 0), modes: ModeTracker::default() }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.modes.scan(bytes);
    }
    pub fn resize(&mut self, rows: u16, cols: u16) { self.parser.screen_mut().set_size(rows, cols); }
    pub fn snapshot(&self) -> Vec<u8> {
        let mut out = self.parser.screen().state_formatted();
        out.extend_from_slice(&self.modes.replay());
        out
    }
    pub fn rows_text(&self) -> Vec<String> {
        self.parser.screen().contents().lines().map(str::to_owned).collect()
    }
}
```

**Step 5: Pass, then commit** — `daemon: TermModel — vt100-backed single-viewport terminal model`.

### Task 1.2: `ModeTracker` — the modes vt100 doesn't carry

vt100's `state_formatted()` covers mouse/paste/keypad. It does **not** track: the alternate screen (`?1049`), focus reporting (`?1004`), synchronized output (`?2026`, `?2031`), or the Kitty keyboard push (`\e[>1u` / pop `\e[<u`) — all observed in spike 09. Without replaying these, a re-attached xterm has wrong input behavior (and, for `?1049`, paints into the wrong screen buffer).

**Step 1: Failing tests** (same inline module):

```rust
#[test]
fn tracks_focus_and_kitty_and_replays_them() {
    let mut t = ModeTracker::default();
    t.scan(b"junk\x1b[?1004h more \x1b[>1u junk");
    let r = String::from_utf8_lossy(&t.replay()).to_string();
    assert!(r.contains("\x1b[?1004h"));
    assert!(r.contains("\x1b[>1u"));
}

#[test]
fn later_reset_wins() {
    let mut t = ModeTracker::default();
    t.scan(b"\x1b[?1004h\x1b[>1u");
    t.scan(b"\x1b[?1004l\x1b[<u");
    let r = t.replay();
    assert!(!String::from_utf8_lossy(&r).contains("1004h"));
    assert!(!String::from_utf8_lossy(&r).contains(">1u"));
}

#[test]
fn sequence_split_across_chunk_boundary_is_still_seen() {
    let mut t = ModeTracker::default();
    t.scan(b"\x1b[?10");
    t.scan(b"04h");
    assert!(String::from_utf8_lossy(&t.replay()).contains("\x1b[?1004h"));
}
```

**Step 2: Implement.** Keep it dumb and correct: a carry buffer of the last 15 bytes prepended to each scan, then literal substring search (last occurrence wins) for each tracked pair: `\x1b[?1004h|l`, `\x1b[?2026h|l`, `\x1b[?2031h|l`; for Kitty, track the most recent of `\x1b[>1u` (push) vs `\x1b[<u` (pop). `replay()` emits the `h`-state sequences and the Kitty push if active. The chunk-boundary test is the whole reason the carry buffer exists — don't skip it.

**Step 3: Pass, commit** — `daemon: ModeTracker — replay focus/sync/kitty modes vt100 omits`.

### Task 1.3: `SessionHost` registry core

**Step 1: Failing tests** (inline in `host.rs`; no pty yet — pure registry semantics):

```rust
#[test]
fn register_list_get_remove() {
    let host = SessionHost::default();
    let id = host.insert(LivePtyMeta::new(Some("/tmp".into())));
    assert_eq!(host.list().len(), 1);
    assert!(host.get(id).is_some());
    host.remove(id);
    assert!(host.get(id).is_none());
}

#[test]
fn ids_are_unique_and_monotonic() {
    let host = SessionHost::default();
    let a = host.insert(LivePtyMeta::new(None));
    let b = host.insert(LivePtyMeta::new(None));
    assert!(b > a);
}
```

**Step 2: Implement.**

```rust
pub type PtyId = u64;

#[derive(Default)]
pub struct SessionHost {
    inner: Mutex<HashMap<PtyId, Arc<LivePty>>>,
    next_id: AtomicU64,
}
```

Shape `LivePty` now even though spawn comes next task:

```rust
pub struct LivePty {
    pub id: PtyId,
    pub cwd: Option<PathBuf>,
    pub spawned_at: SystemTime,
    pub meta: Mutex<PtyMeta>,            // uuid: Option<String>, last_activity: SystemTime, exited_at: Option<SystemTime>, child_pid: u32
    shared: Mutex<PtyShared>,            // model: TermModel, subscribers: Vec<UnboundedSender<Outbound>>
    pty: Mutex<crate::Pty>,              // write_input / resize / child handle
}
```

Reuse the existing `Outbound { Binary(Vec<u8>), Text(String) }` enum (move it from `lib.rs:620` into `host.rs`, re-export).

**Step 3: Pass, commit** — `daemon: SessionHost registry skeleton`.

### Task 1.4: Spawn-and-register, output pump, attach/detach fan-out

The heart of the phase. Invariant to protect: **snapshot and subscribe happen under the same lock as feed** — otherwise bytes arriving between "snapshot taken" and "subscriber registered" are lost or doubled.

**Files:**
- Modify: `crates/daemon/src/host.rs`
- Test: `crates/daemon/tests/host.rs` (integration — spawns real `sh`, mirrors `tests/pty.rs` style)

**Step 1: Failing tests**

```rust
// crates/daemon/tests/host.rs
use eigen_daemon::host::{SessionHost, Outbound};
use std::time::Duration;

#[tokio::test]
async fn attach_after_output_repaints_then_streams() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "printf 'EARLY\\n'; sleep 30"], None, (24, 80)).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await; // let EARLY land in the model

    let (snapshot, mut rx) = pty.attach();
    assert!(String::from_utf8_lossy(&snapshot).contains("EARLY"));

    pty.write_input(b"").unwrap(); // attach is live: writer works
    drop(rx); // detach
    assert!(host.get(pty.id).is_some(), "detach must not kill the pty");
}

#[tokio::test]
async fn two_viewers_both_receive_output() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &[], None, (24, 80)).unwrap();
    let (_, mut rx1) = pty.attach();
    let (_, mut rx2) = pty.attach();
    pty.write_input(b"printf BOTHSEE\n").unwrap();
    let saw = |rx: &mut _| /* drain with timeout, collect Binary bytes, assert contains BOTHSEE */;
    // implement a small `drain_until` helper with tokio::time::timeout
}

#[tokio::test]
async fn output_while_detached_lands_in_the_next_snapshot() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &[], None, (24, 80)).unwrap();
    pty.write_input(b"printf WHILE_AWAY\n").unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (snapshot, _rx) = pty.attach(); // first-ever attach: still must contain it
    assert!(String::from_utf8_lossy(&snapshot).contains("WHILE_AWAY"));
}
```

**Step 2: Implement**

```rust
impl SessionHost {
    pub fn spawn(&self, program: &str, args: &[&str], cwd: Option<&Path>, size: (u16, u16))
        -> anyhow::Result<Arc<LivePty>>
    {
        let pty = crate::Pty::spawn(program, args, cwd, size)?;
        let mut reader = pty.reader()?;
        let live = Arc::new(LivePty { /* ... */ });
        self.inner.lock().unwrap().insert(live.id, live.clone());
        let weak = Arc::downgrade(&live);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let n = match reader.read(&mut buf) { Ok(0) | Err(_) => break, Ok(n) => n };
                let Some(live) = weak.upgrade() else { break };
                let mut shared = live.shared.lock().unwrap();
                shared.model.feed(&buf[..n]);
                shared.subscribers.retain(|tx| tx.send(Outbound::Binary(buf[..n].to_vec())).is_ok());
                live.meta.lock().unwrap().last_activity = SystemTime::now();
            }
            if let Some(live) = weak.upgrade() { live.mark_exited(); } // Task 1.5
        });
        Ok(live)
    }
}

impl LivePty {
    pub fn attach(&self) -> (Vec<u8>, mpsc::UnboundedReceiver<Outbound>) {
        let mut shared = self.shared.lock().unwrap(); // one lock: snapshot + subscribe atomically
        let snapshot = shared.model.snapshot();
        let (tx, rx) = mpsc::unbounded_channel();
        shared.subscribers.push(tx);
        (snapshot, rx)
    }
    pub fn broadcast_text(&self, msg: String) { /* fan Text frames out the same way */ }
}
```

Detach is implicit: dropping the receiver makes the next `send` fail and `retain` drops the sender. Use `tokio::sync::mpsc::UnboundedSender` precisely because `send` is sync and callable from the std reader thread.

`resize` goes through `self.pty.lock()` **and** `shared.model.resize(rows, cols)` — keep model and pty dimensions in lockstep.

**Step 3: Run** `cargo test -p eigen-daemon --test host` → PASS. Also re-run the full crate suite.

**Step 4: Commit** — `daemon: SessionHost spawn/attach/detach with atomic snapshot fan-out`.

### Task 1.5: Lifecycle — reap, kill, GC sweep

**Step 1: Failing tests** (same file):

```rust
#[tokio::test]
async fn child_exit_marks_exited_but_keeps_the_entry_briefly() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "true"], None, (24, 80)).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pty.exited_at().is_some());
    assert!(host.get(pty.id).is_some(), "kept for a final view");
}

#[tokio::test]
async fn sweep_reaps_long_dead_entries() {
    // spawn `sh -c true`, wait for exit, then host.sweep(Duration::ZERO) → entry gone
}

#[tokio::test]
async fn kill_terminates_the_child_and_removes_the_entry() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "sleep 30"], None, (24, 80)).unwrap();
    host.kill(pty.id).unwrap();
    assert!(host.get(pty.id).is_none());
}
```

**Step 2: Implement.** `mark_exited()` (called from the reader thread on EOF): `pty.lock().child.wait()` to reap the zombie, set `meta.exited_at = Some(now)`, and `broadcast_text(r#"{"type":"exit"}"#)` so attached clients can show "exited". `SessionHost::sweep(max_age)` retains live entries and recently-exited ones; call it at the top of `list()` with a 10-minute default. `kill(id)`: `child.kill()`, then remove.

**Step 3: Pass, commit** — `daemon: pty lifecycle — reap on EOF, explicit kill, GC sweep`.

### Task 1.6: `AppState` refactor (mechanical, but do it carefully)

`SessionHost` must be shared by route handlers; `Config` is pure config and shouldn't own runtime state.

**Step 1:** In `lib.rs`:

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub host: Arc<host::SessionHost>,
}
```

`app(config)` builds `AppState` and the router takes `.with_state(state)`. Change every handler signature `State(cfg): State<Arc<Config>>` → `State(state): State<AppState>` and `cfg.` → `state.config.` (about a dozen handlers — the route table in `app()` lists them all).

**Step 2:** `cargo test -p eigen-daemon` → everything still green (this task adds no behavior; the test suite is the safety net). Fix any test that constructs the router directly.

**Step 3: Commit** — `daemon: AppState carries Config + SessionHost`.

### Task 1.7: HTTP surface — attach protocol + registry endpoints

**Files:**
- Modify: `crates/daemon/src/lib.rs` (`pty_ws` line ~498, `pty_command` ~518, `bridge` ~656, `PtyQuery` ~490, route table)
- Test: extend `crates/daemon/tests/ws.rs` + new cases in `tests/host_routes.rs`

**Behavior spec:**
- `GET /pty?new=<cwd>` / `?session=<uuid>` / bare — resolve the command via the existing `pty_command` (unchanged), but spawn through `host.spawn(...)` and **first send a text frame** `{"type":"pty","id":"<id>"}`, then the snapshot as one binary frame, then stream.
- `GET /pty?attach=<ptyId>` — no spawn; `host.get(id)` (404-equivalent close if missing), send the pty-id text frame, snapshot, stream.
- Socket close ⇒ receiver drops ⇒ detach. The pty lives on.
- Incoming `Control::Stdin`/`Control::Resize` (existing enum, line ~649) route to `live.write_input` / `live.resize`.
- `GET /api/pty` → `[{ "id", "cwd", "uuid", "state", "spawnedAt", "lastActivity" }]` (state is `"unknown"` until Task 1.9).
- `DELETE /api/pty/:id` → `host.kill(id)`, 204; 404 if absent.

**Step 1: Failing WS integration test** (tokio-tungstenite, mirror `tests/ws.rs` fixtures; spawn the daemon with `program: "sh"` so bare `/pty` runs a shell):

```rust
#[tokio::test]
async fn reattach_repaints_prior_output() {
    // connect /pty (bare) → read text frame {"type":"pty","id":N}
    // send {"type":"stdin","data":"printf REPAINT_ME\n"}
    // close socket. connect /pty?attach=N → first binary frame(s) contain "REPAINT_ME"
}

#[tokio::test]
async fn closing_the_socket_leaves_the_pty_listed() {
    // connect bare /pty, grab id, close. GET /api/pty → contains id.
}

#[tokio::test]
async fn delete_kills_and_unlists() { /* DELETE /api/pty/:id → 204; GET /api/pty → gone */ }
```

**Step 2: Implement.** Rewrite `bridge` as `attach_socket(socket: WebSocket, live: Arc<LivePty>)`: one task pumps `rx` → socket (Binary/Text), the socket read loop dispatches Control messages. Keep the CSRF Origin check exactly as-is. The old per-socket spawn path dies; bare `/pty` now also registers (uniform model).

**Step 3:** The session-uuid watcher (`watch_new_session`, lines ~690): instead of sending into the socket's channel, it now sets `meta.uuid` on the `LivePty` and calls `broadcast_text({"type":"session","uuid":...})`. The inline test at `lib.rs:932` (`detects_a_new_session_jsonl_under_the_project_dir`) must keep passing.

**Step 4:** Full suite green: `cargo test -p eigen-daemon`.

**Step 5: Commit** — `daemon: attach protocol — /pty?attach, GET /api/pty, DELETE /api/pty/:id`.

### Task 1.8: Reconcile registry against `sessions/*.json`

The design names `~/.claude/sessions/<pid>.json` (`{"pid", "sessionId", "cwd"}`) the **pid authority**. Reconciliation: for each registry entry, if a sessions-file pid matches the pty's direct child pid, adopt its `sessionId` as the uuid (covers sessions born before the JSONL watcher fires, and `--resume` ptys whose uuid changes).

**Step 1: Failing test** (tempdir `sessions/` with a crafted `<pid>.json` matching a spawned `sh` pty's pid):

```rust
#[tokio::test]
async fn reconcile_adopts_uuid_from_matching_pid_file() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "sleep 30"], None, (24, 80)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pid = pty.child_pid();
    std::fs::write(dir.path().join(format!("{pid}.json")),
        format!(r#"{{"pid":{pid},"sessionId":"abc-123","cwd":"/tmp"}}"#)).unwrap();
    host.reconcile(dir.path());
    assert_eq!(pty.uuid(), Some("abc-123".into()));
}
```

**Step 2: Implement** `SessionHost::reconcile(sessions_dir)` — read each `<pid>.json` (reuse the parsing in `eigen_forest::live_forest` if exported; otherwise a local serde struct), match pids, set uuid if unset. Call it from the `GET /api/pty` handler (cheap: a dozen small files).

**Step 3: Pass, commit** — `daemon: reconcile pty registry against sessions/<pid>.json authority`.

### Task 1.9: State taxonomy (spike-08 detector, server-side)

**Step 1: Failing tests** (pure function, inline tests):

```rust
#[test]
fn selector_grid_means_waiting() {
    let rows = vec!["Do you trust this folder?".into(),
                    " ❯ 1. Yes, I trust this folder".into(),
                    "   2. No, exit".into()];
    assert_eq!(classify(&rows, age_secs(10), false), PtyState::Waiting);
}
#[test]
fn recent_output_means_working() {
    assert_eq!(classify(&[], age_secs(1), false), PtyState::Working);
}
#[test]
fn quiet_prompt_means_idle() {
    assert_eq!(classify(&["> ".into()], age_secs(60), false), PtyState::Idle);
}
#[test]
fn exited_wins() { assert_eq!(classify(&[], age_secs(1), true), PtyState::Exited); }
#[test]
fn numbered_list_without_caret_is_not_waiting() {
    let rows = vec!["1. apples".into(), "2. oranges".into()];
    assert_ne!(classify(&rows, age_secs(10), false), PtyState::Waiting);
}
```

**Step 2: Implement** `classify(rows: &[String], since_activity: Duration, exited: bool) -> PtyState`:
precedence **exited → working (activity < 2s) → waiting → idle**. Working outranks waiting because the selector rows persist while claude streams above them; a *blocked* selector produces no output, so the activity gate falls through naturally. Waiting detector per spike 08: ≥2 consecutive rows matching `^\s*(❯\s*)?\d+\.\s+\S` with exactly one `❯` among them. Use the `regex` crate if already in the workspace; otherwise hand-roll (the pattern is trivial).

**Step 3:** Wire into `GET /api/pty` (replace `"unknown"`); states serialize as `"working" | "waiting" | "idle" | "exited"`.

**Step 4: Pass, commit** — `daemon: pty state taxonomy — working/waiting/idle/exited from grid + activity`.

### Task 1.10: CLI mirror — `eigen ptys`

**Files:** `crates/eigen-cli/src/main.rs`, `crates/eigen-cli/Cargo.toml` (+ `ureq = "2"`)

**Step 1:** Add `Cmd::Ptys { #[arg(long, default_value_t = <daemon default port>)] port: u16 }` following the `Forest` pattern (main.rs:211). Handler: `ureq::get(&format!("http://127.0.0.1:{port}/api/pty"))`, parse the JSON array, print one row per pty: `id  state  uuid-or-—  cwd  age`. Daemon unreachable → exit nonzero with `daemon not running on :<port>` (friendly, not a panic).

**Step 2:** Manual check: run the daemon, open a bare `/pty` socket via the browser or `tests/ws.rs` style script, run `eigen ptys`, see the row. (No automated test — it's a thin HTTP client; the endpoint is already tested.)

**Step 3: Commit** — `cli: eigen ptys — CLI mirror of GET /api/pty`.

**Phase 1 done when:** all daemon tests green; a bare-`/pty` WebSocket can disconnect and re-attach with a clean repaint; `eigen ptys` lists it with a sensible state.

---

## Phase 2 — Shell + terminal (Sonnet)

Vanilla TS in `webterm/src/`. Pure logic in its own modules with `node --test` tests; DOM code thin and untested (woland's posture).

### Task 2.1: `pty.ts` — lifted socket wiring

**Files:** Create `webterm/src/pty.ts`, `webterm/src/types.ts`

**Step 1:** Read `web/src/main.ts:204–212` (Terminal construction), `370–412` (`sendResize`, `connectPty`, `sendPrompt`). Lift into a module — same framing, plus the new pty-id text frame:

```ts
// types.ts
export type PtyState = "working" | "waiting" | "idle" | "exited";
export interface PtyInfo { id: string; cwd: string; uuid?: string; state: PtyState; spawnedAt: string; lastActivity: string; }
export interface ForestItem { uuid: string; title: string | null; cwd: string; recency: string; live: boolean; state: string; spark: number[]; }

// pty.ts
export interface PtyEvents {
  onPtyId(id: string): void;
  onSessionUuid(uuid: string): void;
  onExit(): void;
}
export function connectPty(query: string, term: Terminal, ev: PtyEvents): { dispose(): void } {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const sock = new WebSocket(`${proto}://${location.host}/pty${query}`);
  sock.binaryType = "arraybuffer";
  sock.onmessage = (m) => {
    if (typeof m.data === "string") {
      const msg = JSON.parse(m.data);
      if (msg.type === "pty") ev.onPtyId(msg.id);
      else if (msg.type === "session") ev.onSessionUuid(msg.uuid);
      else if (msg.type === "exit") ev.onExit();
    } else term.write(new Uint8Array(m.data));
  };
  const data = term.onData((d) => sock.readyState === WebSocket.OPEN &&
    sock.send(JSON.stringify({ type: "stdin", data: d })));
  const resize = term.onResize(({ cols, rows }) => sock.readyState === WebSocket.OPEN &&
    sock.send(JSON.stringify({ type: "resize", cols, rows })));
  return { dispose() { data.dispose(); resize.dispose(); sock.close(); } };
}
```

Terminal options: copy woland's font stack and `cursorBlink: true`; theme minimal for now. Send an initial resize as soon as the socket opens (the daemon spawned at 80×24; the fit addon knows the real size — this triggers claude's self-healing repaint per spike 09).

**Step 2:** `npm run build` + tsc clean. **Commit** — `term: pty.ts — socket wiring lifted from woland + attach protocol frames`.

### Task 2.2: Roster data layer (pure, tested)

**Files:** Create `webterm/src/roster.ts`, `webterm/src/roster.test.ts`

**Step 1: Failing tests** (`node --test`, import pattern from `web/src/cooling.test.ts`):

```ts
test("live ptys sort above recent forest rows", ...);
test("a forest row with the same uuid as a pty merges into one row (live wins)", ...);
test("label degrades: aiTitle -> cwd basename -> first-prompt snippet -> 'new session'", () => {
  assert.equal(deriveLabel({ aiTitle: "Fix the parser", cwd: "/h/p/eigen" }), "Fix the parser");
  assert.equal(deriveLabel({ cwd: "/h/p/eigen" }), "eigen");
  assert.equal(deriveLabel({ firstPrompt: "please look at..." }), "please look at...");
  assert.equal(deriveLabel({}), "new session");
});
test("user override beats every derived label", ...);
```

**Step 2: Implement** `buildRoster(ptys: PtyInfo[], forest: ForestItem[], overrides: Record<string, string>): RosterRow[]` — merge by uuid (registry rows win, keep `ptyId`), live first, then recency; `deriveLabel` as specified in design §5. Overrides keyed by session uuid, persisted in `localStorage` by the caller.

**Step 3:** `npm test` green. **Commit** — `term: roster merge + label derivation (pure, tested)`.

### Task 2.3: Shell UI — sidebar + tabs + terminal host

**Files:** Create `webterm/src/shell.ts`; modify `webterm/src/main.ts`, `webterm/src/style.css`

**Step 1: Layout.** Left rail (sidebar) + tab strip + terminal host filling the rest. CSS grid; one `Terminal` instance **per open tab** (kept alive while the tab is open, hidden via `display:none` when inactive — re-fitting on tab switch with `fit.fit()`).

**Step 2: Sidebar.** Fetch `GET /api/pty` + `GET /api/forest` (lift `loadForest` from `web/src/data.ts:201`), `buildRoster`, render rows: state badge (●, colored per state), label, dim cwd-basename chip, relative recency. Poll `/api/pty` every 3s and re-render badges (cheap; SSE later if it itches). Double-click label → inline rename → `localStorage` override.

**Step 3: Open flows.** Click row: if `ptyId` → `connectPty(`?attach=${id}`)`; else if uuid-on-disk → `connectPty(`?session=${uuid}`)`. Tab header: label, badge, cwd chip, close ✕ (detach = `dispose()` the socket, drop the tab — pty lives), and an explicit "kill" in a row context-menu or header overflow that confirms (`confirm()` is fine for v1) then `DELETE /api/pty/:id`.

**Step 4: Reload reconciliation.** Persist open-tab descriptors (`{ptyId?, uuid?, label}`) in `localStorage`; on boot, fetch `/api/pty`, reopen tabs whose ptyId still exists (attach), fall back to `?session=` for ones that died with a uuid, drop the rest.

**Step 5:** Build + manual smoke against the daemon (bare `sh` ptys: open two tabs, type in one, close, re-open — repaint correct). **Commit** — `term: shell — sidebar roster, tabs, attach/detach/kill flows`.

### Task 2.4: Input-fidelity acceptance (user-run)

**Step 1:** Add a checklist to the PR/commit message and ask **the user** to run it against a live claude (you must not launch claude yourself):

- [ ] arrow keys / Enter / Esc in a selector widget
- [ ] multi-line paste (bracketed paste — no accidental submit)
- [ ] mouse wheel + click where claude uses mouse tracking
- [ ] shift+enter & modified keys (Kitty keyboard path)
- [ ] no flicker during streaming (sync output)
- [ ] detach mid-stream → re-attach: clean single repaint, input still correct
- [ ] reload the browser tab: same

**Step 2:** Fix what the user reports. Client-side fixes live in `pty.ts`/xterm options; repaint/mode bugs are Phase 1 (`ModeTracker`/`TermModel`) — escalate those to Opus per the model table.

**Phase 2 done when:** the checklist passes against live claude, and a working session left in the background shows up as a live sidebar row you can open onto an instant repaint.

---

## Phase 3 — Launcher (Sonnet)

### Task 3.1: `GET /api/candidates`

**Files:** Modify `crates/daemon/src/lib.rs` (route + Config), `crates/daemon/Cargo.toml` (+ `eigen-projects = { path = "../projects" }`); test `crates/daemon/tests/candidates_route.rs`

**Step 1:** Add `pub workspace_root: Option<PathBuf>` to `Config` (CLI flag `--workspace`, default `~/projects` resolved at CLI level, soft — see design §5).

**Step 2: Failing test:** tempdir workspace with subdirs `alpha/`, `beta/`; tempdir projects-dir with one session JSONL whose cwd is `beta`; GET `/api/candidates` → `[{path: ".../beta", recent: true}, {path: ".../alpha", recent: false}]` (recents first, deduped).

**Step 3: Implement:** recents = cwds from `eigen_forest::list(...)` in recency order, deduped; subdirs = `eigen_projects::immediate_subdirs(root)` (root missing → empty, not 500); merge via `eigen_projects::merge_candidates(&recents, &subdirs)`.

**Step 4: Commit** — `daemon: GET /api/candidates — launcher list over merge_candidates`.

### Task 3.2: CLI mirror — `eigen candidates`

Same pattern as Task 1.10 but computed **locally** (no daemon needed — it's all disk): recents from `eigen_forest::list`, subdirs from `eigen_projects`, print `path  [recent]` rows. One integration test in `crates/eigen-cli` if the CLI has a test harness; otherwise verify by running it. **Commit** — `cli: eigen candidates — CLI mirror of the launcher list`.

### Task 3.3: Fuzzy scorer (pure, tested)

**Files:** Create `webterm/src/fuzzy.ts`, `webterm/src/fuzzy.test.ts`

Per `docs/plans/2026-06-09-woland-blocked-input-and-fuzzy-picker-design.md` (Subsystem A): subsequence match; rank by basename hits > contiguous runs > word-boundary starts; recent breaks ties.

**Step 1: Failing tests** — `"eig"` matches `/home/r/projects/eigen` (basename) above `/home/r/eigtmp/old` (non-basename); non-subsequence → excluded; empty query → input order preserved.

**Step 2: Implement** `rankCandidates(query: string, candidates: {path: string; recent: boolean}[]): Candidate[]`.

**Step 3: Commit** — `term: fuzzy candidate ranking (pure, tested)`.

### Task 3.4: Picker UI + mkdir-then-launch

**Files:** Create `webterm/src/picker.ts`; modify `webterm/src/shell.ts`; daemon: `pty_command` + test

**Step 1: Daemon side first (TDD):** extend `PtyQuery` with `create: Option<bool>`; in the `new=<cwd>` arm, if `create` and the path is **under `workspace_root`** (reject otherwise — 400), `fs::create_dir_all` before spawning. Test: `pty_command`/handler creates the dir under a tempdir root; refuses `/etc/x`.

**Step 2: Picker UI:** `+ New session` sidebar entry → input + custom dropdown (no `<datalist>`); ↑/↓ highlight, Enter launches highlighted (`/pty?new=<path>`), free-text path allowed, Esc closes; a typed bare name (no `/`) resolves to `<workspace_root>/<name>` and launches with `&create=1`. Rows: bold basename, dim parent, faint "recent" tag. Candidates from `GET /api/candidates`, ranked by `rankCandidates` as you type.

**Step 3:** Manual smoke (spawn `sh` via daemon default; for claude launches, the user drives). **Commit** — `term: launcher — fuzzy picker, mkdir-under-workspace`.

**Phase 3 done when:** `eigen candidates` and the picker show the same list (CLI-mirror rule), and typing a fresh name creates the dir and lands in a new tab.

---

## Phase 4 — Transcript drawer (Sonnet)

### Task 4.1: Tool detail in `session_json`

Design §6 wants drill-down to **full input params and result output**; today's `Exchange.tool.detail` carries preview lines only (`web/src/data.ts:6–22`).

**Step 1:** Read `eigen_render::session_json` (crates/render) and the `Tool`-building code path. Write a failing test in the render crate's existing test style: a fixture JSONL with one tool_use (input `{"file_path": "/x", "old_string": "a", ...}`) and its tool_result → the JSON gains `tool.input` (the raw JSON value) and `tool.output` (string, truncated to 50 KB with a `truncated: true` flag when cut).

**Step 2:** Implement; keep the existing fields untouched (woland consumes them — zero-risk rule). Run **both** `cargo test -p eigen-render` and `-p eigen-daemon`.

**Step 3: Commit** — `render: session_json carries full tool input/output for drill-down`.

### Task 4.2: Drawer — turns, SSE, auto-scroll

**Files:** Create `webterm/src/drawer.ts`, `webterm/src/turns.ts`, `webterm/src/turns.test.ts`; modify `shell.ts`

**Step 1: Pure grouping, tested:** `groupTurns(exchanges: Exchange[]): TurnGroup[]` — consecutive exchanges between user turns collapse into one assistant group (design §6 "multi-message assistant turn collapses as one group"). Tests: 1 user + 3 tool rounds + 1 user → 2 groups; leaf exchange flagged.

**Step 2: Drawer shell:** toggle button in the tab header slides a panel over the terminal's right side (terminal stays primary; CSS transform, no layout reflow of xterm — do **not** resize the pty when the drawer opens). Data: `fetchSession(uuid)` + SSE `/api/watch/:uuid` → re-fetch on `change` (lift the pattern from `web/src/data.ts:104` and `main.ts:473`). Auto-scroll to bottom unless the user has scrolled up (woland's rule).

**Step 3:** Collapsible turn groups: header = turn number + first line of the user text; click to fold/unfold. Render assistant text as plain text v1 (markdown is backlog).

**Step 4: Commit** — `term: transcript drawer — grouped turns, SSE live, auto-scroll`.

### Task 4.3: Tool drill-down

Collapsed: tool kind + `arg` one-liner (already in the JSON). Expanded: pretty-printed `tool.input` JSON + `tool.output` in a `<pre>`, with the existing `detail.lines` color classes if present. No new tests beyond a `turns.test.ts` case asserting tool exchanges carry through grouping. **Commit** — `term: tool drill-down in the drawer`.

### Task 4.4: Per-turn edit-and-fork → new tab

**Step 1:** Fork affordance on each **user** turn header: opens the turn's text in a textarea (prefilled), confirm → `POST /api/session/:uuid/fork` with `{turn, text}` (lift `forkSession` from `web/src/data.ts:117–130`; the endpoint exists and is tested, `lib.rs:142`).

**Step 2:** On `{uuid}` response: open a new tab via `connectPty("?session=" + uuid)` (fork-on-disk → resume), add to roster optimistically, keep the source tab untouched (copy-on-fork).

**Step 3:** Manual verification with the user driving claude. **Commit** — `term: per-turn edit-and-fork opens the fork as a new tab`.

**Phase 4 done when:** you can navigate to the first user turn of a long session in the drawer while the terminal streams, expand a tool call to its full input/output, and fork from turn N into a new live tab.

---

## Out of scope (recorded in design §8 — do not build)

User-turn spine/jump-nav (first post-v1 feature), overlay-on-terminal niceties, multiple workspaces, cross-session search, daemon-restart recovery beyond `--resume`, markdown rendering in the drawer, SSE for `/api/pty` (poll is fine), drawer-open pty resize.

### Backlog: picker bare-name workspace_root exposure (3.4)

The picker's `resolvePick` derives the workspace root by using the parent of the
first non-recent candidate (Task 3.4). This heuristic breaks when all candidates
are recent (no subdirs) — bare-name input falls through to `null`, requiring the
user to type an absolute path.

Cleaner fix: expose `workspace_root` in the GET /api/candidates response as
`{ root: string | null, candidates: Candidate[] }`. Deferred because it changes
the 3.1 route contract + CLI mirror + tests. Do before a public release.
