//! Session host: a registry of live ptys that outlive any single WebSocket.
//!
//! `TermModel` is the heart of re-attach fidelity. The daemon owns a pty that
//! survives client disconnects; on re-attach the server must repaint a *fresh*
//! xterm. We feed every pty byte into a `vt100::Parser`, and `snapshot()` emits
//! the bytes that reconstruct the current viewport on a blank terminal.
//!
//! Spike 09 confirmed claude's TUI runs in the *alternate screen*, so a single
//! viewport grid (no scrollback ring) is sufficient — `Parser::new(rows, cols, 0)`.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Tracks the terminal modes vt100 does NOT model, so a re-attached xterm gets
/// the right screen buffer and input behaviour. Covers, all observed in claude's
/// startup (spike 09) and proven needed by Task 1.1's round-trip test:
///
/// - `?1049` alternate screen — vt100 serializes the visible grid but never
///   re-emits the alt-screen *entry*, so a replayed snapshot would paint into
///   the normal buffer. Must be re-established BEFORE contents (see `snapshot`).
/// - `?1004` focus reporting,
/// - `?2026` / `?2031` synchronized output,
/// - the Kitty keyboard protocol push `\e[>1u` (popped by `\e[<u`).
///
/// # How it works
/// We do not parse CSI generally — claude emits these exact byte strings, so a
/// literal substring scan with "last occurrence wins" (scan order == stream
/// order, so the latest h/l is the current state) is sufficient and dumb-correct.
/// A 15-byte carry buffer is prepended to each scan so a sequence straddling a
/// pty read-chunk boundary is still seen whole.
///
/// `?2026`/`?2031` are toggled h...l around each frame; a snapshot taken
/// mid-frame may replay a stale `h`, but the live stream's next `l` clears it —
/// self-correcting, so we don't special-case it.
#[derive(Default)]
pub struct ModeTracker {
    /// Last-seen on/off state of each DEC private toggle we track.
    alt_screen: Option<bool>, // ?1049
    focus: Option<bool>,      // ?1004
    sync_2026: Option<bool>,  // ?2026
    sync_2031: Option<bool>,  // ?2031
    /// Kitty keyboard push active? `?` until first seen, then last push-vs-pop.
    kitty_push: Option<bool>,
    /// Trailing bytes of the previous scan, prepended to the next so a sequence
    /// split across a chunk boundary is still matched. Capped at `CARRY`.
    carry: Vec<u8>,
}

/// Longest tracked sequence is `\x1b[?1049h` (8 bytes); 15 leaves margin so any
/// prefix split across a chunk boundary is recoverable.
const CARRY: usize = 15;

impl ModeTracker {
    /// Scan a chunk of pty output for the mode-setting sequences we track,
    /// updating last-seen state. The carry buffer handles boundary-split seqs.
    pub fn scan(&mut self, bytes: &[u8]) {
        let mut buf = std::mem::take(&mut self.carry);
        buf.extend_from_slice(bytes);

        // Each tracked toggle: last occurrence of its h/l in `buf` wins.
        update_toggle(&mut self.alt_screen, &buf, b"\x1b[?1049h", b"\x1b[?1049l");
        update_toggle(&mut self.focus, &buf, b"\x1b[?1004h", b"\x1b[?1004l");
        update_toggle(&mut self.sync_2026, &buf, b"\x1b[?2026h", b"\x1b[?2026l");
        update_toggle(&mut self.sync_2031, &buf, b"\x1b[?2031h", b"\x1b[?2031l");
        // Kitty: push `\e[>1u` on, pop `\e[<u` off; most recent wins.
        update_toggle(&mut self.kitty_push, &buf, b"\x1b[>1u", b"\x1b[<u");

        // Keep the tail for the next scan.
        let keep = buf.len().min(CARRY);
        self.carry = buf[buf.len() - keep..].to_vec();
    }

    /// Is the live session in the alternate screen? Drives `snapshot` ordering:
    /// `?1049h` must precede the contents (so the alt grid is painted into), and
    /// a normal-buffer session must NOT be switched.
    pub fn in_alt_screen(&self) -> bool {
        self.alt_screen == Some(true)
    }

    /// Bytes re-establishing alt-screen entry — emitted BEFORE contents so the
    /// repaint draws into the alt grid. Empty unless `?1049h` is the live state.
    pub fn replay_pre_contents(&self) -> Vec<u8> {
        if self.in_alt_screen() {
            b"\x1b[?1049h".to_vec()
        } else {
            Vec::new()
        }
    }

    /// Bytes re-establishing the position-independent input modes (focus, sync,
    /// Kitty push) — emitted AFTER contents for clarity. Only `h`/active states
    /// are replayed; a fresh terminal already has every mode off.
    pub fn replay_post_contents(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.focus == Some(true) {
            out.extend_from_slice(b"\x1b[?1004h");
        }
        if self.sync_2026 == Some(true) {
            out.extend_from_slice(b"\x1b[?2026h");
        }
        if self.sync_2031 == Some(true) {
            out.extend_from_slice(b"\x1b[?2031h");
        }
        if self.kitty_push == Some(true) {
            out.extend_from_slice(b"\x1b[>1u");
        }
        out
    }

    /// All tracked-mode bytes (pre + post), for unit tests of the tracker in
    /// isolation. `snapshot` interleaves these around the contents instead.
    pub fn replay(&self) -> Vec<u8> {
        let mut out = self.replay_pre_contents();
        out.extend_from_slice(&self.replay_post_contents());
        out
    }
}

/// Set `state` to reflect the last occurrence of `on` vs `off` in `buf`.
/// Leaves `state` unchanged if neither appears in this buffer.
fn update_toggle(state: &mut Option<bool>, buf: &[u8], on: &[u8], off: &[u8]) {
    let last_on = rfind(buf, on);
    let last_off = rfind(buf, off);
    match (last_on, last_off) {
        (Some(i), Some(j)) => *state = Some(i > j),
        (Some(_), None) => *state = Some(true),
        (None, Some(_)) => *state = Some(false),
        (None, None) => {}
    }
}

