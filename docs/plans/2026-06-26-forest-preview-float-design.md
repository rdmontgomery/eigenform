# Forest preview float + intuitiveness improvements

**Date:** 2026-06-26
**Status:** Design, ready for implementation

## Summary

Make the forest (session rail) more intuitive by letting you **inspect a session
before committing to launch it**, plus a set of smaller forest-UX fixes. The
anchor feature is a floating transcript preview that appears when you focus a
forest row. Bundled with it: per-project cwd colors, inline row meta, a launch
button in the float, and keyboard navigation.

This is deliberately a *separate float* from the docked inspect panel. The dock
keeps meaning "the active session's inspect panel"; the float means "I'm
browsing, not committed yet." They never drive the same data.

## Decisions (locked)

| Question | Decision |
| --- | --- |
| Trigger | Select/focus a row (single click **or** arrow key) |
| Relationship to dock | Separate floating overlay, not the flex dock |
| Float content | Transcript + compact meta header |
| Liveness | Static snapshot, re-fetched on focus (no SSE) |
| Extras | Per-cwd colors, launch-from-float, inline row meta, keyboard nav |
| Message count | Approximate (jsonl newline count), labeled `~N` |

## Non-goals (YAGNI)

- No live/SSE tailing in the float. It is a static snapshot; this also decouples
  the feature from the unresolved live-update SSE wiring bug (see Appendix).
- No reach map in the float — transcript + meta only.
- No exact grouped-exchange count in the rail rows (too heavy per row).

## Interaction model

The central change: **decouple "focus a row" from "launch a row."** Today a
single click immediately launches/attaches (`webterm/src/shell.ts:1287`).

New behavior:

- **Focus** (single click *or* `↑/↓`) → marks the row selected
  (`.rail-row--selected`) and opens the preview float for that session. No launch,
  no tab opened.
- **Launch/Resume** → explicit commit: `Enter`, double-click on the row, or the
  Launch button in the float. Routes through the existing `openTabWithQuery`
  (attach via `?attach=<ptyId>` when a `ptyId` exists, else resume via
  `?session=<uuid>`).
- **Dismiss** → `Esc`, clicking empty space, or focusing nothing.

Mouse and keyboard share one `selectedKey` state so they never fight.

> **Behavior change accepted:** single click now previews instead of launching
> instantly. Launch remains one keystroke away (`Enter`).

## The preview float

One reusable overlay node, created once and repositioned/repopulated as focus
moves — not one node per row.

**Structure** (`forest-preview`):

- **Meta header** — cwd (full path, with the per-cwd tint applied),
  last-activity (`relativeRecency`), message/turn count, model, and a `· live`
  marker when the session is currently running. A **Launch/Resume** button lives
  here.
- **Transcript body** — scrollable transcript rendered by the *same*
  `groupTurns` + render path the dock's drawer uses (`webterm/src/drawer.ts`), so
  there is one transcript renderer, not two. Scrolled to the most-recent end on
  open.

**Positioning** — `position: fixed`, anchored to the right of the rail and
vertically aligned to the focused row, clamped to the viewport so it never clips
off-screen. On narrow widths, falls back to an overlay centered over the
terminal. Layered above the terminal, below modals.

**Lifecycle** — exactly one float instance. Focusing a new row updates its
content in place (cheap; no teardown churn). Losing focus hides it
(`display:none`) but keeps the node for reuse. Because the float never subscribes
to anything, it sidesteps the mount/teardown races suspected in the dock's
`syncDock`.

## Data flow (static fetch)

The float fetches once per focus, with a small client cache so re-focusing a
recently-seen row is instant.

**Row shapes** (from `webterm/src/roster.ts:45`):

- **Has `uuid`** (disk session, or reconciled live pty) → fetch
  `/api/session/<uuid>/json` directly. Common case for browse-before-launch.
- **Has `ptyId`, no `uuid` yet** (freshly spawned, not reconciled) → no
  transcript on disk yet. Show the meta header with a "session still
  initializing" placeholder. No infinite spinner.
- **Neither** → not launchable; preview disabled.

**Fetch + derive** (on focus with a uuid):

1. `GET /api/session/<uuid>/json` — already cached server-side on `(mtime,len)`
   (`crates/daemon/src/lib.rs` `SessionJsonCache`), so cheap and re-render-safe.
