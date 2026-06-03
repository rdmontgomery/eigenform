# woland — design (browser workbench)

Status: designing 2026-06-03. Covers foundation build steps 6–8 (daemon, browser,
affordances). Woland is eigen's browser-based **Claude Code workbench** — the all-seeing-
as-recall cure to the Aleph, not a read-only viewer.

## Vision

A browser where you drive live Claude Code sessions, switch between the work calling you,
and see the conversation **re-rendered as a living document** (collapsible turns/tools,
hover visuals, fork-from-any-turn, custom compaction), with Claude's HTML/markdown output
rendered in place — so discovery happens without bouncing to VS Code or sludging through
walls of terminal text.

## Architecture

```
browser (no-framework TypeScript)        daemon (Rust)
  xterm.js terminal   ⇄ websocket ⇄        pty manager: spawn/attach, stream stdio, resize
  semantic re-render  ⇄ http/sse ⇄         render: forest/tree/diff/transcript views
  forest sidebar      ⇄ http ⇄             forest: list/resolve sessions
  affordance buttons  ⇄ http ⇄             surgery: fork / inject / compact
                                           file-watch: JSONL liveness → push
```

- **JSONL is the source of truth.** The semantic view re-renders from the growing JSONL
  (spike 01: incremental writes). The pty owns only what JSONL can't give live:
  keystrokes, in-flight token shimmer, permission prompts (hard rule #2).
- **Collapse/hover/visuals come from OUR DOM**, rendered from JSONL — not from
  manipulating xterm.js's character grid (which has no turn/tool semantics).
- **Dev affordance — split pane:** left = faithful pty (xterm.js, ground truth), right =
  our semantic re-render. Divergence = a rendering bug, caught instantly.
- **Frontend:** no-framework TypeScript + xterm.js, bundled with esbuild. No SPA runtime.
  The daemon serves the bundle as static assets.
- **Stack:** axum + tokio (http + websocket), `portable-pty` (cross-platform pty),
  `notify` (file-watch). All localhost, single-user.

## Token safety (standing rule)

A live pty IS the engine; spawning `claude --resume` spends team tokens. The pty bridge
drives an **arbitrary command**. We build and test it against a dummy (`bash`/`cat`); real
`claude --resume` launches **only when the user clicks**. Automated tests and agent-run
demos never spawn claude.

## Slice trajectory

**Slice 1 — "just see the rendered pty".** `eigen daemon` serves an xterm.js page; the
browser opens a websocket; the daemon spawns a pty (a **shell**, for a zero-token first
demo) and bridges stdio both ways, with resize. Proves the load-bearing bridge and the
feel. A minimal forest sidebar (from `forest::list`) lists sessions; clicking one is wired
later to `claude --resume <uuid>` in that session's cwd (user-initiated).

**Slice 2 — the semantic transcript.** Right pane: render turns/tools from JSONL as living
DOM (collapsible, hoverable), live-updated as the file grows. Reuses `render`'s view
logic, now as html. The data model forms by feel here (kept internal/fluid — see
`project_render_defer_schema`). Split-pane diffing against the left pty.

**Slice 3+ — affordances.** Fork-from-turn, custom compaction (synthetic-turn surgery),
multi-session management, in-place HTML/markdown rendering of Claude output. Typographic
playground (html-in-canvas, chenglou/pretext, Library-of-Babel easter eggs).

## Testing

- Daemon pty bridge: Rust integration test spawns a **dummy** command (`cat`/`echo`),
  connects a websocket client, asserts stdio round-trips and resize is honored. No claude.
- render html projections: unit tests as with text (golden-ish, but kept fluid).
- Frontend: minimal, verified manually in-browser (the split pane is itself a live check).

## Non-goals (early slices)

- No SPA framework, no client-side routing beyond panes.
- No multi-user / remote / auth (localhost single-user).
- No `--print`/SDK engine path (hard rule #1; pty only).
- No persisted index / frozen json schema (still deferred).
