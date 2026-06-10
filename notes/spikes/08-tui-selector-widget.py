#!/usr/bin/env python3
"""Capture claude's trust-dialog TUI widget from a real pty. Zero tokens:
the trust prompt renders before any API call. We never confirm it."""
import os, pty, select, struct, fcntl, termios, time, sys, tempfile, signal

workdir = tempfile.mkdtemp(prefix="eigen-spike-trust-")
# make it look like a real project so the trust prompt is meaningful
open(os.path.join(workdir, "README.md"), "w").write("# spike scratch\n")

raw = bytearray()

def set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

pid, fd = pty.fork()
if pid == 0:  # child
    os.chdir(workdir)
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    os.execvpe("claude", ["claude"], env)
    os._exit(127)

# parent
set_winsize(fd, 40, 100)

def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.2)
        if fd in r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                return False
            if not data:
                return False
            raw.extend(data)
    return True

pump(3.5)            # initial render → trust dialog
marker = len(raw)
os.write(fd, b"\x1b[B")  # Down arrow — does the ❯ cursor move?
pump(1.5)
arrow_marker = len(raw)
os.write(fd, b"\x1b[A")  # Up arrow — move back
pump(1.5)

os.kill(pid, signal.SIGKILL)
os.close(fd)

with open("/tmp/eigen-spike-trust-raw.bin", "wb") as f:
    f.write(raw)

print(f"workdir: {workdir}")
print(f"total bytes: {len(raw)}  (pre-arrow={marker}, post-down={arrow_marker})")
print("=== RENDERED (escape sequences shown as repr, by line) ===")
# crude render: strip CSI sequences for a readable view, keep structure
import re
text = raw.decode("utf-8", "replace")
# show the last full-screen redraw: split on clear-ish sequences is messy;
# instead print the raw repr of the final ~4KB so patterns are visible.
print("--- final 3500 bytes, repr ---")
print(repr(text[-3500:]))
