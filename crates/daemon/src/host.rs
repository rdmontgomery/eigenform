//! Session host: server-side terminal models that outlive a WebSocket.
//!
//! `TermModel` is the heart of re-attach fidelity. The daemon owns a pty that
//! survives client disconnects; on re-attach the server must repaint a *fresh*
//! xterm. We feed every pty byte into a `vt100::Parser`, and `snapshot()` emits
//! the bytes that reconstruct the current viewport on a blank terminal.
//!
//! Spike 09 confirmed claude's TUI runs in the *alternate screen*, so a single
//! viewport grid (no scrollback ring) is sufficient — `Parser::new(rows, cols, 0)`.

/// Tracks the terminal modes vt100 does NOT model (focus ?1004, sync output
/// ?2026/?2031, Kitty keyboard push). Real logic is Task 1.2 — stub for now.
///
/// `Default` is derived (not the unit ctor) because Task 1.2 grows real fields;
/// callers use `ModeTracker::default()` so that change stays internal.
#[derive(Default)]
pub struct ModeTracker;

impl ModeTracker {
    /// Scan a chunk of pty output for mode-setting escape sequences. No-op stub.
    pub fn scan(&mut self, _bytes: &[u8]) {}
    /// Bytes that re-establish the tracked modes on a fresh terminal. Empty stub.
    pub fn replay(&self) -> Vec<u8> {
        Vec::new()
    }
}

/// A vt100-backed single-viewport terminal model.
///
/// Feed it raw pty output; `snapshot()` returns escape-sequence bytes that
/// repaint a blank xterm to match — contents plus the input modes vt100 tracks
/// (mouse, bracketed paste, application keypad/cursor) plus, eventually, the
/// modes `ModeTracker` covers.
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

    /// Bytes that repaint a fresh terminal to the current state: screen contents
    /// and the input modes vt100 tracks, then any `ModeTracker`-tracked modes.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut out = self.parser.screen().state_formatted();
        out.extend_from_slice(&self.modes.replay());
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

#[cfg(test)]
mod tests {
    use super::*;

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
        // The re-attach claim, contents half: a fresh terminal fed snapshot()
        // repaints the same visible grid a running claude TUI is showing.
        //
        // NOTE (Task 1.2): vt100's state_formatted() serializes the *visible*
        // grid (the alt grid, when in alt screen) + input modes, but it does
        // NOT emit `?1049h` itself — alt-screen is a DEC private mode vt100
        // doesn't replay. So `fresh` here shows the right glyphs but stays on
        // the normal screen. Re-establishing `?1049h` (and focus/sync/kitty)
        // is ModeTracker's job. This test asserts only what Task 1.1 owns.
        let mut live = TermModel::new(24, 80);
        live.feed(b"\x1b[?1049h\x1b[2J\x1b[H  \xe2\x9d\xaf 1. Yes");
        assert!(live.parser.screen().alternate_screen(), "live is in alt screen");

        let mut fresh = TermModel::new(24, 80);
        fresh.feed(&live.snapshot());

        assert!(fresh.rows_text()[0].contains("❯ 1. Yes"));
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