/// Byte-offset of the last occurrence of `needle` in `haystack`.
fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .rev()
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// A vt100-backed single-viewport terminal model.
///
/// Feed it raw pty output; `snapshot()` returns escape-sequence bytes that
/// repaint a blank xterm to match — contents plus the input modes vt100 tracks
/// (mouse, bracketed paste, application keypad/cursor) plus the modes
/// `ModeTracker` covers (alt-screen, focus, sync, Kitty).
pub struct TermModel {
    parser: vt100::Parser,
    modes: ModeTracker,
}

impl TermModel {
    pub fn new(rows: u16, cols: u16) -> Self {
        // `default()` (not the unit ctor) is deliberate: Task 1.2 gives
        // ModeTracker real fields, and this call site should not need to change.
        #[allow(clippy::default_constructed_unit_structs)]
        Self {
            parser: vt100::Parser::new(rows, cols, 0),
            modes: ModeTracker::default(),
        }
    }

    /// Process a chunk of raw pty output.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.modes.scan(bytes);
    }

    /// Resize the viewport (e.g. on client window resize).
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Bytes that repaint a fresh terminal to the current state.
    ///
    /// Ordering is load-bearing for `?1049`: alt-screen entry must come BEFORE
    /// the contents, or the repaint paints the normal screen and then switches
    /// to an empty alt grid. So: `replay_pre_contents()` (alt-screen entry, only
    /// if the live session is in alt) → `state_formatted()` (visible grid + the
    /// input modes vt100 tracks: mouse, paste, keypad) → `replay_post_contents()`
    /// (focus/sync/Kitty — position-independent, kept after contents for clarity).
    pub fn snapshot(&self) -> Vec<u8> {
        let mut out = self.modes.replay_pre_contents();
        out.extend_from_slice(&self.parser.screen().state_formatted());
        out.extend_from_slice(&self.modes.replay_post_contents());
        out
    }

    /// The visible grid as one string per row, for the state detector (Task 1.9).
    pub fn rows_text(&self) -> Vec<String> {
        self.parser
            .screen()
            .contents()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// What the daemon pushes to an attached client over the pty websocket.
///
/// Lives here (not `lib.rs`) because Task 1.4's reader-thread pump fans pty output
/// out to every attached subscriber as `Outbound` values; `lib.rs`'s current bridge
/// re-exports and keeps using it unchanged until that refactor lands.
pub enum Outbound {
    /// Raw pty output.
    Binary(Vec<u8>),
    /// A JSON control message (e.g. a new session's uuid).
    Text(String),
}

/// A registry-allocated, monotonically increasing handle to a live pty.
pub type PtyId = u64;

/// Server-owned registry of live ptys, decoupled from any WebSocket's lifetime.
///
/// The registry deals only in `Arc<LivePty>`: it allocates a unique monotonic
/// [`PtyId`], stores the entry, and serves get/list/remove. Spawning the process,
/// the reader-thread pump, and attach/detach fan-out all land in Task 1.4 — this
/// type intentionally knows nothing about them.
#[derive(Default)]
pub struct SessionHost {
    inner: Mutex<HashMap<PtyId, Arc<LivePty>>>,
    next_id: AtomicU64,
}

impl SessionHost {
    /// Allocate a fresh id, build the `LivePty` around it, store it, and return the
    /// stored `Arc` so the caller (Task 1.4's `spawn`) can downgrade to a `Weak` for
    /// the pump without a redundant `get(id).unwrap()`.
    ///
    /// The id is allocated *first* and handed to `build` so the `LivePty` can carry
    /// its own id (needed by 1.4's pump, which tags fan-out and self-removal by id).
    /// Taking a builder closure — rather than the plan's `insert(LivePty::new(..))`
    /// sketch — is what lets the entry know the id the registry chose for it.
    pub fn insert(&self, build: impl FnOnce(PtyId) -> LivePty) -> Arc<LivePty> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let live = Arc::new(build(id));
        self.inner.lock().unwrap().insert(id, Arc::clone(&live));
        live
    }

    /// Spawn `program` in a pty, register it, and start the reader-thread pump that
    /// feeds every byte into the model and fans it out to attached subscribers.
    ///
    /// `size` is `(cols, rows)` (matching [`crate::Pty::spawn`]); the model is built
    /// at that size so it agrees with the pty from byte zero.
    ///
    /// The pump holds a `Weak<LivePty>`, not a strong `Arc`: once the registry drops
    /// the entry and every attached receiver is gone, the `LivePty` (and its pty)
    /// drops, the next `upgrade()` returns `None`, and the pump exits. Attached
    /// receivers do NOT keep the `LivePty` alive (they hold only the channel), so a
    /// detached, removed pty really does die.
    pub fn spawn(
        &self,
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        size: (u16, u16),
    ) -> anyhow::Result<Arc<LivePty>> {
        let pty = crate::Pty::spawn(program, args, cwd, size)?;
        let mut reader = pty.reader()?;
        let cwd_buf = cwd.map(Path::to_path_buf);
        let live = self.insert(|id| LivePty::new(id, pty, cwd_buf, size));
        let id = live.id;
        let weak = Arc::downgrade(&live);

        // Named so a pump panic is attributable in logs. If `model.feed()` (the
        // third-party vt100 parser) panics, this thread dies — visibly, by name —
        // rather than being silently caught per-chunk; see the poison-recovery note
        // on `PtyShared` for why a dead pump still leaves the session attachable.
        std::thread::Builder::new()
            .name(format!("pty-pump-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break, // EOF or read error: stop pumping.
                        Ok(n) => n,
                    };
                    // Upgrade per-chunk: if the LivePty is gone there's no one to feed.
                    let Some(live) = weak.upgrade() else { break };
                    let chunk = &buf[..n];

                    // One critical section: feed the model AND fan out, so an attach that
                    // snapshots between these two steps cannot miss (or double) this chunk.
                    // The same `chunk` is fed and fanned out, so the two are visibly equal.
                    let mut shared = live.shared.lock().unwrap_or_else(PoisonError::into_inner);
                    shared.model.feed(chunk);
                    shared
                        .subscribers
                        .retain(|tx| tx.send(Outbound::Binary(chunk.to_vec())).is_ok());
                    drop(shared); // release before taking meta — never hold two locks here.

                    live.meta.lock().unwrap_or_else(PoisonError::into_inner).last_activity =
                        SystemTime::now();
                }
                // Task 1.5: reader EOF means the child closed the master — reap and
                // mark exited. The pump holds only a `Weak`; if the entry was killed or
                // removed mid-read, `upgrade()` fails and there is nothing to mark
                // (kill() already reaped). This exit point holds NO locks, so
                // mark_exited (which takes pty → meta, then broadcasts under shared) is
                // free to acquire them in canonical order.
                if let Some(live) = weak.upgrade() {
                    live.mark_exited();
                }
            })
            .expect("spawn pty-pump thread");

        Ok(live)
    }

    /// The live pty for `id`, if still registered.
    pub fn get(&self, id: PtyId) -> Option<Arc<LivePty>> {
        self.inner.lock().unwrap().get(&id).cloned()
    }

    /// All currently-registered live ptys (order unspecified), after a GC sweep.
    ///
    /// MUTATES the registry: it sweeps first with the default [`SWEEP_MAX_AGE`],
    /// dropping any expired entry (one whose final view has long elapsed) before
    /// returning, so a caller never sees a long-dead pty. `list` is the canonical
    /// read path (the roster, the CLI mirror, 1.7's index). Use
    /// [`SessionHost::list_unswept`] if you need the raw registry without the sweep.
    pub fn list(&self) -> Vec<Arc<LivePty>> {
        self.sweep(SWEEP_MAX_AGE);
        self.list_unswept()
    }

    /// All currently-registered live ptys without sweeping. Tests and lifecycle code
    /// that want to observe the registry exactly as it stands use this.
    pub fn list_unswept(&self) -> Vec<Arc<LivePty>> {
        self.inner.lock().unwrap().values().cloned().collect()
    }

    /// Reap entries that exited more than `max_age` ago. Live entries (never exited)
    /// and recently-exited ones (still within their "final view" window) are retained;
    /// everything older is dropped from the registry.
    ///
    /// Dropping the registry's `Arc` does not necessarily kill the pty instantly — an
    /// attached client holding its own clone keeps the `LivePty` alive until it
    /// detaches (see the `spawn` doc). For a long-dead entry that is acceptable: the
    /// child is already reaped, and the clone is a harmless husk that frees on detach.
    pub fn sweep(&self, max_age: Duration) {
        let now = SystemTime::now();
        self.inner.lock().unwrap().retain(|_, live| {
            match live.exited_at() {
                None => true, // live: never reap.
                Some(exited) => {
                    // Keep while the entry exited recently (within max_age). `elapsed`
                    // errors only on clock skew (exited in the "future"); treat that as
                    // recent (keep) rather than reaping something that just died.
                    now.duration_since(exited)
                        .map(|age| age <= max_age)
                        .unwrap_or(true)
                }
            }
        });
    }

    /// Terminate the child for `id` and remove its entry, returning `Ok(())` on a hit.
    ///
    /// Semantics for the HTTP layer (Task 1.7): an unknown id is [`KillError::NotFound`]
    /// (→ 404); a successful kill is `Ok` (→ 204). We `remove` first (taking the
    /// registry's `Arc`), then kill the child through the pty lock. Removal is what makes
    /// this idempotent against the pump's EOF path: if the child was already dying, the
    /// pump's `mark_exited` finds the entry gone via its `Weak` and does nothing.
    ///
    /// We do not `wait_child` here: `kill` (SIGKILL) closes the master, the pump's reader
    /// EOFs, and that path could reap — but the pump holds only a `Weak` to a now-removed
    /// entry, so it exits without reaping. To avoid leaving a zombie, kill then reap
    /// synchronously under the same pty lock.
    pub fn kill(&self, id: PtyId) -> Result<(), KillError> {
        let live = self.remove(id).ok_or(KillError::NotFound)?;
        let mut pty = live.pty.lock().unwrap_or_else(PoisonError::into_inner);
        // Best-effort: if the child already exited (lost the race to the EOF path), the
        // kill/wait may error harmlessly. We still reap to avoid a lingering zombie.
        let _ = pty.kill_child();
        let _ = pty.wait_child();
        Ok(())
    }

    /// Drop `id` from the registry, returning the removed entry (if any). The `Arc`
    /// (and its pty) lives until the last attached client also drops its clone.
    ///
    /// The `Option` carries the 404/204 distinction Task 1.7 needs and lets 1.5
    /// avoid double-reaping: `Some` means this call did the removal, `None` that the
    /// id was already gone.
    pub fn remove(&self, id: PtyId) -> Option<Arc<LivePty>> {
        self.inner.lock().unwrap().remove(&id)
    }

    /// Reconcile the registry against claude's pid authority, `sessions/<pid>.json`
    /// (`{"pid", "sessionId", "cwd"}` — the design doc §3 names this file claude's own
    /// recognition of its process ids). For each `<pid>.json` whose `pid` matches a
    /// registered pty's direct child pid, adopt its `sessionId` as the pty's uuid — but
    /// only if the uuid is not already set ("first writer wins"; the JSONL watcher (1.7)
    /// and this source agree in practice). This covers two cases the watcher misses or
    /// is slow on: a fresh session born before the watcher fires, and a `--resume` pty
    /// whose uuid changes from the resumed one.
    ///
    /// We spawn claude directly (no shell wrapper), so the pty child pid IS claude's pid
    /// — a direct pid equality is correct, no process-tree walk needed.
    ///
    /// Cost is a dozen small file reads; the `/api/pty` handler calls it inline. Malformed
    /// or unreadable files (and non-`.json` entries) are skipped without panicking.
    ///
    /// ## v1 stance on pid reuse
    /// We do NOT guard against a recycled OS pid: if a stale `sessions/<pid>.json` survives
    /// and the kernel later hands that same pid to one of our ptys, we would adopt the dead
    /// session's uuid. In practice the window is tiny and the JSONL watcher (which keys on
    /// the live transcript, not the pid) is the primary uuid source — reconcile is a backfill.
    /// A future guard could re-check `cwd` agreement or that the file's mtime postdates the
    /// pty's spawn; deferred until it bites.
    ///
    /// Unlike the 1.7 JSONL watcher, reconcile does NOT broadcast — it runs inline in the
    /// `/api/pty` read path, so the caller already returns the freshly-adopted uuid to the
    /// client; there is no out-of-band push to make.
    pub fn reconcile(&self, sessions_dir: &Path) {
        // Early-out before touching the filesystem: in steady state every entry already
        // has a uuid, and the `/api/pty` poll shouldn't rescan the dir each time. Snapshot
        // the registry once and reuse it for the loop (no second `list_unswept`).
        let entries: Vec<Arc<LivePty>> = self.list_unswept();
        if entries.iter().all(|live| live.uuid().is_some()) {
            return;
        }

        let dir_entries = match std::fs::read_dir(sessions_dir) {
            Ok(e) => e,
            Err(_) => return, // dir missing/unreadable: nothing to reconcile.
        };

        // pid → sessionId for every well-formed `<pid>.json` in the dir.
        let mut by_pid: HashMap<u32, String> = HashMap::new();
        for entry in dir_entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            // Local struct (forest inlines its own parse; nothing to reuse). Unknown
            // fields are ignored by serde, so extra keys never break us.
            let Ok(rec) = serde_json::from_str::<PidFile>(&text) else { continue };
            by_pid.insert(rec.pid, rec.session_id);
        }
        if by_pid.is_empty() {
            return;
        }

        for live in entries {
            if live.uuid().is_some() {
                continue; // already set: first writer wins, don't overwrite.
            }
            let pid = live.child_pid();
            if pid == 0 {
                continue; // pidless-spawn sentinel: never adopt (a stray `0.json` mustn't mass-adopt).
            }
            if let Some(sid) = by_pid.get(&pid) {
                live.set_uuid(sid.clone());
            }
        }
    }
}

