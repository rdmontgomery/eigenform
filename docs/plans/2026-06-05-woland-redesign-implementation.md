# Woland redesign — implementation plan

Implements the Claude Design handoff (`woland.html` + its 7 imports) into the real
`web/` surface. The design is a React prototype; we recreate its visuals and
interactions in **vanilla TypeScript** (no framework), honoring the existing
esbuild + xterm.js stack and the "clean, non-compute-hog rendering" goal.

See also the original brief in `2026-06-03-claude-design-brief.md`. The chat that
produced the handoff evolved the brief substantially: "burn" was **inverted** —
the cache is a single **cooling clock** (time since last leaf write vs. TTL), not a
spatial gradient; cost is the cold-cache re-warm, not destruction; and a **Mind**
token-ledger was added (resident context = the cached prefix = what re-warms).

## Decisions (locked with the user)

- **Vanilla TS, no React.** The only continuously-changing element is the 1 Hz
  cooling clock. React would re-render the whole tree (100s of turns) every second —
  exactly the idle-compute waste we reject. Browser DOM is already damage-tracked, so
  the TUI's *flashing* class of bug does not apply; the real concern is idle CPU.
- **Cooling-model rendering pattern.** Build the DOM once. The 1 Hz tick writes a
  single CSS custom property (`--temp`, and `--temp-color` via `color-mix`) on the
  root plus one countdown text node + the clock-arc dash; CSS cascades the rest.
  Idle ≈ one property write/sec. Interaction handlers mutate only the affected
  subtree — never a full re-render.
- **Stub the backend gaps.** Live where the backend already serves it; faithful
  placeholder data (with a marked seam) where it does not.

## Live vs. stub boundary

| Surface | Source |
| --- | --- |
| Forest (session list) | **Live** `/api/sessions` (real names; glyph shape/branches stubbed) |
| Furnace expanded pane | **Live** pty/xterm WebSocket (preserved from current `main.ts`) |
| New-session flow, projects datalist | **Live** `/api/projects`, `/pty?new=` |
| Leaf send | **Live** `sendPrompt` (bracketed-paste + Enter) into the pty |
| Manuscript transcript | **Stub** — sample 132–137 window; seam `loadSession()` → future structured-JSON endpoint |
| Mind ledger / per-turn deltas | **Stub** (`MIND`, `MIND_DELTAS`) |
| Fork / re-warm cost numbers | **Stub** (computed from stub `forkReading`) |
| Tool-call diffs | **Stub** |
| Cooling clock | **Mechanism real** (ticks, relights on send/fork); seed idle mocked |

## Module layout (`web/src/`)

- `cooling.ts` — pure cache model: `cacheReading`, `forkReading`, `fmtClock`, `fmtK`,
  `prefixTokensTo`, `dropsAt`, temp→color. **TDD'd** with `node --test`.
- `theme.ts` — `furnace`/`paper` palettes as CSS custom properties; `FONTS`.
- `data.ts` — the live/stub seam: `loadForest()` (live), `loadSession()` (stub),
  `MIND`, `MIND_DELTAS`.
- `marks.ts` — SVG marks (role/leaf/fork/norgie/mind/woland, forest glyph, sparkline).
- `clock.ts` — the Furnace cooling clock + its 1 Hz controller.
- `mind.ts` — pinned Mind strip, floating ledger, per-turn margin diff.
- `manuscript.ts` — Exchange render, fold-in-gutter, edit-on-ink, fork banner,
  tool-call introspection, leaf input.
- `shell.ts` — masthead (woland fronts, eigen engine), Forest, Furnace pane,
  cold-fork confirm, fork toast.
- `main.ts` — compose, own interaction state, mount; **preserve** pty WebSocket,
  SSE follow, new-session, dev live-reload.

## Interaction model

- **Fold vs. edit by target.** Gutter (caret + turn no.) folds; prose ink edits.
- **Edit-in-place:** `contenteditable`, assistant block dims, fork banner; Enter
  commits, Esc cancels.
- **Commit → fork:** warm ⇒ instant fork + toast + relight; cold ⇒ `ColdConfirm`
  modal → confirm → toast + relight.
- **Leaf:** Enter sends → relights furnace → toast (and, when live, `sendPrompt`).
- **Mind:** strip pinned under the clock (persists on scroll); click → floating
  ledger; `per-turn Δ` lens swaps margins from cost-reading to mind-delta.
- **Furnace:** norgie/heat-bar ↔ real xterm; theme toggle flips `data-theme`.

## Verification

- `node --test` on `cooling.ts` (the logic most likely swapped for real telemetry).
- esbuild bundle compiles clean.
- `just dev` smoke test — only with explicit user OK (README: no browser unless asked;
  the default pty is a shell, so no tokens spent).

## Out of scope (follow-ups)

- Structured-JSON transcript endpoint to make the Manuscript live.
- Real token accounting / KV-cache telemetry behind the Mind and cost numbers.
- Eviction/curation (prune resident context → fork).
