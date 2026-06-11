# 09 — claude TUI uses the alternate-screen buffer (no terminal scrollback)

**Claim:** Once past the trust prompt, claude's main interactive TUI switches into the
alternate-screen buffer (`\e[?1049h`). In the alternate screen, no traditional terminal
scrollback accumulates — claude owns its viewport and repaints in place, implementing its
own conversation scrolling internally. Therefore the live *terminal pane* cannot, on its
own, "scroll back to the first user input"; durable navigable history must come from the
JSONL transcript (the drawer), and a re-attaching client only needs the **current
alt-screen grid** repainted, not a byte-log of scrollback.
**Status:** CONFIRMED (alt-screen enter, directly observed at startup in a trusted dir)
**claude version:** 2.1.170 (Claude Code)
**Date:** 2026-06-11

## Procedure

Zero tokens — we boot the TUI and send no prompt. `notes/spikes/09-tui-altscreen-scrollback.py`
forks `claude` (no args) under a pty in the already-trusted eigen project dir, drains raw
output for 4s, SIGKILLs (never types anything), and scans for the DEC private-mode escapes
that decide buffer behavior (`\e[?1049h/47h/1047h` alt-screen, `\e[?25l/h` cursor, DECSTBM
scroll region, mouse/paste modes).

```
python3 notes/spikes/09-tui-altscreen-scrollback.py
```

## Result

1990 bytes captured. Decisive signal present exactly once at startup:

```
\e7\e[r\e8\e[?25h\e[?1049h\e[2J\e[H\e[<u\e[>1u\e[>4;2m\e[?1000h\e[?1002h\e[?1003h\e[?1006h\e[?25l\e[?2004h\e[?1004h\e[?2031h…
```

| sequence | meaning | present |
|---|---|---|
| `\e[?1049h` | enter alternate screen | **YES** (x1) |
| `\e[2J` `\e[H` | clear + home (fresh alt screen) | YES |
| `\e[?1000h/1002h/1003h/1006h` | mouse tracking (incl. SGR) | YES |
| `\e[?2004h` | bracketed paste | YES |
| `\e[?1004h` | focus in/out reporting | YES |
| `\e[?2026h` / `\e[?2031h` | synchronized output / mode 2031 | YES |
| `\e[<u` `\e[>1u` | Kitty keyboard protocol push/flags | YES |
| explicit DECSTBM region `\e[<t>;<b>r` | scrolling region set | no (only `\e[r` reset) |

**VERDICT: ALTERNATE SCREEN — no accumulating terminal scrollback while the TUI is live.**

## Reconciliation with spike 08

Spike 08 (run at the *trust prompt* in an untrusted temp dir) observed "no alt-screen" and
pure absolute-column cursor layout. No contradiction: the **trust dialog renders on the
normal buffer**, *before* claude enters its main TUI. Spike 09 runs in a trusted dir, so it
reaches the main TUI and captures the `\e[?1049h` switch. Two phases, both correct:
normal-buffer pre-flight (trust/permission preamble) → alt-screen main TUI.

## Design impact (eigen session-host)

1. The server-side parsed-terminal model needs to hold only the **current alt-screen grid**
   (one viewport), not a scrollback ring. On re-attach: serialize that grid → one clean
   repaint → resume live streaming. No raw-byte re-animation, no unbounded buffer.
2. Re-attaching at a different size: resize the pty to the client → claude repaints the full
   screen itself (alt-screen apps redraw on SIGWINCH), so the snapshot is self-healing.
3. "Scroll back to the first user input" in the *terminal pane* is claude's own concern
   (it scrolls/repaints internally while live) and is **not** reconstructable by us after the
   fact. Durable, navigable-to-origin history is the **JSONL transcript drawer** — that is the
   authoritative deep-history surface, independent of terminal state, reload, or reattach.
4. We must mirror the full mode set on attach (mouse, bracketed paste, focus, Kitty keyboard,
   synchronized output) for input fidelity — xterm.js addons/config must match what claude
   negotiates, or keys/mouse/paste will misbehave.
