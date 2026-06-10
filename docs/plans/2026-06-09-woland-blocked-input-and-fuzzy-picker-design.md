# woland: blocked-input surfacing + fuzzy new-session picker

**Date:** 2026-06-09
**Branch context:** `woland-forest-affordances`
**Spike:** `notes/spikes/08-tui-selector-widget.md` (CONFIRMED)

Three reported pains, two subsystems:

1. *New-session fill-ins are dumb* — only directories you've already had Claude sessions in,
   substring-matched by a native `<datalist>`.
2. *First message from a new session "didn't send"* — text appeared in claude's input but
   nothing started.
3. *Interactive questionnaires don't surface in the Manuscript* — you have to drop to the
   Furnace to answer them.

(2) and (3) are the same subsystem: the Manuscript only ever sees the JSONL and the live pty
tail, and `termTail()` deliberately strips TUI chrome via `isChrome()`. Every blocked-choice
moment claude shows — trust dialog, permission prompt, AskUserQuestion, plan approval — is that
stripped chrome, so it's invisible. The "didn't send" was woland blind-sending its first prompt
into the **trust dialog**: the `\r` confirmed the dialog instead of submitting the prompt.

---

## Subsystem A — fuzzy new-session picker

### Backend (`crates/daemon`, `crates/projects`)

- Add `code_root: Option<PathBuf>` to `Config` (default `~/projects`; overridable via
  `EIGEN_CODE_ROOT` env / CLI flag). `None` ⇒ only recents (current behavior).
- Rework `projects_route` to return a merged, de-duped, ordered candidate list:
  - recent session cwds (`eigen_forest::list`, recency order) → `{ path, recent: true }`
  - immediate subdirs of `code_root` not already present → `{ path, recent: false }`
  - Response shape changes `string[]` → `[{ path, recent }]`.
- CLI mirror (memory: *CLI mirrors browser output*): `eigen projects` prints the same merged
  list (path + a `·recent` marker), so the picker stays reproducible from the terminal.

### Frontend (`web/src/shell.ts`, new `web/src/fuzzy.ts`)

- Replace the native `<datalist>`/`<input list>` combobox in `buildForest` with a small custom
  fuzzy combobox:
  - Fetch candidates once (extend `loadProjectDirs`); keep them in memory.
  - `fuzzy.ts`: subsequence match + score (favor basename hits, contiguous runs, word
    boundaries; `recent` wins ties). Pure function, unit-tested.
  - Render a results dropdown under the input; ↑/↓ move highlight, Enter launches the
    highlight, Esc closes. Enter with free text and no highlight still launches the typed
    path (arbitrary dirs preserved). Each row: basename bold + dimmed parent; a faint "recent"
    tag.
- `onNew(cwd)` unchanged downstream.

---

## Subsystem B — surfacing blocked input as inline clickable choices

One **interaction detector** covers all four prompt types because (per spike 08) they share one
widget: a numbered option list, the selected row marked `❯` (U+276F), navigated by arrows,
confirmed by Enter, cancelled by Esc.

### Detection (`web/src/main.ts` + new `web/src/interaction.ts`)

- On debounced pty output (reuse `onPtyOutput`), scan the xterm buffer tail (rows via
  `translateToString`, like `termTail`) for the selector signature:
  - ≥2 consecutive rows matching `/^\s*❯?\s*\d+\.\s+\S/`, exactly one containing `❯`.
  - Strong corroborator: a nearby `Enter to confirm` / `Esc to cancel` footer.
- Parse `{ question, options: [{n, label}], selectedIndex }`:
  - `selectedIndex` = the `❯` row.
  - `question` = non-empty rows above the option block (trimmed; for trust, the safety
    paragraph + path).
- `interaction.ts` is a pure `parseSelector(rows: string[]) → Selector | null`, unit-tested
  against the spike's captured rows.

### Surfacing (`web/src/manuscript.ts`)

- A new Manuscript region (sibling to the live "responding" region, same insert point above the
  leaf): renders the question text + each option as a button. The currently-`❯`-selected option
  is visually marked.
- While a selector is active, suppress the normal live-streaming region (claude isn't streaming
  prose — it's waiting). Hide/remove the region when the selector clears (buffer no longer
  matches).
- An explicit "answer in the Furnace ↓" escape hatch remains (the detector can misparse; never
  trap the user).

### Answering (`web/src/main.ts`)

- Clicking option *k*: send `(k − selectedIndex)` arrow keys (`\x1b[B` down / `\x1b[A` up) then
  `\r`, over the existing pty websocket. Mirrors human nav; no number-key assumption.
- After sending, optimistically clear the selector region; the next buffer scan reflects the
  real new state (re-surface if claude shows a follow-up selector).

### Send-bug fix (the payoff)

- The detector yields a clean **"claude is at the text input, not a selector"** signal.
- Replace the blind quiet-timer first-send: gate `schedulePendingSend` on *(input prompt visible
  AND no active selector)*. A new session in an untrusted dir thus surfaces the trust dialog
  first; once answered and the input prompt appears, the pending first prompt fires into the
  real input.
- Keep a longer paste→Enter delay as defense-in-depth, but readiness-gating is the real fix.

---

## Error handling / fragility

- TUI parsing is format-fragile by nature; this is why it's spike-gated and why `isChrome()`
  already exists as the inverse. The detector is conservative (requires the multi-row signature
  + ideally the footer) so prose that merely looks like a list doesn't trigger it.
- Always leave the Furnace reachable as the manual fallback; a misdetection degrades to "answer
  in the terminal", never to a stuck session.
- If `code_root` doesn't exist or isn't readable, fall back to recents-only (current behavior).

## Testing

- Rust: `crates/projects` (or daemon) unit test for the merged candidate list (recents first,
  subdir union, dedup); CLI test for `eigen projects`.
- TS: `fuzzy.test.ts` (ranking), `interaction.test.ts` (`parseSelector` over the spike's real
  trust-dialog rows + a synthesized 3-option permission block; negative cases: prose, a single
  numbered line).
- Manual (user, holds token authority): a real new session in an untrusted dir — confirm the
  trust dialog surfaces inline, clicking "Yes" proceeds, and the queued first prompt then sends.

## Sequencing

1. Subsystem A end-to-end (no token spend) — verifiable immediately.
2. Subsystem B against spike 08's confirmed patterns — `parseSelector` + region + answer +
   send-gate. Manual verification with one real new session.

## Open details (non-blocking)

- Permission / AskUserQuestion / plan selectors are inferred from the shared widget; number-key
  shortcuts and bordered-box variants verified lazily when building them.
- AskUserQuestion's structure is also in the JSONL — a later enrichment could render it from
  structured data instead of buffer-scraping.
