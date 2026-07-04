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

- **Primary signal: signature text on a synthetic turn.** The downgrade notice is
  delivered as an `assistant` turn with `message.model == "<synthetic>"` carrying
  a human-readable string (confirmed by enumerating local transcripts — spike 10).
  Match the guardrail string against a single named constant (small regex
  tolerated). **Scrubbed in for now:** no guardrail-safety string exists in local
  history yet (only session-limit and API-error synthetics were observed), so the
  constant is a placeholder captured from the first live occurrence, pinned with
  its `claude --version`. Updating it when Anthropic shifts is a one-line change.
- **Session-limit fallback is NOT a guardrail downgrade.** Hitting the Fable
  session limit *also* produces a `<synthetic>` notice and a Fable→Opus
  transition (`You've hit your session limit · resets …`). The detector must match
  the specific guardrail wording, not the transition — confirming the
  signature-over-inference choice.
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
3. **Resume lands on Fable for free (spike 10).** Resume derives its model from
   the transcript's **last recorded assistant turn**, not sticky session state.
   Since `fork_before` truncates to a completed Fable turn before the offending
   prompt, `claude --resume` continues on Fable with **no flag needed** — this was
   the earlier "must force `--model fable`" gate, now dissolved. We keep passing
   `--model fable` as a cheap belt-and-suspenders (explicit intent; robustness if
   Anthropic changes resume-model derivation), but it is not required for
   correctness. Verified against `claude 2.1.199` in
   `notes/spikes/10-resume-model-derivation.md`.

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
- **Fable pin** → not a failure mode: resume follows the truncated transcript onto
  Fable on its own (spike 10); `--model fable` is a redundant guard.
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

**Spikes:** (1) resume-model derivation + `--model` pinning — **DONE**, spike 10,
which dissolved the Fable-pin gate. (2) exact guardrail marker string — scrubbed
in for now; capture from the first live occurrence and record per the
`vetting-claude-internals` habit.
