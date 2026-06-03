# crates/surgery — design (v0.1)

Status: designed 2026-06-03. Supersedes the "Surgery library shape (v0.1)" sketch in
`docs/plans/2026-06-02-eigen-foundation-design.md` (that sketch's `Header + Vec<Turn>`
model is too thin — see spike 03 and spike 07).

Empirical floor: [spike 03](../../notes/spikes/03-mid-tree-cold-load.md) (mid-tree
cold-load CONFIRMED) and [spike 07](../../notes/spikes/07-cross-version-passthrough.md)
(passthrough + guarded swap version-robust across 2.1.126 → 2.1.161).

## Telos

Pure library that performs context surgery on a Claude Code session JSONL and writes a
new resumable session into the same projects directory. The `claude` pty stays the
engine; surgery is off-path file authoring. No engine invocation in the library or its
default tests.

## Data model — passthrough

A session JSONL is a heterogeneous newline-delimited row stream, internally a tree
(turns carry `parentUuid`; a leaf pointer lives in `last-prompt.leafUuid`). We type only
what surgery reasons about and keep everything else as verbatim bytes.

```rust
enum Row {
    Turn(Turn),               // type ∈ {user, assistant, system}
    LastPrompt(LastPrompt),   // { last_prompt: Option<String>, leaf_uuid: Uuid, session_id: Uuid, raw: String }
    Opaque { raw: String },   // any other type — attachment, file-history-snapshot,
                              // ai-title, mode, permission-mode, pr-link, queue-operation, future
}

struct Turn {
    uuid: Uuid,
    parent_uuid: Option<Uuid>,
    is_sidechain: bool,
    role: Role,               // User | Assistant | System
    raw: String,              // original line; re-emitted unless deliberately edited
}

struct Session {
    session_id: Uuid,
    rows: Vec<Row>,
}
```

- **Total dispatch on `.type`.** Spike 07: 0 typeless rows across 6 versions. Unknown
  `.type` → `Opaque`. New Claude row types pass through untouched.
- **Raw-line retention everywhere.** Even `Turn` keeps `raw`; we index fields, we don't
  re-encode. Round-trip of an unedited session is byte-identical (a test asserts this).
- **`file-history-snapshot`** is the only row without `sessionId`; it is always
  `Opaque` and never id-rewritten.

## The rewrite primitive — guarded swap

Cross-cutting: the new session id must replace the old in every row that carries it
(opaque rows included). Spike 07 verified the session uuid appears *only* at `sessionId`
positions across ~3,700 occurrences, so a string-token swap is byte-faithful elsewhere —
but we make the assumption checked, not assumed:

```rust
fn rewrite_session_id(line: &str, old: Uuid, new: Uuid) -> Result<String> {
    // parse, walk every JSON value; if any value == old at a path whose final key
    // is not a known session field (sessionId), bail with the offending path.
    // otherwise return line.replace(&old, &new) — faithful except the deliberate swap.
}
```

A bail means "this file violates our invariant; refuse rather than corrupt." Loud, not
silent.

## Operations

All four route through one private engine — `splice_and_seal(src, boundary, edit) ->
Session` — which (1) locates the boundary turn (error if absent), (2) splices rows,
(3) drops trailing `last-prompt`s and appends a fresh one with `leaf_uuid` = new tip,
(4) mints a new `session_id` and guarded-swaps it across all rows.

```rust
fn parse(path: &Path) -> Result<Session>;

fn fork_at(src: &Session, turn: Uuid) -> Result<Session>;        // prefix through turn's trailing system row
fn rewind_to(src: &Session, turn: Uuid) -> Result<Session>;      // alias for fork_at
fn edit_then_fork(src: &Session, turn: Uuid, new_payload: TurnPayload) -> Result<Session>; // fork + swap boundary payload
fn inject(src: &Session, after: Uuid, role: Role, payload: TurnPayload) -> Result<Session>; // splice a synthetic turn

fn write(session: &Session, projects_dir: &Path) -> Result<Uuid>; // writes <uuid>.jsonl, refuses to clobber, returns uuid
```

`fork_at` validated by spike 03 Run 1; `inject` by Run 2. `rewind_to` is a fork_at
alias (no resume seed beyond the re-point). The CLI binary stays thin: prints the new
uuid to stdout; the human (or future daemon) runs `claude --resume <uuid>`.

The synthetic-turn builder for `inject`/`edit_then_fork` emits the field shapes spike 03
proved Claude accepts: user rows with `promptId`/`promptSource`/`permissionMode`;
assistant rows with `message.id`/`requestId`/`usage`(core counters + `service_tier`
suffice)/`stop_reason`; each turn optionally closed by a `turn_duration` system row.
uuids/ids are freshly minted.

## Testing

**Unit (committed, CI-safe, deterministic):** hand-built synthetic fixtures in
`crates/surgery/tests/fixtures/`, scrubbed by construction, covering each version's
row-type set and edge cases (`pr-link`, `queue-operation`, sidechain turns, multi-row
assistant turns, a session whose stray-guard must bail). The spike sessions are ideal
real-but-safe 2.1.161 fixtures.

**Corpus property test (gated, graceful, bounded):** `tests/corpus.rs`
- Corpus dir: `EIGEN_CORPUS_DIR` env, else `~/.claude/projects`. Absent or empty →
  `eprintln!` skip note and pass. Never fails a dev for lacking history.
- Streams each file line-by-line (never loads whole). Caps at 64 sessions sampled
  newest-first under a total byte budget; if more exist it validates the sample and
  **logs the count skipped** (`EIGEN_CORPUS_FULL=1` lifts the cap). Target: sub-second,
  well under the 1-minute ceiling even on large corpora.
- Per session asserts: `parse` succeeds · re-emit byte-identical to input ·
  guarded swap finds 0 stray for that file's own id · last `last-prompt.leafUuid`
  resolves. Surfaces cross-version drift automatically on machines with history.

**Live `claude` test (triple-gated, unchanged from foundation design):**
`#[cfg(feature = "live-claude")]` + `EIGEN_ALLOW_LIVE_CLAUDE=1` or panic + `just
test-live` only; CI never runs it; never invoked without explicit human go-ahead.

## Build order (TDD)

1. `parse` + `Row`/`Turn` model + byte-identical round-trip test (synthetic fixtures).
2. `rewrite_session_id` guarded swap + bail test.
3. `splice_and_seal` engine + `fork_at`/`rewind_to`.
4. `write` (clobber-refusal, returns uuid).
5. `inject` + synthetic-turn builder.
6. `edit_then_fork`.
7. Corpus property test.
8. Thin CLI wiring: `eigen surgery fork|rewind|inject|edit`.

## Non-goals (v0.1)

- No in-place mutation of the source session (always writes a new file).
- No semantic validation of payload content.
- No daemon/HTTP surface (lands with `crates/daemon`, build step 6).
- No tail fork via `--fork-session` (that's spike 02 territory / a separate path).
