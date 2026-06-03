# spikes

Empirical verification of load-bearing claims. One file per claim.

Format (every spike):

```
# <NN> — <topic>

**Claim:** one sentence.
**Status:** CONFIRMED | REFUTED | PENDING | INCONCLUSIVE
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