/// The three fields of `sessions/<pid>.json` (claude's pid authority). Unknown keys are
/// ignored; `cwd` is present in the file but unused here (the registry already knows it).
#[derive(serde::Deserialize)]
struct PidFile {
    pid: u32,
    #[serde(rename = "sessionId")]
    session_id: String,
}

/// How long an exited pty is kept for a final view before [`SessionHost::sweep`]
/// reaps it. The design doc's "keep briefly for a final view" — ten minutes.
pub const SWEEP_MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// Below this since-last-output age, a pty counts as actively *working* — claude
/// is streaming. Above it, the grid decides (waiting selector, else idle). Design
/// doc §5 state taxonomy.
pub const WORKING_THRESHOLD: Duration = Duration::from_secs(2);

/// The state taxonomy reported per pty on `GET /api/pty` (design doc §5). Serializes
/// to lowercase strings: `"working" | "waiting" | "idle" | "exited"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyState {
    /// The child has exited; this wins over everything.
    Exited,
    /// Output within the last [`WORKING_THRESHOLD`] — claude is streaming.
    Working,
    /// Blocked on a selector widget (trust/permission/AskUserQuestion/plan): a grid
    /// of ≥2 consecutive numbered rows with exactly one `❯` (spike 08), and quiet.
    Waiting,
    /// Quiet and not blocked on a selector — idle at a prompt.
    Idle,
}

