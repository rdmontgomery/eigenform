# crates/render — design (v0.1)

Status: shipped 2026-06-03 (TDD). Implements build step 3 (text projection); json/html
deferred. See [`project_render_defer_schema`] in auto-memory and the foundation design's
"CLI is the browser's mirror" rule.

## Principle

Render is **projection-based**: commands build an internal **View tree**, and renderers
project it to `text` (now) / `json` / `html` (later). The View tree is an internal,
refactor-freely IR — **not a published schema**.

**Deliberate deferral.** We do not lock a json schema or stabilize the data model yet;
it will drift as new visualizations emerge. json/html become projections when a real
consumer exists (browser play reveals the data shape; the daemon brings html). v0.1 ships
`text` only. The `--render` flag exists but errors clearly on `json`/`html`
("deferred until we render in the browser"). Committing to a json schema now would be
committing to a fixed way of seeing before seeing anything — the paralysis failure mode
the project is organized against.

## View IR

```rust
pub enum View {
    Document { title: String, body: Vec<View> },
    Tree(Vec<Node>),
}
pub struct Node {
    glyph: Option<String>,   // role glyph / status dot
    text: String,            // primary line
    marker: Option<String>,  // trailing annotation, e.g. "← leaf"
    children: Vec<Node>,
}
```

New variants (Section, Table, KeyValues, Text) land as views need them. `render_text`
walks the tree drawing box-drawing connectors (`├─ └─ │`).

## First view: the session turn-tree

`session_view(&Session) -> View` (takes a parsed `eigen_surgery::Session`):

- **Group by exchange.** User turns at top level; assistant/system replies nested beneath
  the user turn that prompted them.
- **Glyphs:** `●` user, `◇` assistant, `·` system.
- **Previews:** one line, whitespace-collapsed, truncated to ~60 chars with `…`. Content
  is pulled from `message.content` (string, or concatenated text blocks).
- **System rows:** shown muted, labelled with their `turn_duration` (e.g. `4.2s`).
- **Noise hidden:** thinking-only assistant rows (no text) and non-`turn_duration` system
  rows are omitted.
- **Resume leaf:** marked `← leaf`. If the literal leaf is a hidden row, the marker falls
  back to the last visible turn.

CLI: `eigen sessions show <uuid|prefix|path> [--render text]` (uuid resolution via
`crates/forest`).

## Second view: the fork diff (added 2026-06-03)

`fork_diff_view(source: &Session, fork: &Session) -> View` — a **side-by-side** diff
aligned by turn uuid (`fork_at` preserves uuids): kept turns on both columns, dropped
left-only (`-`), injected right-only (`+`), edited both with differing content (`~`).
Header summary (kept/dropped/injected/edited counts) and a `leaf:` move line. CLI:
`eigen sessions diff <a> <b>`, and `eigen surgery fork … --diff` (diff to stderr so
stdout stays the new uuid for scripting). Uses the shared visibility/leaf helpers, so the
diff shows exactly the turns `show` would.

## Tests

Unit/integration in `crates/render/tests/` (projection connectors, exchange grouping,
truncation, whitespace collapse, hidden-row omission, leaf fallback) and end-to-end CLI
tests in `crates/eigen-cli/tests/cli_sessions.rs`. Verified by hand on real local sessions
including tool-heavy and self-referential ones.

## Non-goals (v0.1)

- No json/html projection (deferred — see Principle).
- No per-turn uuid display, no content drill-in, no diff view (later).
- No session discovery/indexing (that's `crates/forest`).

[`project_render_defer_schema`]: ../../../.claude/projects/-home-rdmontgomery-projects-eigen/memory/project_render_defer_schema.md
