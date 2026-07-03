# Fable→Opus Downgrade Recovery

> Design — 2026-07-02

## Problem

Fable (the larger model) sometimes **downgrades a live session to Opus** when a
prompt trips the safety guardrail. Often this is an over-eager false positive
(benign security research, dual-use tooling, loaded-but-innocent phrasing), and
the natural recovery is: "let me restate that so it doesn't trip the guardrail"
— and keep the thread on Fable.

eigenform already owns the primitives to do this cleanly (copy-on-fork context
surgery, a session forest, a live daemon hosting the pty). This feature wires
them into an automatic-but-human-in-the-loop recovery: **detect the downgrade,
fork a fresh Fable session truncated to before the offending prompt, stage a
suggested restatement in the input, and never auto-send.**

Scope for this pass: **sessions eigenform launched** (live). The detector is pure
transcript analysis, so imported/external sessions come nearly for free later —
but we build no UI for them now (YAGNI).

## Guiding stance

**Vitality in the moment; reliability at steady state.** The detection signal is
tied to Anthropic's current downgrade-notice wording and *will* break when they
change it. We accept that, isolate it to one named constant, and document where
to look when it breaks. We do not over-engineer robustness before the feature has
proven useful.

**Not evasion.** The staged rephrase optimizes for *removing genuine ambiguity /
stating benign intent* to recover from over-eager downgrades — not for slipping
disallowed intent past a correct refusal. If the rephraser itself declines a
genuinely loaded prompt, that is the system working, and we surface the refusal.

## Section 1 — Detection

Runs on the same transcript parse the daemon already does as each session's JSONL
grows.

- **Primary signal: signature text.** Match the literal downgrade-notice string
  Claude Code writes into the transcript (small regex tolerated). Captured from a
  real downgraded transcript in a spike and pinned to a single named constant
  with a comment pointing at the spike doc + `claude --version`. Updating it when
  Anthropic shifts is a one-line change.
- **Main chain only.** Filter out subagent/sidechain turns (`isSidechain: true`)
  before matching, so Fable spawning an Opus subagent for grunt work never fires
  — that's an intended benign Opus, not a downgrade of *your* thread.
- **Model transition is corroboration, not trigger.** "Main-chain assistant model
  went Fable→Opus at the same turn as the marker" can confirm later; the text
  match is what fires. Match "not-Fable after Fable" rather than hardcoding an
  exact Opus id so new Opus ids don't silently break detection.
- **Offending turn** = the user prompt tied to the downgrade notice (the turn
  whose response triggered it). That is what we `fork_before`.
- Emit one `DowngradeEvent { session_uuid, offending_turn, at }` per session,
  deduped (a session that stays on Opus for 10 turns fires once).

## Section 2 — Fork, rephrase, force Fable

On a `DowngradeEvent` the daemon does three things:

1. **Fork (reuse).** `surgery::fork_before(src, offending_turn)` → copy-on-fork a
   new session into the same project dir. Source untouched. The branch transcript
   ends cleanly at the completed turn *before* the offending prompt.
2. **Rephrase (headless `claude -p`, not the API).** Shell out to a one-shot
   `claude -p` in the same cwd, feeding the offending prompt's text with an
   instruction to restate it to remove ambiguity / state benign intent while
   preserving the actual ask, and to say so plainly if the ask is genuinely
   disallowed. We shell out rather than take an API-key dependency because the
   daemon already only ever runs `claude` — stays key-free and offline-friendly.
   On decline/error: stage the verbatim prompt + the refusal note; the fork still
   works.
3. **Force Fable on resume.** The existing fork route resumes via
   `claude --resume`, which inherits the (now Opus) model. Add a model override so
   the branch resumes on `--model fable` (exact flag verified in the spike). This
   is the one bit that *must* work — a branch that resumes back on Opus is
   pointless — so it is **spike-gated before any UI is built.**

The rephrase rides the existing fork contract: delivered into the input as staged
`text`, **never written to the branch file, never auto-sent.**

## Section 3 — Surfacing (live, auto-open)

The daemon does the fork + rephrase eagerly (both cheap, non-destructive), then
pushes a control message to the client:
`{ kind: "downgrade_recovered", src_uuid, branch_uuid, offending_turn, staged_text, note }`.

- The new Fable branch is a real session on disk → the forest already renders it;
  we tag it (`fable-retry` badge) so it reads as the rephrase lane, not a stray
  fork, as a sibling of the source at the fork point.
- **Auto-open** the branch in the Furnace: cursor in the input, `staged_text`
  pre-filled, model pinned to Fable. Nothing sends. One nuance: suppress the
  auto-jump if the user has typed in the source session within the last beat, so
  it never eats a keystroke mid-sentence — default is jump.
- If the rephraser declined, the note says so and the box holds the verbatim
  prompt for hand-editing.

**Deliberately not doing (YAGNI):** auto-send; forking a session's downgrade more
than once; the external/imported-session path; any model-driven guardrail
evasion.

## Section 4 — Error handling & testing

Each failure degrades instead of breaking:

- **Marker not found / wording changed** → detector never fires. No crash, no
  false branch. Signature constant + spike doc is where you look.
- **Rephraser declines/errors** → stage verbatim prompt + note; fork still works.
- **Fable pin fails** → spike gate; if it can't hold, we don't ship the auto-fork.
- **`fork_before` fails** (offending turn isn't a completed boundary) → badge with
  "couldn't stage a retry," no auto-open.

Testing:

- **Unit (surgery/detector), pure functions, no daemon:** fixture JSONLs — a
  clean Fable→Opus downgrade with the marker (fires once); a benign Opus subagent
  sidechain (must NOT fire); an always-Opus session (must NOT fire); a session
  that stays downgraded many turns (fires once).
- **Route test (daemon):** the fork path returns a branch uuid, source untouched,
  staged `text` never written to the branch file.
- **Spike (manual, gated):** capture the real marker string; confirm
  `--model fable` pins a resumed session; record with `claude --version`.
- **Visual:** headless Chromium is blocked in this WSL env — verify badge /
  auto-open via throwaway daemon + curl + bundle grep.

## Reused vs new

**Reused as-is:** `surgery::fork_before`, the `POST /api/session/:uuid/fork`
route + client resume flow, forest rendering of on-disk sessions, the daemon's
JSONL watcher.

**New:** the signature-based downgrade detector (+ `DowngradeEvent`); the
`claude -p` rephrase step; the `--model fable` resume override; the
`downgrade_recovered` control message + `fable-retry` badge + auto-open handling.

**Spikes gating the build:** (1) exact downgrade-notice marker string; (2)
`claude --resume --model fable` pins the model. Both recorded per the
`vetting-claude-internals` habit.
