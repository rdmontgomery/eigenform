# 10 — Resume model derivation (and Fable pinning)

**Claim:** When `claude --resume` reopens a session, the model it continues on is
derived from the transcript's **last recorded assistant model**, not from sticky
per-session state. Therefore a fork that truncates a downgraded session back to a
completed Fable turn resumes on Fable **without** any `--model` flag. An explicit
`--model <alias>` still pins deterministically when supplied.
**Status:** CONFIRMED
**claude version:** 2.1.199 (Claude Code)
**Date:** 2026-07-02

## Why this matters

The Fable→Opus downgrade-recovery feature forks a session to just before the
offending prompt and resumes it, wanting the branch to stay on Fable. Open
question: must we force `--model fable` on resume, or does truncating the JSONL
suffice? This spike settles it, and doubles as verification that `--model`
composes with `--resume`.

## Procedure

All runs headless (`-p`), throwaway cwd under the scratchpad, trivial prompts.

1. **Flag composes / pins.** Create a session `--model opus`; last recorded
   assistant model = `claude-opus-4-8`. Resume it with `--model sonnet`; the new
   turn recorded `claude-sonnet-5`. → `--model` pins a resumed session regardless
   of transcript.
2. **No-flag resume inherits.** Resume the opus session with no `--model`; new
   turn = `claude-opus-4-8` (followed the transcript's last model).
3. **Truncation follows the transcript (the decisive test).**
   - Create a session on `--model sonnet` (last recorded = sonnet).
   - Resume-append with `--model opus` (transcript now sonnet → opus, last = opus).
   - Truncate the JSONL to drop the opus turns, leaving the last recorded
     assistant model = `claude-sonnet-5`.
   - Resume with **no** `--model`.

## Result

```
step 1: opus session, resume --model sonnet  -> new turn recorded claude-sonnet-5
step 2: opus session, resume (no flag)        -> new turn recorded claude-opus-4-8
step 3: truncate to sonnet-last, resume (no flag) -> new turn recorded claude-sonnet-5
```

Step 3 is decisive: the session had just produced an Opus turn, but after
truncating back to a Sonnet boundary, no-flag resume continued on **Sonnet**, not
the "sticky" Opus. Resume reads the model from the transcript's last recorded
assistant turn.

## Implications for downgrade recovery

- `surgery::fork_before` already truncates to a completed turn before the
  offending prompt — i.e. a **Fable** boundary. Resuming that fork lands on Fable
  with **no flag needed.** The "force Fable" step drops from a spike gate to an
  optional determinism guard.
- Keep `--model fable` available as a cheap belt-and-suspenders (explicit intent;
  robustness if Anthropic changes resume-model derivation) but it is not required
  for correctness. Aliases confirmed: `fable`, `opus`, `sonnet`, or full names
  like `claude-fable-5`.
- Edge case to respect: the fork MUST end on a real completed assistant turn (the
  existing fork contract already requires this). If the last recorded model were
  `<synthetic>` or absent, resume-model derivation is unspecified — not a concern
  while we fork to a completed Fable turn.

## Related finding (folded in from the same session)

Enumerating `message.model == "<synthetic>"` assistant turns across local
transcripts shows the downgrade/interrupt notice is delivered as a **synthetic
assistant message carrying a human-readable string**. Observed strings: session
limit (`You've hit your session limit · resets 4pm …`), API errors, and the
benign `No response requested.` A real *guardrail-safety* downgrade string was
**not** present locally, so the detector's guardrail signature must be scrubbed in
and captured from a live occurrence. Confirms detection must match the specific
guardrail wording, not merely a Fable→Opus model transition (session-limit
fallback produces the same transition). See the downgrade-recovery design.
