# Woland: new-session, compact density, and markdown prose — design

Date: 2026-06-06

Three additions to the redesigned workbench surface. The redesign (`4d3a2f3`)
restyled the shell but dropped the new-session affordance and never had a density
control or markdown rendering. The backend for new sessions is fully intact, so
two of these are UI-only; the third is a new pure module.

## 1. New session from the Forest

**Choice:** a `⊕` glyph in the Forest header (`Forest · forking paths`) that
reveals an inline directory picker pinned below the header.

The backend is already live and untouched:
- `GET /api/projects` returns recent-first distinct project cwds (daemon `lib.rs`).
- The pty websocket accepts `?new=<cwd>` and spawns `claude` there, watching the
  projects dir for the new session's `.jsonl` and reporting its uuid via a
  `{type:"session", uuid}` control frame.
- `main.ts` already handles that frame in `onSessionBorn(uuid)` (→ `followManuscript`
  + `refreshForest`). The websocket `onmessage` already routes it.

**Shell (`shell.ts`).** `buildForest(onSelect, onNew)` gains:
- an `onNew: (cwd: string) => void` callback param, and
- a returned `setProjectDirs(dirs: string[])` to fill the `<datalist>`.

The header becomes a flex row: the eyebrow on the left, a small `⊕` ghost-glyph
button on the right (`title="new session"`). Clicking toggles an inline picker
below the header: `<input list="project-dirs">` + `<datalist id="project-dirs">`,
styled with `--mono`/`--faint`/`--line` so it reads as chrome. Enter (or a launch
glyph) submits a non-empty value → `onNew(cwd)` and collapses the picker. Escape
collapses without launching.

**Wiring (`main.ts`).** `onNew(cwd)`:
```
activeSession = null; currentUuid = null;
connectPty('?new=' + encodeURIComponent(cwd));
furnace.setOpen(true);   // watch claude boot
```
`onSessionBorn` then takes over when the daemon reports the uuid. At startup,
fetch `/api/projects` once and call `forest.setProjectDirs(dirs)`.

No backend changes.

## 2. Compact density toggle

**Choice:** a binary toggle (normal ↔ compact) in the masthead beside the theme
toggle, persisted to `localStorage`. Tightens both type and padding.

**State + persistence (`main.ts`).** `density: "normal" | "compact"`, read at
startup from `localStorage["woland.density"]` (default `"normal"`), written on
toggle. This is the first localStorage in the codebase, so add a tiny `prefs`
helper (`get`/`set`) rather than scatter raw calls.

**Masthead (`shell.ts`).** A second `button.ghost` labelled `Aa`
(`title="toggle compact density"`), mirroring the existing `onTheme`/`setTheme`
shape: `buildMasthead(..., onDensity)` + a `setDensity(d)` to sync label/active
state. Clicking flips `data-density` on the root element and calls back.

**CSS (`woland.css`).** One multiplier var drives type:
```
:root { --scale: 1; }
:root[data-density="compact"] { --scale: 0.9; }
```
Convert content-surface font sizes to `calc(<base>px * var(--scale))`:
`.prose`, `.prose.assistant`, `.leaf textarea`, `.tool .body`, forest rows,
masthead `.sess`, eyebrows, clock readout. Leave 9px meta labels alone (they'd
get unreadable). Padding tightens via a curated override block:
`:root[data-density="compact"]` reduces the big layout paddings — `.ms-head`
(44px → ~24px horizontal), `.forest` padding, `.leaf` padding-top, row padding,
prose rhythm. (Override block, not calc, because the paddings are few and
deliberate.)

**Terminal.** xterm's font is JS-only: on toggle set
`term.options.fontSize = compact ? 11 : 12` and refit if the Furnace is open.

## 3. Inline + block markdown in assistant prose

**Choice:** full block-level rendering (headings, bullet/numbered lists, fenced
code) plus inline emphasis — applied **only** to `.prose.assistant`. User prose
stays raw editable text; the live pty tail stays mono.

**Renderer (`markdown.ts`, new — pure, TDD'd with `node --test`).** No library,
no `innerHTML` (no XSS surface), built with the codebase `el()` style.
- `inline(text) → (Node | string)[]`: `**bold**`, `*italic*`/`_italic_`,
  `` `code` ``, `~~strike~~` → text nodes + `<strong>`/`<em>`/`<code>`/`<s>`.
  Unmatched/lone markers fall through as literal text (Claude mid-stream emits
  stray `*`/`` ` `` constantly — must never throw or drop text).
- `parseBlocks(text) → Block[]`: ATX headings (`#`–`###`), fenced code (```` ``` ````),
  bullet lists (`-`/`*`/`+`), ordered lists (`N.`), paragraphs (blank-line
  separated; unrecognized lines stay paragraph text).
- `renderMarkdown(text) → Node[]`: maps blocks to `<p>`, `<h2>/<h3>`,
  `<ul>/<ol>` of `<li>`, `<pre class="code">`; runs `inline()` on inline content.
  Fenced code is literal (no inline pass).

**Manuscript (`manuscript.ts:200`).** `.prose.assistant` goes from a single `<p>`
to a `<div class="prose assistant">` containing `...renderMarkdown(e.assistant ?? "")`.

**CSS.** `.prose.assistant` drops `white-space: pre-wrap` (blocks own spacing);
paragraphs (`.prose.assistant p`) keep `pre-wrap` so single newlines inside a
paragraph remain soft breaks. `strong` → `--ink` weight 500 (bold brightens the
ink), `em` → serif italic, inline `code` → `--mono`, `0.86em`, subtle `--raise`
bg. Headings: serif, brightened. Lists: tight rhythm, marker in `--faint`.
`pre.code`: mono panel on `--furnace-bg`, echoing the tool-call body.

## Testing

- `markdown.ts` is pure → TDD with `node --test` (mirrors `cooling.test.ts`):
  emphasis, code, strike, nesting, lone/unmatched markers, headings, both list
  kinds, fenced code, mixed blocks, empty input.
- The two toggles are DOM/websocket wiring → verified by running the app (build +
  manual), as the codebase only unit-tests pure logic.
- `npm run typecheck` must stay clean.

## Out of scope (YAGNI)

- Persisting theme (only the new density pref is persisted now).
- Markdown links/images/tables/blockquotes (add later if Claude's output needs it).
- A multi-step or continuous density control (binary only).