impl PtyState {
    /// The wire string for `/api/pty` — matches the route's `json!` row builder.
    pub fn as_str(self) -> &'static str {
        match self {
            PtyState::Exited => "exited",
            PtyState::Working => "working",
            PtyState::Waiting => "waiting",
            PtyState::Idle => "idle",
        }
    }
}

/// Classify a pty's state from its visible grid, time since last output, and exit flag.
///
/// Precedence: **exited → working (activity < [`WORKING_THRESHOLD`]) → waiting → idle**.
/// Working outranks waiting deliberately: the selector rows persist in the grid while
/// claude streams text above them, so a *streaming* selector reads as working; a
/// *blocked* selector emits no output, so the activity gate falls through to the grid
/// check naturally.
///
/// `rows` tolerates short/empty vecs: [`TermModel::rows_text`] elides trailing blank
/// rows (`contents().lines()`), so `rows.len()` is not the viewport height.
pub fn classify(rows: &[String], since_activity: Duration, exited: bool) -> PtyState {
    if exited {
        return PtyState::Exited;
    }
    if since_activity < WORKING_THRESHOLD {
        return PtyState::Working;
    }
    if is_selector_grid(rows) {
        return PtyState::Waiting;
    }
    PtyState::Idle
}

/// Does the grid tail hold a blocked-choice selector (spike 08)? True iff there is a
/// run of ≥2 *consecutive* rows each matching the numbered-option signature, with
/// *exactly one* `❯` among the rows of that run (the selection marker; its presence in
/// exactly one row is the strong corroborator).
fn is_selector_grid(rows: &[String]) -> bool {
    let mut run_start = 0usize;
    let mut i = 0usize;
    while i <= rows.len() {
        let in_run = i < rows.len() && is_option_row(&rows[i]);
        if !in_run {
            // A blank/non-option row breaks the run (rows must be consecutive). Check
            // the run that just ended [run_start, i).
            if i - run_start >= 2 {
                let carets: usize = rows[run_start..i]
                    .iter()
                    .filter(|r| r.chars().any(|c| c == '❯'))
                    .count();
                if carets == 1 {
                    return true;
                }
            }
            run_start = i + 1;
        }
        i += 1;
    }
    false
}

