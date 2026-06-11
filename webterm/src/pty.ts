import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

// ── Terminal factory ─────────────────────────────────────────────────────────

export interface TermHandle {
  term: Terminal;
  fit: FitAddon;
}

/**
 * Create a pre-configured Terminal + FitAddon pair.
 * Styling (theme, font size) is minimal here — Phase 2.3 owns the full palette.
 * The caller is responsible for `term.open(element)` and the initial `fit.fit()`.
 */
export function newTerminal(): TermHandle {
  const term = new Terminal({
    fontFamily: '"IBM Plex Mono", ui-monospace, Menlo, Consolas, monospace',
    fontSize: 12,
    cursorBlink: true,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  return { term, fit };
}

// ── Socket wiring ────────────────────────────────────────────────────────────

export interface PtyEvents {
  /** Fires when the daemon sends {"type":"pty","id":"<n>"} — the assigned pty id. */
  onPtyId(id: string): void;
  /** Fires when the daemon sends {"type":"session","uuid":"<uuid>"} — session born. */
  onSessionUuid(uuid: string): void;
  /** Fires when the daemon sends {"type":"exit"} — the child process exited. */
  onExit(): void;
  /**
   * Fires when the WebSocket closes, regardless of cause.
   * `reason` is the close frame reason string — notably "no live pty with that id"
   * for an attach-miss POLICY close (socket opens, daemon then immediately closes it).
   * An empty string means a normal / unannounced close.
   */
  onClose(reason: string): void;
}

export interface PtyHandle {
  dispose(): void;
}

/**
 * Open a WebSocket to `/pty<query>`, wire it to `term`, and surface protocol
 * events via `ev`.
 *
 * `query` is appended verbatim to the path (e.g. `"?attach=42"` or `""`).
 *
 * Wire protocol (daemon side, Task 1.7):
 *   • First text frame: `{"type":"pty","id":"<n>"}` — pty id announcement.
 *   • One binary frame: snapshot of the pty's current viewport.
 *   • Subsequent binary frames: live pty output.
 *   • Text frame `{"type":"session","uuid":"<uuid>"}`: session born.
 *   • Text frame `{"type":"exit"}`: child exited.
 *
 * Client → daemon:
 *   • `{"type":"stdin","data":"<chars>"}` — keystrokes from the user.
 *   • `{"type":"resize","cols":<n>,"rows":<n>}` — viewport resize.
 *
 * The initial resize is sent on socket open so the daemon's 80×24 default is
 * corrected immediately; this triggers claude's self-healing repaint (spike 09).
 *
 * Attach-miss: the daemon UPGRADES the socket (HTTP 101 succeeds) then closes
 * with a POLICY close frame + reason "no live pty with that id". This is handled
 * via `sock.onclose` and surfaces as `ev.onClose(event.reason)`.
 *
 * `dispose()` removes the term listeners and closes the socket (if still open).
 * It is safe to call before the socket has finished opening.
 */
export function connectPty(
  query: string,
  term: Terminal,
  ev: PtyEvents,
): PtyHandle {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const sock = new WebSocket(`${proto}://${location.host}/pty${query}`);
  sock.binaryType = "arraybuffer";

  let onData: { dispose(): void } | null = null;
  let onResize: { dispose(): void } | null = null;

  sock.onopen = () => {
    // Send current terminal dimensions immediately so the daemon's 80×24 default
    // is corrected before any output arrives (spike 09 repaint trigger).
    if (sock.readyState === WebSocket.OPEN) {
      sock.send(
        JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }),
      );
    }

    onData = term.onData((d) => {
      if (sock.readyState === WebSocket.OPEN) {
        sock.send(JSON.stringify({ type: "stdin", data: d }));
      }
    });

    onResize = term.onResize(({ cols, rows }) => {
      if (sock.readyState === WebSocket.OPEN) {
        sock.send(JSON.stringify({ type: "resize", cols, rows }));
      }
    });
  };

  sock.onmessage = (event) => {
    if (event.data instanceof ArrayBuffer) {
      term.write(new Uint8Array(event.data));
      return;
    }
    // String frame — JSON control message.
    try {
      const msg = JSON.parse(event.data as string) as Record<string, unknown>;
      if (msg.type === "pty" && typeof msg.id === "string") {
        ev.onPtyId(msg.id);
      } else if (msg.type === "session" && typeof msg.uuid === "string") {
        ev.onSessionUuid(msg.uuid);
      } else if (msg.type === "exit") {
        ev.onExit();
      }
    } catch {
      // Non-JSON text frame — ignore.
    }
  };

  sock.onclose = (event) => {
    ev.onClose(event.reason ?? "");
  };

  return {
    dispose() {
      onData?.dispose();
      onResize?.dispose();
      onData = null;
      onResize = null;
      if (
        sock.readyState === WebSocket.CONNECTING ||
        sock.readyState === WebSocket.OPEN
      ) {
        // Clear onclose before closing so dispose doesn't trigger ev.onClose.
        sock.onclose = null;
        sock.close();
      }
    },
  };
}
