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

Spikes 2–4 were the original gate on implementation start; 3 is confirmed, and 2 (tail fork) and 4 (billing flip) are still pending, with nothing built on them yet. The cache-TTL spike the foundation design deferred to step 9 was never numbered; spike 05 is bundled skills.

RETIRED means the claim may still be true but nothing depends on it anymore; it is skipped by re-vetting (`.claude/skills/vetting-claude-internals`).

Spike 17 covers the plan gate's `PermissionRequest`/`ExitPlanMode` hook contract, and spike 18 the turn hooks that decide whether closing a tab may end its claude (`crates/daemon/src/turns.rs`). Spikes 14–16 cover the OpenAI Codex CLI (`crates/codex`, `.claude/skills/codex-worker`). They record `codex version` in place of `claude version`, and are re-run on a `codex --version` change.
