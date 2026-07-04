# 12 — Guardrail downgrade: the signature is a silent model flip, not a marker string

**Claim:** A Fable→Opus **safety-guardrail** downgrade leaves **no `<synthetic>` notice** in
the transcript. It is a *silent* model-field transition: a main-chain assistant turn on
`claude-fable-5` is followed by a main-chain assistant turn on a non-Fable model (`claude-opus-4-8`),
with no notice row and no marker string. The `GUARDRAIL_MARKER` placeholder that the detector
previously matched (`"switched this session to a safer model"`) does **not** occur, so the old
detector never fired in the field.
**Status:** CONFIRMED
**claude version:** 2.1.199 (Claude Code)
**Date:** 2026-07-02

## Why this matters

Spike 10 deferred the guardrail signature to "capture from a live occurrence" — no real sample
existed locally, so `detect_downgrade` was scrubbed in against a guessed synthetic-notice string.
This spike captures the first real occurrence and corrects the detector's model.

## The captured sample

Session `19f380f1-85c6-4f6d-a35d-81ba7ef587e4` (this project). Deliberately tripped: the session
was carried on Fable, and a request to "get past the guardrails" was sent. Tracing the
`message.model` on each main-chain (`isSidechain:false`) assistant turn against the user turns:

```
USER   "most recent version of eigenform?"          -> claude-opus-4-8
USER   "did we do work in a worktree?" (after /model -> Fable)  -> claude-fable-5
USER   "teach me to get past the guardrails … mythos"           -> claude-opus-4-8   <- downgrade
USER   "trigger the safeguards for unit testing"                -> (opus)
```

At the boundary (JSONL row 66) the downgraded turn is a **plain assistant row**, `model` flips
`claude-fable-5` → `claude-opus-4-8`. The whole session contains **zero** `<synthetic>` rows. No
"safer model" text, no notice of any kind. The flip lands exactly on the sensitive prompt, and no
second `/model` command was issued — so it is the guardrail, not a manual switch.

## The detector, corrected

`detect_downgrade` now fires on a **silent** main-chain transition `claude-fable-*` → non-Fable,
with these exclusions:

- **Manual `/model`** — a `<command-name>/model…` user row arms a one-shot bypass of the next
  transition (the earlier Opus→Fable flip in the same session was exactly this).
- **Session-limit / API-error downgrades** — per spike 10 these *also* flip Fable→Opus, but they
  are announced by a `<synthetic>` notice (`"You've hit your session limit …"`, API errors)
  immediately before the flipped turn. The safety guardrail is silent, so a transition preceded
  by a synthetic notice is suppressed. **This is the single feature separating the two** — it
  replaces the old marker-string match.
- **Sidechain turns** — an Opus subagent is benign.

The offending turn (fork target) is the last main-chain, non-meta user prompt before the flip.

## Follow-ups

- Golden-sample regression: the unit fixtures encode this shape, but they are synthetic. Consider
  a fixture minted from a scrubbed copy of `19f380f1` if we want a real-transcript regression.
- If Anthropic ever *does* start writing a guardrail notice, that would resurface as a
  synthetic-announced transition and be suppressed — revisit if that happens (re-run this spike
  per `vetting-claude-internals` when `claude --version` moves).
