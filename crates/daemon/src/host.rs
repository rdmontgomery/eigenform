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
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::sync::mpsc::UnboundedSender;

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

    /// The live pty for `id`, if still registered.
    pub fn get(&self, id: PtyId) -> Option<Arc<LivePty>> {
        self.inner.lock().unwrap().get(&id).cloned()
    }

    /// All currently-registered live ptys (order unspecified).
    pub fn list(&self) -> Vec<Arc<LivePty>> {
        self.inner.lock().unwrap().values().cloned().collect()
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

/// The terminal model plus the set of attached subscribers, guarded together because
/// the pump must, in one critical section, feed a chunk into the model AND fan it out
/// to subscribers (an attach that joins between those two steps would miss the chunk
/// the snapshot didn't yet include).
struct PtyShared {
    // Task 1.4: the pump feeds chunks into the model; attach reads its snapshot.
    #[allow(dead_code)]
    model: TermModel,
    // Task 1.4: the pump pushes every chunk to these; attach/detach add/remove senders.
    #[allow(dead_code)]
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
    #[allow(dead_code)] // Task 1.4: the pump feeds the model and fans out to subscribers.
    shared: Mutex<PtyShared>,
    #[allow(dead_code)] // Task 1.4+: write_input / resize / child reaping.
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

    #[test]
    fn rows_text_exposes_the_grid_for_the_state_detector() {
        let mut m = TermModel::new(24, 80);
        m.feed(b"\x1b[?1049h\x1b[2J\x1b[H  \xe2\x9d\xaf 1. Yes\r\n    2. No");
        let rows = m.rows_text();
        assert!(rows[0].contains("❯ 1. Yes"));
        assert!(rows[1].contains("2. No"));
    }
}