2. Render transcript via the shared `groupTurns` path.
3. Derive meta: message/turn count from `exchanges.length`; **model** from the
   new top-level `model` field (see backend change below); cwd + last-activity
   from the `RosterRow` we already hold.

**Client cache** — `Map<uuid, payload>` holding the last ~10 focused sessions,
so arrow-key scrubbing doesn't refetch. Dropped on forest refresh.

**Race-free** — focus is serial and user-driven; a stale in-flight fetch is
discarded by comparing against the current `selectedKey` before rendering.

## Backend changes

### 1. Add `model` to `session_json` (required by the float)

`eigenform_render::session_json` (`crates/render/src/lib.rs:88`) currently emits
`{ id, total, branches, windowStart, exchanges }` — **no model**. The model id is
present in the raw JSONL on each assistant turn as `message.model` (same
`value["message"]…` shape `tool_use_blocks` reads at `render/lib.rs:166`).

Add a top-level `"model"` field: scan turns for the first assistant turn carrying
`message.model`, emit it (or `null` if none found). Keeps the `(mtime,len)` cache
valid and gives every consumer the model for free.

### 2. Add approximate `msgCount` to forest rows (for inline row meta)

`RosterRow` (`webterm/src/roster.ts:45`) has no count. The forest backend
(`crates/forest`) emits an **approximate** count = newline count of the jsonl
(one cheap read, no parse) as `RosterRow.msgCount`. Live rows use their known
turn count. Rendered as `~142` to stay honest that it is approximate, not the
exact grouped-exchange count the float shows.

## Frontend extras

- **Per-cwd colors.** Change `inkVar(row.uuid, row.ptyId, row.cwdChip)`
  (`shell.ts:1261`) to hash on the **full cwd path** (`row.cwd`, falling back to
  `cwdChip`). Same project → same hue, so the tint finally means what its
  placement implies. Hash the full path, not the basename, so two unrelated
  `…/src` dirs don't collide. Visible change: existing rows recolor/regroup by
  project (intended). Session identity still carried by dot/state + label.

- **Inline row meta.** Rows already show `relativeRecency`. Add the approximate
  `~N` message count alongside it.

- **Launch from float.** Launch/Resume button in the meta header + `Enter`, both
  routing through `openTabWithQuery`.

- **Keyboard nav.** One `selectedKey` state. `↑/↓` walk the flattened
  visible-rows list (respecting group order + active search filter), clamped at
  ends; `Enter` launches; `Esc` clears selection and hides the float. Keys are
  ignored while the search input or a rename field has focus, so typing is never
  hijacked.

## Error handling

- Fetch failure → meta header renders from the `RosterRow` data we already have;
  transcript body shows an inline "couldn't load transcript" message with the
  status. The float never blocks launch.
- `ptyId`-only / uuid-less rows → placeholder, preview disabled, launch still
  works.
- Stale fetch (focus moved) → discarded, no render.
- Viewport clamping ensures the float is always fully visible.

## Testing

- **Render unit tests** (render crate): `session_json` emits `model` for a
  session with assistant turns; emits `null`/absent cleanly when none; cache key
  unaffected.
- **Roster unit tests**: `msgCount` populated from jsonl line count; live rows
  carry their turn count.
- **Frontend**: focus-selects-without-launching; `Enter`/button launches via the
  correct `?attach=` vs `?session=` route; arrow-key nav crosses group
  boundaries and clamps; `Esc` dismisses; keys ignored while search/rename
  focused; client cache hit on re-focus; stale-fetch discard.
- **Manual**: per-cwd color regrouping looks right across real sessions; float
  positioning clamps near top/bottom of the list and on narrow widths.

## Appendix: related live-update SSE bug (separate work)

While investigating, we found the docked inspect panel (reach + transcript) does
**not** live-update for the currently-running session. Root cause is **not**
confirmed. Leading candidates:

1. A uuid-arrival race in the merged `syncDock()` introduced by commit
   `7c7a6d0` — the dock may mount before the live session's uuid is broadcast and
   not cleanly remount when it arrives.
2. A watcher-setup race in `watch_sse` (`crates/daemon/src/lib.rs`): the SSE
   response is returned before `notify::Watcher::watch()` is armed, so early
   writes can be missed.

This preview feature is intentionally static, so it does **not** depend on
fixing this. The bug should be pinned down and fixed as its own task.

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
