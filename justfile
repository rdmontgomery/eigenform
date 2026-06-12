# eigenform — canonical command invocations.
# Engine-touching targets are human-triggered. CI never runs them.

# Default target lists available recipes.
default:
    @just --list

# --- spikes --------------------------------------------------------------
# These spawn `claude` interactively against your real plan. Run by hand.

# spike 02 — tail fork via --fork-session.
# Sets up before/after snapshots so you can diff the projects dir.
spike-02 src_uuid:
    @echo "spike-02: see notes/spikes/02-tail-fork.md"
    @echo "snapshotting projects dir BEFORE fork ..."
    ls ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl | sort > /tmp/eigen-spike-02-before.txt
    @echo "now launching: claude --resume {{src_uuid}} --fork-session"
    @echo "(send one trivial turn, then /exit)"
    claude --resume {{src_uuid}} --fork-session
    @echo "snapshotting projects dir AFTER fork ..."
    ls ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl | sort > /tmp/eigen-spike-02-after.txt
    @echo "--- new files ---"
    diff /tmp/eigen-spike-02-before.txt /tmp/eigen-spike-02-after.txt || true
    @echo "record outcome in notes/spikes/02-tail-fork.md"

# spike 03 — mid-tree cold-load.
# This one needs editing notes/spikes/03 by hand. Justfile only enforces the
# safety check that you really mean it.
spike-03:
    @echo "spike-03 is hand-driven. see notes/spikes/03-mid-tree-cold-load.md"
    @echo "you'll need: a source session with a memorable mid-conversation fact,"
    @echo "the truncation turn uuid, and willingness to ask the resumed model"
    @echo "the same probe questions twice (once on source, once on fork)."

# spike 04 — billing flip.
# Pure observation; no command needed beyond your own dashboard checks.
spike-04:
    @echo "spike-04 is pure observation. see notes/spikes/04-billing-flip.md"
    @echo "track /cost before and after one --print invocation."

# --- woland (browser workbench) ------------------------------------------

# Build the TypeScript + xterm.js frontend bundle (web/dist).
build-web:
    cd web && npm install && npm run build

# Run woland: a live pty (your shell) rendered in the browser. Open the URL it prints.
# Spawns your $SHELL, NOT claude — wiring claude --resume is a later, user-initiated slice.
woland port="4317": build-web
    cargo run -q -p eigenform-cli -- daemon --port {{port}}

# Hot-reload dev loop: esbuild --watch rebuilds the bundle; cargo-watch rebuilds+restarts
# the daemon on Rust changes; the browser live-reloads on frontend changes (dev mode).
# Edit .ts → browser refreshes; edit .rs → daemon restarts. claude never auto-respawns.
# Requires: cargo install cargo-watch
dev port="4317":
    #!/usr/bin/env bash
    set -euo pipefail
    cd web && npm install >/dev/null 2>&1
    npx esbuild src/main.ts --bundle --outdir=dist --format=esm --watch &
    ESBUILD=$!
    trap "kill $ESBUILD 2>/dev/null || true" EXIT
    cd ..
    cargo watch -w crates -w Cargo.toml -x 'run -q -p eigenform-cli -- daemon --port {{port}} --dev'

# --- testing -------------------------------------------------------------

# Safe default: unit tests only.
test:
    cargo test --workspace

# Live integration test (spawns real claude). Triple-gated.
# Requires: cargo feature 'live-claude' AND env EIGENFORM_ALLOW_LIVE_CLAUDE=1.
test-live:
    EIGENFORM_ALLOW_LIVE_CLAUDE=1 cargo test --workspace --features live-claude -- --ignored