/// Match one numbered-option row: optional leading whitespace, optional `❯` (+ws),
/// one-or-more digits, `.`, whitespace, then at least one non-whitespace char. Spike
/// 08's `^\s*(❯\s*)?\d+\.\s+\S`. Hand-rolled over `chars()` (not bytes) so the
/// multibyte `❯` (U+276F) is handled correctly; no `regex` dep for one matcher.
fn is_option_row(row: &str) -> bool {
    let mut chars = row.chars().peekable();
    // optional leading whitespace
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
    // optional `❯` then optional whitespace
    if chars.peek() == Some(&'❯') {
        chars.next();
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
    }
    // one-or-more ASCII digits
    let mut saw_digit = false;
    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
        chars.next();
        saw_digit = true;
    }
    if !saw_digit {
        return false;
    }
    // literal `.`
    if chars.next() != Some('.') {
        return false;
    }
    // one-or-more whitespace
    if !chars.peek().is_some_and(|c| c.is_whitespace()) {
        return false;
    }
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
    // at least one non-whitespace label char
    chars.peek().is_some_and(|c| !c.is_whitespace())
}

/// Why a [`SessionHost::kill`] could not act. The HTTP layer (Task 1.7) maps this to
/// a status code: `NotFound` → 404, `Ok` → 204.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KillError {
    /// No registered pty has this id (already reaped, never existed, or killed twice).
    #[error("no live pty with that id")]
    NotFound,
}

/// Mutable, frequently-updated facts about a live pty, guarded separately from the
/// terminal model and the pty handle so a reader updating activity never blocks an
/// attach reading the model. Populated by Task 1.4's pump and lifecycle code.
pub struct PtyMeta {
    /// The claude session uuid, once detected (fresh sessions discover it late).
    pub uuid: Option<String>,
    /// Last time the pty produced output — drives the idle/working state detector.
    pub last_activity: SystemTime,
    /// Set when the child exits; `None` while running.
    pub exited_at: Option<SystemTime>,
    /// OS pid of the child, for `sessions/<pid>.json` reconciliation (Task 1.8).
    pub child_pid: u32,
}

/// A point-in-time copy of the mutable meta fields the `/api/pty` roster reports
/// (Task 1.7). Taken under one `meta` lock by [`LivePty::meta_snapshot`] so the row is
/// internally consistent (uuid/activity/exit all from the same instant).
pub struct MetaSnapshot {
    pub uuid: Option<String>,
    pub last_activity: SystemTime,
    pub exited_at: Option<SystemTime>,
}

/// The terminal model plus the set of attached subscribers, guarded together because
/// the pump must, in one critical section, feed a chunk into the model AND fan it out
/// to subscribers (an attach that joins between those two steps would miss the chunk
/// the snapshot didn't yet include).
///
/// # Poison recovery (deliberate)
/// Every lock on `shared` (and `meta`) in this module uses
/// `.lock().unwrap_or_else(PoisonError::into_inner)` rather than `.unwrap()`. The
/// protected state is a terminal grid plus a subscriber list. If `model.feed()` (the
/// third-party vt100 parser) panics it poisons this mutex; with `.unwrap()` every later
/// `attach`/`broadcast_text`/`resize` would cascade-panic, taking down every future
/// operation on the session. Recovering the inner value instead means the worst case is
/// a stale or imperfect snapshot — strictly better than a poisoned-mutex landmine. The
/// pump thread itself still dies on a feed panic (it is named `pty-pump-{id}`, so the
/// panic is visible in logs); recovery only guarantees the session stays attachable and
/// killable rather than panicking everyone else.
struct PtyShared {
    /// The pump feeds chunks into the model; attach reads its snapshot.
    model: TermModel,
    /// The pump pushes every chunk to these; attach/detach add/remove senders.
    ///
    /// Intentionally unbounded for v1: the std pump thread does a synchronous `send`,
    /// so there is no async point to apply backpressure. Slow-consumer eviction is
    /// deferred — a wedged consumer's queue can grow unboundedly until it disconnects.
    subscribers: Vec<UnboundedSender<Outbound>>,
}

/// One server-side live pty: the immutable identity, the mutable meta, the shared
/// terminal-model-plus-subscribers, and the pty handle for input/resize.
///
/// Three independent locks (`meta`, `shared`, `pty`) so the high-frequency paths
/// don't serialize against each other. See the lock-ordering note on [`LivePty`]'s
/// impl: anywhere two are held at once, the order is `shared` → `meta` → `pty`, and
/// today (Task 1.3) no method holds more than one at a time.
pub struct LivePty {
    pub id: PtyId,
    pub cwd: Option<PathBuf>,
    pub spawned_at: SystemTime,
    pub meta: Mutex<PtyMeta>,
    /// The pump feeds the model and fans out to subscribers; attach snapshots+subscribes.
    shared: Mutex<PtyShared>,
    /// write_input / resize go through here; Task 1.5 adds child reaping.
    pty: Mutex<crate::Pty>,
}

impl LivePty {
    /// Build a live pty around an already-spawned [`crate::Pty`]. The id comes from
    /// the registry (see [`SessionHost::insert`]). The [`TermModel`] starts at the
    /// pty's real `size` (`(cols, rows)`, matching [`crate::Pty::spawn`]) so the
    /// model and pty dimensions agree from byte zero — no first-feed-at-wrong-size.
    pub fn new(id: PtyId, pty: crate::Pty, cwd: Option<PathBuf>, size: (u16, u16)) -> Self {
        let now = SystemTime::now();
        let child_pid = pty.child_pid().unwrap_or(0);
        let (cols, rows) = size;
        Self {
            id,
            cwd,
            spawned_at: now,
            meta: Mutex::new(PtyMeta {
                uuid: None,
                last_activity: now,
                exited_at: None,
                child_pid,
            }),
            shared: Mutex::new(PtyShared {
                model: TermModel::new(rows, cols),
                subscribers: Vec::new(),
            }),
            pty: Mutex::new(pty),
        }
    }

