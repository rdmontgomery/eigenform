# 08 — claude TUI selector widget (trust / permission / AskUserQuestion / plan)

**Claim:** When claude blocks on a discrete choice (trust dialog, permission prompt, AskUserQuestion, plan approval) it renders one shared widget — a numbered option list with the selected row marked by `❯`, navigated by arrow keys and confirmed with Enter — and that widget is legible from the xterm.js buffer grid (not just raw bytes), so woland can detect it, parse the question + options + current selection, and answer it by sending arrow keys + `\r`.
**Status:** CONFIRMED (trust dialog, directly) / INFERRED (permission, AskUserQuestion, plan — shared widget; number-key shortcuts unverified)
**claude version:** 2.1.169 (Claude Code)
**Date:** 2026-06-09

## Procedure

Zero tokens — the trust prompt renders before any API call, and we never confirm it.
`notes/spikes/08-tui-selector-widget.py` forks `claude` in a fresh (untrusted) temp dir under
a real pty (TERM=xterm-256color, 40×100), pumps output ~3.5s to capture the first render, sends
Down (`\x1b[B`) then Up (`\x1b[A`) to test navigation, captures after each, then SIGKILLs
(never presses Enter). Raw bytes → `/tmp/eigen-spike-trust-raw.bin`.

```
python3 notes/spikes/08-tui-selector-widget.py
```

## Result

Real output (repr of the final render; `\x1b[NG` = absolute-column moves — claude lays the
TUI out by cursor positioning, no alt-screen):

```
…❯\x1b[4G\x1b[38;2;153;153;153m1.\x1b[7G\x1b[38;2;177;185;249mYes,\x1b[12GI\x1b[14Gtrust\x1b[20Gthis\x1b[25Gfolder\x1b[39m
\x1b[4G\x1b[38;2;153;153;153m2.\x1b[7G\x1b[39mNo,\x1b[11Gexit
…\x1b[2G\x1b[38;2;153;153;153mEnter\x1b[8Gto\x1b[11Gconfirm\x1b[19G·\x1b[21GEsc\x1b[25Gto\x1b[28Gcancel…
```

Which lays out in the terminal grid as:

```
❯ 1. Yes, I trust this folder
  2. No, exit

Enter to confirm · Esc to cancel
```

Findings:
- **Selection marker:** the active row carries `❯` (U+276F) in `rgb(177,185,249)`; numbers
  render as `N.` (dimmed `rgb(153,153,153)`). Non-selected rows have no `❯`.
- **Navigation:** sending Down (`\x1b[B`) redrew with `❯` moved off "Yes" onto "No, exit"
  (`…\x1b[7GYes, I trust this folder` un-cursored; `❯\x1b[7GNo, exit` cursored). Arrow keys
  move the selection. Confirmed.
- **Confirm/cancel:** the footer states it literally — Enter confirms the current selection,
  Esc cancels.
- **Buffer legibility:** although the raw stream is column-positioned, each option is a single
  logical grid row, so `term.buffer.active` `translateToString` yields `❯ 1. Yes…` / `  2. No…`
  directly. The detector reads rows, not raw bytes.

## Implication

The woland interaction-detector (Subsystem B) is viable:

- **Detect** a blocked-choice state by scanning the xterm buffer tail for ≥2 consecutive rows
  matching `/^\s*❯?\s*\d+\.\s+\S/`, exactly one bearing `❯`; the "Enter to confirm · Esc to
  cancel" footer is a strong corroborating signal. `isChrome()` currently *discards* these rows
  — the detector claims them instead.
- **Parse** `selectedIndex` = the `❯` row; `options[]` = the numbered labels; `question` = the
  non-empty rows above the option block.
- **Answer** option *k*: send `(k − selectedIndex)` × Down/Up arrows then `\r`. Mirrors human
  nav; avoids assuming number-key semantics (the footer advertises only Enter/Esc, so number
  shortcuts are NOT relied on).

**Send-bug root cause (corroborated):** a new session in an untrusted dir blocks on THIS dialog.
woland's blind first-send bracketed-pastes the prompt (eaten by the dialog) then sends `\r`,
which *confirms the trust dialog* (default row = "Yes, I trust") rather than submitting the
prompt. Fix: gate the new-session first-send on detecting the text-input prompt (not a selector),
and route the trust dialog through the same surfaced-choice affordance.

**Open detail (verify lazily, not blocking):** permission prompts and AskUserQuestion use the
same widget but MAY also accept number-key shortcuts and MAY span a bordered box; AskUserQuestion's
structure is additionally in the JSONL (could enrich rendering later). Arrow+Enter is the safe
universal path for all four; confirm number-keys only if we choose to add them.
