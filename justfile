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

# --- eigenform (browser app) ---------------------------------------------

# Build the eigenform (webterm) bundle → webterm/dist (served at /, baked in by install).
build:
    cd webterm && npm install && npm run build

# Build the legacy woland workbench bundle → web/dist (paused; served at /woland).
build-woland:
    cd web && npm install && npm run build

# Build the app, start the daemon, open the browser at / (pty spawns $SHELL, never claude).
run port="4317": build
    cargo run -q -p eigenform-cli -- daemon --port {{port}} --open

# Install a self-contained `eigenform` (assets baked in) onto your PATH — run it from anywhere.
install: build
    cargo install --path crates/eigenform-cli --features embed-assets --locked
    @echo
    @echo "  installed 'eigenform'. add the short 'ef' alias to your shell:"
    @echo
    @echo "      echo 'alias ef=eigenform' >> ~/.zshrc && source ~/.zshrc"
    @echo
    @echo "  (use ~/.bashrc for bash) — then just run:  ef"

# Hot-reload dev loop (needs `cargo install cargo-watch`): .ts → browser refresh, .rs → daemon restart.
dev port="4317":
    #!/usr/bin/env bash
    set -euo pipefail
    cd webterm && npm install >/dev/null 2>&1
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
