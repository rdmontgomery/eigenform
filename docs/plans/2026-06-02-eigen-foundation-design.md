# eigen — foundation design

> Date: 2026-06-02. Status: validated, pre-implementation.
> Scope: the architecture and v0.1 deliverables. Later sections of the build order are sketched but not specified in detail.

## Telos

A control surface over Claude Code (and imported Claude Chat) that performs context surgery, manages a session forest, and surfaces the eigenforms across a body of work. Aufheben at two levels: per-session, fight the 70% problem by enabling resume-not-restart (fork *negates*, recent-work *preserves*); across the corpus, sublate threads into reframings via a hypergraph whose edges name shared fixed points (the *elevates*). Coarse-graining is the cure for the Aleph: every feature must serve resumption, forking, recall, or reframing.

## Hard architectural rules

1. **pty-only engine.** The interactive `claude` CLI in a pty draws the team token allotment; `--print` and the SDK flip to usage-based billing. The engine must be interactive. `--print` and the SDK are reserved for off-path enrichment (embeddings, fingerprint passes) and are explicitly NOT routed through during normal operation.
2. **JSONL is the source of truth.** Claude Code writes per-turn JSONL into `~/.claude/projects/<escaped-cwd>/<uuid>.jsonl` live as the session runs. The forest, context inspector, eigenform graph, and token economy all read from there, not from the terminal scrape. The pty layer is small and owns only what requires the live process: input, live token shimmer, permission-prompt interception.
3. **Copy-on-fork.** Source JSONLs are never mutated in place. Forks are new files with new uuids dropped alongside the original; the derived index lives in a sibling directory under our control.
4. **CLI is the browser's mirror.** Every renderable view has a CLI command that produces the same data. Bug reports are reproducible: the user pastes `eigen … --render text` output; the developer runs the same command and sees identical bytes. There is no browser-only code path. `crates/render/` is the single source of view logic, projected to `text|json|html`.
5. **Surgery is a thin layer over native fork.** `claude --fork-session` exists as a CLI flag; we shell out to it for tail forks. Surgery only writes JSONL bytes for mid-tree forks, edit-then-fork, synthetic-turn injection, and rewind.

## Empirical anchors (verified)

