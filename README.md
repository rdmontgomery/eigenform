# eigenform

> manuscripts don't burn.

A control surface over Claude Code (and imported Claude Chat) that performs context surgery, manages a session forest, and surfaces the eigenforms across a body of work.

The binary is `eigenform`; alias it to taste (`alias ef=eigenform`). Dual-licensed [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE).

## The mulligan

Sometimes the bigger model (Fable) decides a prompt is risky and quietly drops your live session to a smaller one, mid-conversation. Usually it's a false read — a benign security question, a dual-use tool, a CTF box — and the session just got dumber without asking you.

eigenform catches the drop as it lands. It forks a fresh Fable branch rewound to the turn *before* the one that tripped the wire, drafts a cleaner way to ask, and stages it in your input — **never sent.** You read it, edit it, decide. One keystroke and you're back on the model you started with.

If the ask was genuinely over the line, the draft says so instead of helping — this is recovery from *wrong* downgrades, not a way around right ones. It's the [fork operation](#what-this-is) pointed at one specific, annoying failure: copy-on-write, so the original thread is never touched, and nothing leaves your machine.

## Install & run

eigenform is a single local daemon that serves a browser app and hosts your Claude Code pty sessions. Install it once and the app is baked into the binary — no Node, no build step, no flags:

```sh
just install        # builds the frontend, then `cargo install` with assets embedded
eigenform           # starts the daemon and opens the app in your browser
```

`eigenform` with no arguments is the one-command launch: if a daemon is already running it just opens the browser; otherwise it starts the daemon **in the background** (port 4317), opens the app, and returns your terminal. The daemon outlives the launching shell on purpose — it hosts your Claude sessions, so closing the terminal must not kill them. Run it again any time to reopen the app; it won't start a second daemon.

```sh
eigenform           # launch (or reopen) the app — backgrounds the daemon
eigenform status    # is a daemon running? (pid + version)
eigenform stop      # shut the daemon down (ends the sessions it hosts)
```

For a foreground daemon (dev, debugging, scripting) use the explicit `eigenform daemon` (`--port`, `--open`, `--cmd`, `--workspace`, `--dev`); there `Ctrl-C` stops it. Background logs go to `~/.eigenform/state/daemon.log`. The pty spawns your `$SHELL`; **`claude` is only ever launched from inside the app**, never by the daemon.

### Prerequisites

- **[Rust](https://rustup.rs)** toolchain (`cargo`) — builds the binary.
- **[Node](https://nodejs.org)** — only at build time, to bundle the frontend.
- **[`just`](https://github.com/casey/just)** — a command runner. If you know `make`, `just` is the same idea: the [`justfile`](justfile) holds named recipes (`install`, `run`, `dev`, `test`) and `just <recipe>` runs one. It's simpler than `make` — recipes are plain shell, no tabs-vs-spaces, no build-graph. Install it with:

  ```sh
  cargo install just          # any platform (you already have cargo)
  # or: brew install just · apt install just · scoop install just
  ```

  Then `just --list` shows every recipe. `cargo install` builds a release binary with `--features embed-assets`, which is what makes it self-contained.

Prefer not to install `just`? The recipes are thin — run the underlying commands yourself:

```sh
cd webterm && npm install && npm run build && cd ..        # = just build
cargo install --path crates/eigenform-cli --features embed-assets --locked   # = just install
```

### Alias `ef`

The binary is `eigenform`; most people alias it to `ef`. Add it to your shell rc:

```sh
echo 'alias ef=eigenform' >> ~/.zshrc   # or ~/.bashrc
source ~/.zshrc
```

Then `ef` launches the app and `ef daemon`, `ef sessions`, `ef surgery …` all work.

## Develop

```sh
just dev            # esbuild --watch + cargo-watch: edit .ts → browser reloads, .rs → daemon restarts
just run            # one-shot: build the app, run the daemon, open the browser
just test           # workspace unit/integration tests (never spawns claude)
```

In a dev checkout the daemon serves the frontend from disk (`webterm/dist`), so you don't rebuild the binary to see UI changes — `just dev` rebuilds the bundle and live-reloads the page. The legacy **woland** workbench is paused; when built (`just build-woland`) it's served at `/woland`.

## Status

Early but running. The browser app — a full-fidelity terminal centerpiece with a session host, launcher, and transcript drawer — is implemented and self-contained via `just install`. The context-surgery, forest, render, skills, memory, and inspect crates are built and tested; the eigenform graph is still ahead. The original design is at [`docs/plans/2026-06-02-eigen-foundation-design.md`](docs/plans/2026-06-02-eigen-foundation-design.md), and spike notes (load-bearing empirical claims) live in [`notes/spikes/`](notes/spikes/).

The [mulligan](#the-mulligan) (Fable→Opus downgrade recovery) is wired end-to-end and tested. It arms once the guardrail's exact notice string is pinned from a live occurrence — until then it's dormant by design (detection is a signature match, and the marker is a documented placeholder). Design: [`docs/plans/2026-07-02-fable-downgrade-recovery-design.md`](docs/plans/2026-07-02-fable-downgrade-recovery-design.md).

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
