# crates/forest — design (v0.1)

Status: designing 2026-06-03. Implements build step 5 (session side) — enough to stop
pasting paths: resolve a session by uuid globally, and list recent sessions per project.
Skills/memory indexing under the same crate is deferred.

## Goal

`eigen sessions show <uuid>` and `eigen sessions list` work without file paths. A uuid is
globally unique, so `show` resolves machine-wide; `list` answers "what have I worked on
recently" — scoped to the current project, last 7 days by default.

## Data

```rust
pub struct SessionRef {
    pub uuid: String,
    pub path: PathBuf,
    pub cwd: PathBuf,             // decoded project cwd
    pub recency: DateTime<Utc>,   // last tail timestamp, else file mtime
    pub title: Option<String>,    // last ai-title, else last-prompt snippet
}
```

## API (on top of `eigen-projects`)

- `resolve(projects_dir, query) -> Result<PathBuf>` — find `<query>*.jsonl` across ALL
  projects. Exact uuid wins; else unique prefix (git-style). Errors: `NotFound`,
  `AmbiguousPrefix(Vec<SessionRef>)` (carries candidates so the CLI can list them). A
  literal existing path is handled by the CLI before calling resolve, so paths still work.
- `list(projects_dir, scope, since, now) -> Result<Vec<SessionRef>>` — enumerate, tail-peek
  each for `recency` + `title`, filter to the window, sort recent-first. `scope` =
  `Project(cwd)` (default, current project) or `AllProjects`. `since` = `Option<Duration>`
  (`None` = all time). `now` is injected for deterministic tests.

## Tail-peek (the recency signal)

A **byte-stream** read, not line-based (JSONL has no line index):

1. `seek(End - 64KB)`, read the window, split on `\n`. Discard the leftmost piece (a
   partial row at the chunk boundary).
2. Scan complete lines from the last backward; JSON-parse each; take the first carrying a
   `timestamp`. Unparseable fragments are skipped — a mid-line fragment is never mistaken
   for a row.
3. `title`: last `ai-title`'s `aiTitle`, else a snippet of the last `last-prompt`'s
   `lastPrompt`, from the same trailing block.
4. **Escalation**: if the window yields no timestamped row and didn't cover the whole
   file, re-read a doubled window, ultimately a full read, finally fall back to file mtime.
   Normal sessions cost one 64KB read; only an oversized final turn pays more.

No persisted index. The mtime pre-filter (tail-peek only files whose mtime is within the
window + slack) and a sidecar index are **deferred scale optimizations**, added when a
real corpus makes listing slow — and `log`-ged when any cap kicks in (no silent
truncation). Keeping recency on-the-fly avoids freezing a derived schema while the data
model is still fluid (cf. render's deferred json schema).

**Forks sort by conversation time**, by design: a fork's appended rows carry no
timestamp, so its `recency` is the original last turn's time, not the fork-creation time.

## Render

`render::sessions_view(&[SessionRef], now) -> View` — one row per session,
`<short-uuid>  <relative-time>  <title>`, with a project column under `--all-projects`.
Relative-time (`2h ago`, `3d ago`) is computed in render. **Newest at the bottom**: sort
recent-first internally, emit oldest→newest so the latest sits nearest the prompt.

## CLI

```
eigen sessions show <uuid|prefix|path> [--render text]
eigen sessions list [--since 7d|all] [--all-projects] [--cwd <dir>] [--render text]
```

- `show`: if the arg is an existing file, use it; else `resolve` globally. On
  `AmbiguousPrefix`, print the candidates (`<short>  <title>`) and exit non-zero.
- `list`: defaults to current project (via `project_for_cwd`) and `--since 7d`.
  `--all-projects` widens scope; `--cwd <dir>` targets another project. `--since all`
  disables the window.

New dep: `chrono` (timestamp parse, duration math, relative formatting; reused later by the
token economy).

## Tests

- forest: resolve exact / unique-prefix / ambiguous / not-found; tail-peek picks the last
  *timestamped* row past trailing state rows; tail-peek title precedence; window filter +
  sort with injected `now`; oversized-final-turn escalation.
- render: `sessions_view` rows, relative time, newest-at-bottom, project column.
- CLI end-to-end: `show <uuid>` resolves; ambiguous prints candidates; `list` windows and
  scopes.

## Non-goals (v0.1)

- No persisted index, no skills/memory indexing in forest yet.
- No keyword/content search (design's `--keyword`) — later.
- No json/html (render defers those).