    /// Attach a viewer: take a repaint `snapshot` of the current screen AND register
    /// a subscriber for the live stream, atomically under one `shared` lock.
    ///
    /// Atomicity is the whole point: holding `shared` across both the snapshot and the
    /// `subscribers.push` means the pump (which also needs `shared` to feed+fanout)
    /// cannot slip a chunk between them. A byte either makes it into the snapshot, or
    /// is fanned out to this new subscriber — never lost, never both.
    ///
    /// Detach is implicit: drop the returned receiver and the pump's next `send` to it
    /// fails, so `retain` removes the dead sender.
    pub fn attach(&self) -> (Vec<u8>, UnboundedReceiver<Outbound>) {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let snapshot = shared.model.snapshot();
        let (tx, rx) = mpsc::unbounded_channel();
        shared.subscribers.push(tx);
        (snapshot, rx)
    }

    /// Fan a `Text` control frame out to every attached subscriber (e.g. the session
    /// uuid once detected, or an exit notice). Same retain-on-send-ok discipline as
    /// the pump's binary fan-out; used by Tasks 1.5/1.7.
    ///
    /// Takes `shared`: do NOT call while already holding `shared` or `meta` (the 1.5
    /// EOF path must broadcast its exit notice from outside any such critical section).
    pub fn broadcast_text(&self, msg: String) {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        shared
            .subscribers
            .retain(|tx| tx.send(Outbound::Text(msg.clone())).is_ok());
    }

    /// Record the claude session uuid once the fresh-session watcher detects it.
    /// (Fresh sessions discover their uuid late, after the JSONL first appears.)
    pub fn set_uuid(&self, uuid: String) {
        self.meta
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .uuid = Some(uuid);
    }

    /// The claude session uuid, if detected yet. A cheap single-field read for
    /// reconciliation's "don't overwrite a set uuid" check.
    pub fn uuid(&self) -> Option<String> {
        self.meta
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .uuid
            .clone()
    }

    /// OS pid of the pty's direct child. We spawn claude with no shell wrapper, so the
    /// pty child pid IS claude's pid — it matches `sessions/<pid>.json` directly.
    pub fn child_pid(&self) -> u32 {
        self.meta
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .child_pid
    }

    /// A consistent copy of the mutable meta fields the `/api/pty` roster reports,
    /// taken under one lock so the row is internally consistent.
    pub fn meta_snapshot(&self) -> MetaSnapshot {
        let meta = self.meta.lock().unwrap_or_else(PoisonError::into_inner);
        MetaSnapshot {
            uuid: meta.uuid.clone(),
            last_activity: meta.last_activity,
            exited_at: meta.exited_at,
        }
    }

    /// Classify this pty's [`PtyState`] (working/waiting/idle/exited) from its grid,
    /// time since last output, and exit flag (design doc §5; the classifier is the pure
    /// [`classify`]).
    ///
    /// # Lock discipline
    /// Takes the two locks *sequentially*, never together: `shared` for the grid rows,
    /// released, then `meta` for activity/exit. Both use poison-recovery. The brief gap
    /// between the takes can't misclassify meaningfully — the only race is the pump
    /// feeding a chunk and stamping activity between them, which at worst reads a
    /// just-pre-output grid against just-post-output activity, i.e. resolves to working
    /// either way.
    pub fn state(&self) -> PtyState {
        let rows = {
            let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            shared.model.rows_text()
        }; // shared released before taking meta — never hold both.
        let (since_activity, exited) = {
            let meta = self.meta.lock().unwrap_or_else(PoisonError::into_inner);
            let since = meta
                .last_activity
                .elapsed()
                .unwrap_or(Duration::ZERO); // clock skew (activity "in the future") → treat as just-now.
            (since, meta.exited_at.is_some())
        };
        classify(&rows, since_activity, exited)
    }

    /// When the child exited, or `None` while it is still running.
    pub fn exited_at(&self) -> Option<SystemTime> {
        self.meta
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .exited_at
    }

    /// Reap the child and record its exit. Called by the pump at reader EOF (the
    /// master closed because the child died). Idempotent on the timestamp: if already
    /// marked, we leave the first exit time and skip re-broadcasting.
    ///
    /// # Lock discipline
    /// `broadcast_text` takes `shared`, so it must NOT run while we hold `shared` or
    /// `meta`. We therefore: (1) reap + stamp under `pty` then `meta` (canonical
    /// order, both released), then (2) broadcast the exit notice with no lock held.
    pub fn mark_exited(&self) {
        // (1) Reap the zombie under the pty lock, then stamp meta. Neither lock is held
        // across the broadcast below.
        self.pty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wait_child()
            .ok(); // already-reaped (e.g. kill won the race) is fine.

        {
            let mut meta = self.meta.lock().unwrap_or_else(PoisonError::into_inner);
            if meta.exited_at.is_some() {
                return; // already marked: don't re-stamp or re-broadcast.
            }
            meta.exited_at = Some(SystemTime::now());
        } // meta released here.

        // (2) No lock held: safe to take `shared` inside broadcast_text.
        self.broadcast_text(r#"{"type":"exit"}"#.to_string());
    }

    /// Send input bytes to the child's stdin (keystrokes from an attached client).
    pub fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
        self.pty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write_input(bytes)?;
        Ok(())
    }

