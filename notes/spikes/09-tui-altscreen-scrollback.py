#!/usr/bin/env python3
"""Spike 09: does `claude`'s TUI use the alternate-screen buffer?

We need to know whether the live terminal pane can accumulate traditional
scrollback (normal buffer) or whether claude takes over the alternate screen
(no accumulating scrollback while live). This decides whether our server-side
parsed-terminal snapshot can "scroll back to the first user input" in the
terminal pane, or whether deep history must come from the JSONL drawer.

Method: spawn `claude` in a pty, send NO prompt (zero tokens), read the raw
startup byte stream for a few seconds, kill it, and scan for the relevant
DEC private-mode escape sequences:

    \\e[?1049h  enter alternate screen buffer   (the decisive signal)
    \\e[?1049l  leave alternate screen buffer
    \\e[?47h / \\e[?47l   legacy alt-screen toggle
    \\e[?1047h           alt-screen (no clear)
    \\e[?25l / \\e[?25h   hide / show cursor
    \\e[r / \\e[<t>;<b>r  set scrolling region (DECSTBM)

Run: python3 notes/spikes/09-tui-altscreen-scrollback.py
"""

import os
import pty
import re
import select
import signal
import sys
import time

CLAUDE = os.path.expanduser("~/.local/bin/claude")
READ_SECONDS = 4.0


def main() -> int:
    captured = bytearray()
    pid, fd = pty.fork()
    if pid == 0:  # child
        # No args, no prompt — just boot the interactive TUI.
        os.execv(CLAUDE, [CLAUDE])
        os._exit(127)

    # parent: drain the pty for a few seconds, never write anything to stdin.
    deadline = time.time() + READ_SECONDS
    try:
        while time.time() < deadline:
            r, _, _ = select.select([fd], [], [], 0.2)
            if fd in r:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                captured.extend(chunk)
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(pid, 0)
        except ChildProcessError:
            pass

    raw = bytes(captured)
    print(f"# claude TUI startup capture — {len(raw)} bytes in {READ_SECONDS}s\n")

    checks = {
        r"\e[?1049h enter alt-screen (DECISIVE)": rb"\x1b\[\?1049h",
        r"\e[?1049l leave alt-screen": rb"\x1b\[\?1049l",
        r"\e[?47h  legacy alt-screen": rb"\x1b\[\?47h",
        r"\e[?1047h alt-screen (no clear)": rb"\x1b\[\?1047h",
        r"\e[?25l  hide cursor": rb"\x1b\[\?25l",
        r"\e[?25h  show cursor": rb"\x1b\[\?25h",
        r"DECSTBM set scroll region \e[<t>;<b>r": rb"\x1b\[\d+;\d+r",
        r"\e[?2004h bracketed paste": rb"\x1b\[\?2004h",
        r"\e[?1000h/1002h/1006h mouse tracking": rb"\x1b\[\?100[026]h",
    }
    for label, pat in checks.items():
        hits = len(re.findall(pat, raw))
        mark = "YES" if hits else " no"
        print(f"  [{mark}] {label}   (x{hits})")

    alt = bool(re.search(rb"\x1b\[\?1049h", raw)) or bool(
        re.search(rb"\x1b\[\?47h", raw)
    ) or bool(re.search(rb"\x1b\[\?1047h", raw))
    print()
    print("VERDICT:", "ALTERNATE SCREEN (no accumulating terminal scrollback)"
          if alt else "NORMAL BUFFER (terminal scrollback accumulates)")

    # First ~200 bytes, escaped, for the record.
    head = raw[:200].decode("latin-1").replace("\x1b", "\\e")
    print("\n# first 200 bytes (escaped):\n" + head)
    return 0


if __name__ == "__main__":
    sys.exit(main())