- claude version: 2.1.161.
- JSONL path: `~/.claude/projects/-home-rdmontgomery-projects-eigen/<uuid>.jsonl` (escape rule: `/` → `-`).
- JSONL is incremental and live during a session (this conversation's file grew while in progress).
- `~/.claude/sessions/` contains tiny per-PID bookkeeping files, NOT transcripts. The spec's claim that active sessions live there is corrected: only `~/.claude/projects/` holds transcripts.
- `claude --fork-session` exists as a flag.
- JSONL header rows encode a `leafUuid` pointer; turn rows carry `parentUuid` and `isSidechain`. The file is internally a tree with a leaf pointer, not a flat log.
- **Mid-tree cold-load CONFIRMED (spike 03, 2026-06-03):** a hand-built JSONL — prefix-truncated, sessionId rewritten, dropped into the projects dir — resumes via `claude --resume`, and the model's context is exactly that file: dropped turns don't leak, fully fabricated turns load and are recalled. Resume head = last `last-prompt` row's `leafUuid`, which surgery must re-point. This unblocks `crates/surgery` (build step 2).

## Repo layout

```
eigen/
├── Cargo.toml              # cargo workspace
├── crates/
│   ├── surgery/            # JSONL parsing + mid-tree fork + injection (library)
│   ├── render/             # View tree -> text|json|html projections
│   ├── forest/             # session/skills/memory indexing (step 5)
│   ├── daemon/             # http+ws daemon (step 6)
│   └── eigen-cli/          # the `eigen` binary, clap subcommands
├── web/                    # TS frontend (step 7+)
├── docs/plans/             # design docs (this file)
├── notes/spikes/           # empirical verification records
├── justfile                # canonical command invocations
└── README.md
```

Polyglot repo: `crates/` for Rust, `web/` for TS, siblings.

## CLI surface

One binary: `eigen`. `git`-style subcommands.

```
eigen surgery fork    <session> [--at <turn>] [--edit <file>]
eigen surgery inject  <session> --at <turn> --as <user|assistant> --content <file>
eigen surgery rewind  <session> --to <turn>

eigen sessions list   [--since <duration>] [--cwd <dir>] [--keyword <q>] [--render ...]
eigen sessions show   <session> [--render text|json|html]

eigen skills tree     [--cwd <dir>] [--render ...]
eigen skills list     --all-projects [--render ...]
eigen skills audit    [--render ...]

eigen memory tree     [--cwd <dir>] [--render ...]
eigen memory list     --all-projects [--render ...]
eigen memory audit    [--render ...]

eigen daemon                       # step 6
eigen web                          # step 7
```

`--render` semantics:
- `text` (default): paste-friendly, monospace, no ANSI when stdout is not a tty.
- `json`: structured, stable schema.
- `html`: same bytes the browser would render.

## Skills & memory semantics

**Override stack, not winners-only.** Skills resolution is additive across levels (global, plugin, repo, cwd) — only same-name skills collide. `tree` shows every level's contribution and marks collisions. `list --all-projects` is a full inventory across every project under `~/.claude/projects`. `audit` is a heuristic drift scan:
- skills in `~/.claude/skills/` whose content references cwd-specific paths (probably mis-scoped global).
- skills duplicated across projects with diverging content (copy-paste drift).
- name collisions with plugin skills.

Memory follows the same shape, applied to `~/.claude/projects/<cwd>/memory/` and any hierarchical `CLAUDE.md` files.

## Surgery library shape (v0.1)

Pure library, no I/O policy beyond writing the new JSONL into the same projects directory as the source. The CLI binary is thin.

```rust
struct Header { leaf_uuid: Uuid, session_id: Uuid, /* … */ }
struct Turn   { uuid: Uuid, parent_uuid: Option<Uuid>, is_sidechain: bool, payload: TurnPayload }
struct Session { header: Header, turns: Vec<Turn> }

fn parse(path: &Path) -> Result<Session>;
fn fork_at(src: &Session, turn: Uuid) -> Session;            // mid-tree, new uuid, prefix-only
fn edit_then_fork(src: &Session, turn: Uuid, new_payload: TurnPayload) -> Session;
fn inject(src: &Session, after: Uuid, as_role: Role, payload: TurnPayload) -> Session;
fn rewind_to(src: &Session, turn: Uuid) -> Session;          // alias for fork_at; no resume seed
fn write(session: &Session, projects_dir: &Path) -> Result<Uuid>;  // returns new uuid
```

The CLI prints the new uuid to stdout. The user (or future daemon) runs `claude --resume <uuid>` to continue.

## Tests

- Unit tests against fixture JSONLs in `crates/surgery/tests/fixtures/` — scrubbed real sessions.
- One **integration test, triple-gated** against accidental run:
  - `#[cfg(feature = "live-claude")]`
  - reads `EIGEN_ALLOW_LIVE_CLAUDE=1` from env or panics with a clear message
  - documented in justfile as the only canonical invocation: `just test-live`
  - CI never runs it.
- The test creates a tiny sentinel session via interactive `claude` (one turn), forks at the leaf, asserts the new uuid is listed by `claude --resume` picker and resumes without error.

## Spike plan

`notes/spikes/<NN>-<topic>.md` per spike. Each spike has: claim, procedure, result, claude version, date.

1. **JSONL liveness** — file is written incrementally during a session. **CONFIRMED 2026-06-02 on claude 2.1.161.**
2. **Tail fork via `--fork-session`** — pending. Documented in `notes/spikes/02-tail-fork.md`.
3. **Mid-tree cold-load** — pending. Load-bearing. Documented in `notes/spikes/03-mid-tree-cold-load.md`.
4. **Billing flip** — pending. Documented in `notes/spikes/04-billing-flip.md`.
5. **Cache TTL behavior** — deferred to step 9 (token economy).

Spikes 2–4 are gating for implementation start. The justfile contains `just spike-2`, `just spike-3`, `just spike-4` targets; engine invocations are explicit and human-triggered to respect the credit budget.

## Build order (final)

1. Spikes 2–4.
2. `crates/surgery` + `eigen surgery fork|inject|rewind|tail`.
3. `crates/render` (text + json projections; html lands with daemon).
4. `eigen skills tree|list|audit` + `eigen memory tree|list|audit`.
5. `crates/forest` + `eigen sessions list|show`.
6. `crates/daemon` (http + ws; html projection in render).
7. Browser vertical slice: forest browser + context inspector.
8. Affordance buttons (browser → daemon → surgery).
9. Token economy + cache-hotness.
10. Rhizome layer (semantic edges; local embeddings, likely via Ollama).
11. Labeling UI → eigenform classifier.
12. Chat→Code handoff.

Each step usable on its own.

## Non-goals

- Not a terminal multiplexer.
- Not a generic dashboard.
- No `--print`/SDK in the engine path.
- No eigenform classifier before hand-labeled edges exist.
- No renderer before surgery round-trips.

## Known unknowns

- ~~Whether `claude --resume` will accept a JSONL we author from scratch with a parent-uuid prefix copy.~~ RESOLVED (spike 03, 2026-06-03): yes — fully synthetic 3-turn session loaded and was recalled.
- ~~Whether mid-tree edits actually reach the model (cold-load assumption).~~ RESOLVED (spike 03, 2026-06-03): yes — truncated fork resumed with no knowledge of dropped turns.
- Whether plugin skills resolve identically to user skills in the override stack. Verifying during step 4.
- Embedding stack choice (Ollama local vs ONNX in-process vs Python sidecar). Deferred to step 10.
- TS frontend framework choice. Deferred to step 7.