    /// Resize the pty AND the model in lockstep, so the next attach's snapshot
    /// repaints at the new geometry. `size` is `(cols, rows)`.
    ///
    /// Lock order: this is the one method that needs two locks (`shared` for the
    /// model, `pty` for the master). The canonical order is `shared` → `meta` → `pty`,
    /// so we take `shared` first and hold it across the pty resize.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        self.pty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resize(cols, rows)?;
        shared.model.resize(rows, cols);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A `LivePty` wrapping a cheap real process, for registry-semantics tests.
    ///
    /// Per Task 1.3: the registry stores `Arc<LivePty>` and `LivePty` holds a real
    /// `Mutex<crate::Pty>` (no Option/generic testability seam — that would be
    /// theater). So the "pure" map tests still spawn a trivial child. `cat` with no
    /// args blocks reading its stdin and exits on EOF: cheap, no output, and the pty
    /// keeps the master writer alive so it never wedges. The registry never touches
    /// the process; these tests exercise only insert/get/list/remove/id allocation.
    fn live_pty(host: &SessionHost, cwd: Option<PathBuf>) -> PtyId {
        let pty = crate::Pty::spawn("cat", &[], cwd.as_deref(), (80, 24)).expect("spawn cat");
        host.insert(|id| LivePty::new(id, pty, cwd, (80, 24))).id
    }

    #[test]
    fn register_list_get_remove() {
        let host = SessionHost::default();
        let id = live_pty(&host, Some("/tmp".into()));
        assert_eq!(host.list().len(), 1);
        assert!(host.get(id).is_some());
        host.remove(id);
        assert!(host.get(id).is_none());
        assert_eq!(host.list().len(), 0);
    }

    #[test]
    fn ids_are_unique_and_monotonic() {
        let host = SessionHost::default();
        let a = live_pty(&host, None);
        let b = live_pty(&host, None);
        assert!(b > a, "ids must be monotonic: {b} > {a}");
        assert_ne!(a, b, "ids must be unique");
    }

    #[test]
    fn live_pty_carries_its_id_and_cwd() {
        let host = SessionHost::default();
        let id = live_pty(&host, Some("/tmp".into()));
        let lp = host.get(id).expect("present");
        assert_eq!(lp.id, id);
        assert_eq!(lp.cwd.as_deref(), Some(Path::new("/tmp")));
    }

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
    fn snapshot_round_trips_an_alt_screen_viewports_contents() {
        // The full re-attach claim: a fresh terminal fed snapshot() repaints the
        // same visible grid AND lands in the alternate screen. ModeTracker (Task
        // 1.2) replays the `?1049h` vt100's state_formatted() omits, and
        // snapshot() emits it BEFORE the contents so the alt grid is the one
        // painted into.
        let mut live = TermModel::new(24, 80);
        live.feed(b"\x1b[?1049h\x1b[2J\x1b[H  \xe2\x9d\xaf 1. Yes");
        assert!(live.parser.screen().alternate_screen(), "live is in alt screen");

        let mut fresh = TermModel::new(24, 80);
        fresh.feed(&live.snapshot());

        assert!(
            fresh.parser.screen().alternate_screen(),
            "fresh must land in the alternate screen"
        );
        assert!(fresh.rows_text()[0].contains("❯ 1. Yes"));
    }

    #[test]
    fn normal_buffer_session_is_not_switched_to_alt() {
        // claude's trust-dialog phase (spike 08) runs in the NORMAL buffer.
        // snapshot() must not emit ?1049h or the fresh terminal flips to an
        // empty alt screen.
        let mut live = TermModel::new(24, 80);
        live.feed(b"Do you trust this folder?");
        assert!(!live.parser.screen().alternate_screen());

        let snap = String::from_utf8_lossy(&live.snapshot()).to_string();
        assert!(!snap.contains("\x1b[?1049h"), "no alt-screen entry for normal buffer");

        let mut fresh = TermModel::new(24, 80);
        fresh.feed(&live.snapshot());
        assert!(!fresh.parser.screen().alternate_screen());
        assert!(fresh.rows_text()[0].contains("Do you trust this folder?"));
    }

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

    #[test]
    fn sequence_split_three_ways_byte_at_a_time_is_still_seen() {
        // The 2-way split test proves the carry survives one boundary; this proves
        // it survives several. Feed `\x1b[?1004h` one byte per scan() call (8 scans,
        // 7 boundaries) — the carry must accumulate the whole sequence.
        let mut t = ModeTracker::default();
        for b in b"\x1b[?1004h" {
            t.scan(&[*b]);
        }
        assert!(String::from_utf8_lossy(&t.replay()).contains("\x1b[?1004h"));
    }

    #[test]
    fn tracks_alt_screen_and_sync_modes() {
        let mut t = ModeTracker::default();
        t.scan(b"\x1b[?1049h\x1b[?2026h\x1b[?2031h");
        let r = String::from_utf8_lossy(&t.replay()).to_string();
        assert!(r.contains("\x1b[?1049h"), "alt screen tracked");
        assert!(r.contains("\x1b[?2026h"), "sync output tracked");
        assert!(r.contains("\x1b[?2031h"), "sync output 2031 tracked");
    }

    #[test]
    fn alt_screen_exit_clears_replay() {
        let mut t = ModeTracker::default();
        t.scan(b"\x1b[?1049h");
        t.scan(b"\x1b[?1049l");
        assert!(!String::from_utf8_lossy(&t.replay()).contains("1049h"));
    }

    #[test]
    fn kitty_last_pop_wins_within_one_chunk() {
        // A chunk containing push then pop: pop is later, so it wins.
        let mut t = ModeTracker::default();
        t.scan(b"\x1b[>1u stuff \x1b[<u");
        assert!(!String::from_utf8_lossy(&t.replay()).contains(">1u"));
    }

    #[test]
    fn kitty_last_push_wins_within_one_chunk() {
        let mut t = ModeTracker::default();
        t.scan(b"\x1b[<u stuff \x1b[>1u");
        assert!(String::from_utf8_lossy(&t.replay()).contains("\x1b[>1u"));
    }

