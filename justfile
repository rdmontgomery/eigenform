# eigenform — canonical command invocations.
# Nothing here spawns `claude`; the engine is only ever launched from inside the app.

# Default target lists available recipes.
default:
    @just --list

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
    # --watch=forever, not --watch: esbuild stops watching when its stdin closes,
    # which it does the moment we background it with `&` in this non-interactive
    # recipe — you'd get one build and no hot reload. `forever` keeps it watching.
    npx esbuild src/main.ts --bundle --outdir=dist --format=esm --watch=forever </dev/null &
    ESBUILD=$!
    trap "kill $ESBUILD 2>/dev/null || true" EXIT
    cd ..
    cargo watch -w crates -w Cargo.toml -x 'run -q -p eigenform-cli -- daemon --port {{port}} --dev'

# --- testing -------------------------------------------------------------

# Rust workspace + webterm unit tests (never spawns claude).
test:
    cargo test --workspace
    cd webterm && npm install >/dev/null 2>&1 && npm run typecheck && npm test
