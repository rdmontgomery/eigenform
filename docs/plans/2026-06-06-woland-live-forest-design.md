# Woland: the live Forest — corroborated session state + real sparkline

Date: 2026-06-06

Turn the Forest from a static recent-list into a live, glanceable view of every
session: which are running, which are working vs ready for you, what each is about,
and a real activity sparkline. Built on **corroboration as the source of truth** —
the daemon reconstructs state from the filesystem at any instant — with hooks as a
later enrichment, not the channel.

## Guiding principle

Two kinds of state, two homes:

- **Derivable** (reconstructible from disk anytime): liveness (pid), topic/recency
  (JSONL tail), working-vs-ready (last turn complete?), activity (per-turn tokens).
- **Eventful** (a moment awkward to read back): the *instant* a turn ends; a
  permission-wait. These are what v2 hooks capture.

Making corroboration the source of truth dissolves two traps:
- **Cold-start** (5 sessions already running when woland starts): the daemon reads
  their full state from disk; it never needed to witness their events.
- **TTL/cleanup**: the lifecycle is the *process*, not a timer. A stale
  `<pid>.json` (April files prove they linger) is ignored once its pid is dead, and
  pruned opportunistically. **The pid check is the GC** — no TTL.

## A. Data model & corroboration (`eigen-forest`)

New pure function mirroring `list()`:

```
live_forest(projects_dir, sessions_dir, now) -> Vec<LiveSession>
```

1. Enumerate `~/.claude/sessions/<pid>.json` → `{pid, sessionId, cwd}`.
2. **Liveness filter**: pid alive? (`/proc/<pid>` on this WSL box; `kill(pid,0)`
   portable). Dead → drop (the GC).
3. Join each live session to its project JSONL via the existing cheap **tail-peek**
   → `title`, `recency`, and a new `complete: bool` (does the tail end on a
   `turn_duration` row?).
4. State: `ready` if complete, else `working`.
5. Merge with `list()` recents; non-live → `recent`.

```
LiveSession { uuid, title, cwd, recency, live: bool, state: Working|Ready|Recent, spark: Vec<u32> }
```

State vocabulary (v1): live → `working` | `ready`; non-live → `recent` (recency
conveys staleness; no separate "idle"). v2 hooks add `needs-input` (permission-wait).

Pure → unit-tested with temp `sessions/` + `projects/` fixtures.

## B. The persisted metrics file (`~/.eigen/state/`)

`~/.eigen/` becomes eigen's home (configs, derived state). Per session:

```
~/.eigen/state/<sessionId>.json
  { source_mtime, source_len, spark: [out₁, out₂, … outₙ], total }
```

- `spark[i]` = turn i's `output_tokens` (from the JSONL `usage` block, present on
  every assistant turn) — **work generated per turn**. Up/down = rhythm: spikes are
  long generations, flat are quick exchanges.
- Maintained **parse-on-change**: if the JSONL's `(mtime, len)` differs from the
  stored stamp, recompute (full parse, already cached in memory) and rewrite; else
  it's a cheap read. The expensive scan is amortized to once-per-new-turn, not
  once-per-glance. (Incremental append is a later optimization.)
- `live_forest` reads this file for the spark. A never-parsed session renders
  without a spark and fills in async — the live list never blocks on parsing.
- Same dir v2 hooks write `ready`/`working`/`needs-input` markers to.

## C. Daemon endpoints + live channel (SSE)

```
GET /api/forest        → live_forest() snapshot JSON (CLI mirror, debugging)
GET /api/watch/forest  → SSE: pushes the snapshot on change
```

`forest_sse(cfg)` task (new; `watch_sse` only pings, this computes + dedups):

- Triggers: **`notify`** on `~/.claude/sessions/` and the projects dir (snappy:
  activity, new sessions) **∪ a coarse ~3s tick** (catches pid exits, which are not
  filesystem events). v2: the `~/.eigen/state/` hook write is a third trigger.
- On any trigger: recompute snapshot, **emit only if changed** (hash vs last emit) —
  the tick is silent when nothing moved. Payload travels **in the SSE event**, so
  the client just renders (no refetch).

Errors: no `sessions/` dir → empty live set, fall back to recents; unreadable JSONL
→ row without a state badge.

## D. Forest UI (frontend)

- **Model**: `ForestEntry` gains `live`, `state`, and `spark` (already has shape →
  replace `stubShape`). SSE payload maps straight on.
- **Sort/group**: `ready` (wants you) → `working` → faint "— recent —" divider →
  recents by recency. Recency order within groups.
- **Badges** (reuse the dot + `wbEmber` pulse): `working` → pulsing `--cool` dot +
  "working…"; `ready` → steady `--amber` dot + "ready ↵"; `recent` → faint dot,
  recency only. Active (currently-viewed) keeps its amber left-border — orthogonal
  to process state, so a row can be both.
- **Recency stamp** via a small pure `fmtAgo()` (now/2m/3h/yesterday), unit-tested.
- **Real sparkline**: `forestGlyph` fed `spark` (output-tokens/turn). Fork whiskers
  stay dormant (no fake branches) until lineage tracking exists.
- **Diff-render**: `forest.fill()` becomes a **uuid-keyed diff** — patch existing
  rows in place (badge, recency, spark, active class), append new, remove dead,
  reorder by moving nodes. Preserves `:hover` and the open `⊕` picker; no flicker.

## E. CLI mirror

`eigen forest --live` prints the same `live_forest()` snapshot —
`state · title · cwd · recency · ~tokens` — honoring the CLI-mirrors-browser rule.
It's the daemon's corroboration function called from the CLI.

## Testing

- `live_forest()`, the metrics-file read/refresh, and `fmtAgo()` are pure →
  unit-tested (Rust temp dirs; TS). Liveness filter tested with a known-dead pid.
- SSE dedup/recompute and the diff-render are verified in-app.
- `cargo test` + web `node --test` + `typecheck` stay green.

## Out of scope (v1 → later)

- **Hooks** (`Stop`/`UserPromptSubmit`/`Notification` → `~/.eigen/state/`): the
  instant-ready edge and permission-wait state. Slots into `forest_sse` as a trigger.
- **Fork whiskers**: real branch count from fork lineage.
- **Context-weight sparkline** / incremental metrics append.