    #[tokio::test]
    async fn reconcile_adopts_uuid_from_matching_pid_file() {
        let host = SessionHost::default();
        let pty = host.spawn("sh", &["-c", "sleep 30"], None, (80, 24)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let pid = pty.child_pid();
        std::fs::write(
            dir.path().join(format!("{pid}.json")),
            format!(r#"{{"pid":{pid},"sessionId":"abc-123","cwd":"/tmp"}}"#),
        )
        .unwrap();
        host.reconcile(dir.path());
        assert_eq!(pty.uuid(), Some("abc-123".into()));
        host.kill(pty.id).ok(); // hygiene: no stray sleep 30.
    }

    #[tokio::test]
    async fn reconcile_does_not_overwrite_a_set_uuid() {
        let host = SessionHost::default();
        let pty = host.spawn("sh", &["-c", "sleep 30"], None, (80, 24)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let pid = pty.child_pid();
        pty.set_uuid("already-set".into()); // e.g. the JSONL watcher won the race.
        std::fs::write(
            dir.path().join(format!("{pid}.json")),
            format!(r#"{{"pid":{pid},"sessionId":"abc-123","cwd":"/tmp"}}"#),
        )
        .unwrap();
        host.reconcile(dir.path());
        assert_eq!(pty.uuid(), Some("already-set".into()), "first writer wins");
        host.kill(pty.id).ok();
    }

    #[tokio::test]
    async fn reconcile_ignores_pid_file_with_no_matching_entry() {
        let host = SessionHost::default();
        let pty = host.spawn("sh", &["-c", "sleep 30"], None, (80, 24)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        // A pid that does not match the pty's child (off-by-one is enough).
        let other = pty.child_pid().wrapping_add(1);
        std::fs::write(
            dir.path().join(format!("{other}.json")),
            format!(r#"{{"pid":{other},"sessionId":"abc-123","cwd":"/tmp"}}"#),
        )
        .unwrap();
        host.reconcile(dir.path());
        assert_eq!(pty.uuid(), None, "no matching entry: nothing adopted");
        host.kill(pty.id).ok();
    }

    #[tokio::test]
    async fn reconcile_skips_malformed_json_without_panicking() {
        let host = SessionHost::default();
        let pty = host.spawn("sh", &["-c", "sleep 30"], None, (80, 24)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let pid = pty.child_pid();
        // Right filename, garbage contents — must be skipped, not panic.
        std::fs::write(dir.path().join(format!("{pid}.json")), b"not json {").unwrap();
        // A non-`.json` file and a stray directory should also be tolerated.
        std::fs::write(dir.path().join("README.txt"), b"hello").unwrap();
        std::fs::create_dir(dir.path().join("subdir.json")).unwrap();
        host.reconcile(dir.path());
        assert_eq!(pty.uuid(), None);
        host.kill(pty.id).ok();
    }

    /// A tiny `Duration` for the activity-age axis of `classify`.
    fn age_secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn selector_grid_means_waiting() {
        let rows = vec![
            "Do you trust this folder?".into(),
            " ❯ 1. Yes, I trust this folder".into(),
            "   2. No, exit".into(),
        ];
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
    fn exited_wins() {
        assert_eq!(classify(&[], age_secs(1), true), PtyState::Exited);
    }

    #[test]
    fn numbered_list_without_caret_is_not_waiting() {
        let rows = vec!["1. apples".into(), "2. oranges".into()];
        assert_ne!(classify(&rows, age_secs(10), false), PtyState::Waiting);
    }

    #[test]
    fn two_carets_is_not_waiting() {
        // Exactly one ❯ is the corroborator (spike 08): two selected rows is not
        // a coherent selector, so it falls through to idle, not waiting.
        let rows = vec!["❯ 1. one".into(), "❯ 2. two".into()];
        assert_ne!(classify(&rows, age_secs(10), false), PtyState::Waiting);
    }

    #[test]
    fn selector_rows_split_by_blank_row_are_not_consecutive() {
        // Spike 08 says ≥2 *consecutive* numbered rows. A blank row between them
        // breaks the run, so this is not a waiting selector.
        let rows = vec![
            " ❯ 1. Yes".into(),
            "".into(),
            "   2. No".into(),
        ];
        assert_ne!(classify(&rows, age_secs(10), false), PtyState::Waiting);
    }

    #[test]
    fn working_outranks_waiting_while_streaming() {
        // The selector grid persists while claude streams above it; recent output
        // (activity < 2s) means working even if the grid is present. A *blocked*
        // selector produces no output, so the activity gate falls through to waiting.
        let rows = vec![" ❯ 1. Yes".into(), "   2. No".into()];
        assert_eq!(classify(&rows, age_secs(1), false), PtyState::Working);
    }

    #[test]
    fn classify_at_exactly_threshold_is_not_working() {
        // The working gate is `since_activity < WORKING_THRESHOLD` (strict `<`).
        // At exactly 2s the gate does NOT fire; the grid is checked instead.
        // An empty grid → Idle (not Working).
        assert_eq!(
            classify(&[], WORKING_THRESHOLD, false),
            PtyState::Idle,
            "at exactly the 2s boundary the working gate must NOT apply"
        );
        // At exactly the boundary with a selector grid the working gate also does
        // NOT fire, so the grid check runs and yields Waiting (not Working).
        let selector_rows = vec![" ❯ 1. Yes".to_string(), "   2. No".to_string()];
        assert_eq!(classify(&selector_rows, WORKING_THRESHOLD, false), PtyState::Waiting,
            "at exactly the boundary with a selector grid: Waiting, not Working");
    }

    #[test]
    fn exited_wins_over_selector_grid() {
        // exited=true must trump the grid check: even a valid selector grid with
        // recent silence should report Exited, not Waiting.
        let selector_rows = vec![
            " ❯ 1. Yes, I trust this folder".into(),
            "   2. No, exit".into(),
        ];
        assert_eq!(
            classify(&selector_rows, WORKING_THRESHOLD, true),
            PtyState::Exited,
            "exited must win over a selector grid"
        );
    }

    #[test]
    fn rows_text_exposes_the_grid_for_the_state_detector() {
        let mut m = TermModel::new(24, 80);
        m.feed(b"\x1b[?1049h\x1b[2J\x1b[H  \xe2\x9d\xaf 1. Yes\r\n    2. No");
        let rows = m.rows_text();
        assert!(rows[0].contains("❯ 1. Yes"));
        assert!(rows[1].contains("2. No"));
    }
}
