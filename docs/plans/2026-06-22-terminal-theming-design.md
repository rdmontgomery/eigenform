# Terminal theming — full-surface schemes

**Date:** 2026-06-22
**Branch:** `terminal-theming` (off `terminal-typography`, merged to `main`)
**Status:** design approved, implementing

## Problem

The terminal's look is hardcoded: one warm-ink palette, a binary light/dark
toggle. The user wants to pick from the well-known terminal color schemes
(Gruvbox, Dracula, Solarized, Nord, …) and have the choice recolor the **whole
app**, not just the terminal pane. Warm-ink should become one theme among many,
not a frame forced on every scheme.

## Model

A **theme** is the iTerm2 surface: 16 ANSI colors + `foreground`, `background`,
`cursor`, `selection` — i.e. a full xterm `ITheme`. ~18 hex values.

Two halves:

1. **Terminal** — the `ITheme` is passed straight to xterm. Trivial.
2. **Chrome** — a pure function `deriveChrome(iTheme) → Record<token, oklch>`
   generates every semantic CSS token the UI already uses, computed in OKLCH:
   - **Surface ladder** — interpolate `background` toward `foreground` in OKLab
     by growing steps: `--bg` → `--surface` → `--raised` → `--raised-2` →
     `--line`. Direction is automatic (a light scheme's fg is dark, so steps
     darken). Dark vs light = `L(background) < 0.5`.
   - **Text ladder** — `--tx` = fg, then `--tx-2/3/faint` fade toward bg.
   - **Accent** — the scheme's `cursor` color (its deliberate signature), with
     `--accent-2`/`--accent-wash` derived. Fallback to the highest-chroma ANSI
     color if the cursor is near-greyscale.
   - **Status** — mapped semantically onto ANSI (which is built for exactly
     this): `--st-error` = red, `--st-success` = green, `--st-running` = yellow,
     `--st-idle` = muted fg. Lightness/chroma normalized into a legible band.
   - **6 session ink hues** — mapped onto ANSI red/yellow/green/cyan/blue/
     magenta, all normalized to one L/C band so per-session badges stay legible
     and uniform on any scheme.

The function is pure and unit-tested. No dependency: a ~40-line self-contained
sRGB↔OKLCH conversion reads `L(bg)` and drives the interpolations.

## Apply / persist / pick

- **Apply** is two live writes (colors only — no re-measure):
  1. Chrome: `setProperty('--bg', …)` for every derived token on
     `document.documentElement`. Inline `:root` overrides beat the stylesheet
     defaults, so `style.css` structure stays intact; the `.theme-light` and
     most of `.term-scope` blocks become vestigial (terminal now shares the
     scheme bg) and get pruned.
  2. Terminal: `term.options.theme = iTheme` on every open tab; new tabs get it
     via the factory. `TERM_THEME` in `pty.ts` stops being hardcoded and becomes
     a parameter — same refactor pattern as the font work.
- **Persist** — `eigenform:term:theme:v2` stores a scheme id. One-time migration
  maps the old `…:theme:v1` (`light`/`dark`) → `warm-ink-light`/`warm-ink-dark`.
- **Pick** — the sun/moon button becomes a theme button opening a popover (reuses
  the font-popover pattern: anchored, outside-click/Esc to close). Each row is
  the scheme name + a live **swatch strip** (bg + 5–6 ANSI colors). The picker
  subsumes the light/dark toggle, since each scheme is intrinsically one or the
  other.
- **CLI parity** — a `themes` subcommand lists bundled ids + swatches as text, so
  a styling bug report stays reproducible.

## File layout (`webterm/`)

- `src/themes/schemes.ts` — curated catalog: `{ id, name, dark, iTheme }[]`.
  Committed, bundled by esbuild. ~15–20 schemes incl. Warm Ink Dark/Light.
- `src/themes/derive.ts` — `deriveChrome(iTheme)` + self-contained sRGB↔OKLCH.
- `scripts/gen-schemes.mjs` — dev-time `.itermcolors`-plist → `schemes.ts`
  converter, run by hand. The seam for "full iTerm2 repo later"; not shipped.
- `src/pty.ts` — parametrize the terminal theme.
- `src/shell.ts` — theme state, picker popover, live apply, v1→v2 migration.
- `src/style.css` — swatch styling; prune dead `.theme-light` / `.term-scope`.

## Testing

Data-driven over **every** bundled scheme (`node --test`, native TS strip):

- `--tx`/`--bg` contrast ≥ legibility threshold (catches an unreadable scheme).
- Monotonic surface ladder: distance-from-bg in L increases across
  bg→surface→raised→line (works for dark and light).
- Every token present and a valid color.
- Dark/light detection matches `L(bg) < 0.5` for known schemes.
- The 6 ink hues stay mutually distinguishable.

An ugly or illegible scheme fails CI, not the user's eyes.

## Build order

1. `derive.ts` + `derive.test.ts` (TDD — the riskiest piece) with 2–3 seed
   schemes.
2. Flesh out `schemes.ts` catalog (+ the gen script).
3. `pty.ts` parametrization.
4. `shell.ts` state, apply, migration, picker popover; `style.css` swatches +
   pruning.
5. `themes` CLI subcommand.
