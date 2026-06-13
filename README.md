# eigenform

> manuscripts don't burn.

A control surface over Claude Code (and imported Claude Chat) that performs context surgery, manages a session forest, and surfaces the eigenforms across a body of work.

The binary is `eigenform`; alias it to taste (`alias ef=eigenform`). Dual-licensed [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE).

## Install & run

eigenform is a single local daemon that serves a browser app and hosts your Claude Code pty sessions. Install it once and the app is baked into the binary — no Node, no build step, no flags:

```sh
just install        # builds the frontend, then `cargo install` with assets embedded
eigenform           # starts the daemon and opens the app in your browser
```

`eigenform` with no arguments picks a port (4317), serves the app at `http://127.0.0.1:4317/`, and opens it. For scripting, `eigenform daemon` is the explicit form (`--port`, `--open`, `--cmd`, `--workspace`). The pty spawns your `$SHELL`; **`claude` is only ever launched from inside the app**, never by the daemon.

Requires a Rust toolchain (and Node only at build time). `cargo install` builds a release binary with `--features embed-assets`, which is what makes it self-contained.

## Develop

```sh
just dev            # esbuild --watch + cargo-watch: edit .ts → browser reloads, .rs → daemon restarts
just run            # one-shot: build the app, run the daemon, open the browser
just test           # workspace unit/integration tests (never spawns claude)
```

In a dev checkout the daemon serves the frontend from disk (`webterm/dist`), so you don't rebuild the binary to see UI changes — `just dev` rebuilds the bundle and live-reloads the page. The legacy **woland** workbench is paused; when built (`just build-woland`) it's served at `/woland`.

## Status

Pre-implementation. The design is at [`docs/plans/2026-06-02-eigen-foundation-design.md`](docs/plans/2026-06-02-eigen-foundation-design.md). Spike notes (load-bearing empirical claims) live in [`notes/spikes/`](notes/spikes/).

## What this is

Three operations, one dialectic:

- **Fork** *negates* — context surgery on a session: branch, rewind, edit-then-fork, inject a synthetic turn.
- **Recent-work surfacing** *preserves* — a session forest indexed by project, time, keyword, and semantics.
- **Eigenform graph** *elevates* — a hypergraph whose edges name shared fixed-point structures across surface-disparate threads.

The Aleph is the failure mode (all-seeing as simultaneity = paralysis). Coarse-graining is the cure (all-seeing as recall = help). Every feature serves resumption, forking, recall, or reframing.

## What this is not

- Not a terminal multiplexer.
- Not a generic token dashboard.
- Not an SDK/`--print` engine. The interactive `claude` pty is the engine; everything else is off-path enrichment.

## How to read this repo

1. Start with [`docs/plans/2026-06-02-eigen-foundation-design.md`](docs/plans/2026-06-02-eigen-foundation-design.md).
2. Then [`notes/spikes/`](notes/spikes/) — what we've verified empirically, what's pending, what would falsify the design.
3. Then [`justfile`](justfile) — the canonical commands. Engine-touching targets are human-triggered.
