# spikes

Empirical verification of load-bearing claims. One file per claim.

Format (every spike):

```
# <NN> — <topic>

**Claim:** one sentence.
**Status:** CONFIRMED | REFUTED | PENDING | INCONCLUSIVE | RETIRED
**claude version:** <version>
**Date:** <ISO date>

## Procedure
Exact commands, exact files touched. Reproducible.

## Result
What happened. Paste real output, do not summarise.

## Implication
What this means for the design. If REFUTED, what changes.
```

Spikes 2–4 gate implementation start. Spike 5 (cache TTL) defers to step 9.

RETIRED means the claim may still be true but nothing depends on it anymore; it is skipped by re-vetting (`.claude/skills/vetting-claude-internals`).

Spikes 14–16 cover the OpenAI Codex CLI (`crates/codex`, `.claude/skills/codex-worker`). They record `codex version` in place of `claude version`, and are re-run on a `codex --version` change.
