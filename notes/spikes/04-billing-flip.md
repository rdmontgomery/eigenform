# 04 — Billing flip on `--print` / SDK

**Claim:** Interactive `claude` draws from the team token plan; `claude --print` (non-interactive) and SDK invocations flip to usage-based per-token billing.
**Status:** PENDING
**claude version:** 2.1.161
**Date:** —

## Procedure

This spike does not need code; it needs observation. Two paths to confirm.

**Path A — `/cost` observation.**

1. Start a fresh interactive session, send one short turn, note `/cost` reading. Exit.
2. Run `claude --print "say hi"` (one short turn). Note any `/cost` or stderr cost output.
3. Compare. If the `--print` invocation reports a different billing source than the interactive one, claim is supported.

**Path B — billing dashboard.**

1. Note the dashboard's current "team plan tokens used" and "usage-based credit balance" snapshots.
2. Run a moderately-sized interactive turn (~5k input tokens of pasted content). Refresh dashboard.
3. Run a comparable `claude --print` invocation. Refresh dashboard.
4. Compare which counter moved each time.

(Path B is the authoritative one. Path A is a quick smell test.)

## Result

(paste real output / screenshots here)

## Implication

CONFIRMED: production engine path is `pty` only. The daemon spawns `claude` interactively in a pty per session. `--print` is reserved for off-path enrichment where billing is acceptable (one-shot fingerprint extraction, batch embeddings, etc.) — and even there, we may prefer local Ollama to avoid the spend entirely.

REFUTED (`--print` bills to plan too): we lose nothing — pty is still the right choice for the interactive overlay, but we gain the option of using `--print` for short non-interactive helpers without burning credits. Architecture unchanged, options expand.

PARTIAL (e.g. `--print` bills to plan up to some quota, then flips): record the quota and the flip behavior, since the daemon may need to surface it.
