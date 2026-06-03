# 07 — Cross-version passthrough & guarded-swap corpus survey

**Claim:** The surgery passthrough model (type only user/assistant/system turns +
last-prompt; everything else opaque) and the guarded session-id swap hold across
every Claude Code session version present on this machine — not just the spike's
2.1.161.
**Status:** CONFIRMED
**claude versions surveyed:** 2.1.126, 2.1.128, 2.1.131, 2.1.133, 2.1.138, 2.1.161
**Date:** 2026-06-03

## Corpus

`~/.claude/projects/*/*.jsonl` — 11 sessions, ~9.7 MB, spanning 2026-05-05 → 2026-06-03,
across 6 distinct `version` values. Includes 3 eigen-project sessions (one real, two
spike-authored) and 8 from other local projects.

## Findings

1. **Every row carries a top-level `type`.** 0 typeless rows over the whole corpus.
   Dispatching surgery on `.type` is safe.

2. **Row types observed** (count): `assistant` 1559, `user` 960, `last-prompt` 231,
   `system` 224, `permission-mode` 213, `ai-title` 212, `attachment` 203,
   `file-history-snapshot` 177, `pr-link` 65, `queue-operation` 43, `mode` 21.

   **`pr-link` and `queue-operation` never appeared in spike 03.** They are exactly
   the unmodeled rows passthrough exists for — a fully-typed model would have failed to
   parse them; passthrough emits them verbatim. This is the design-validating result.

3. **`sessionId` coverage:** every row type EXCEPT `file-history-snapshot` carries a
   top-level `sessionId`. The id rewrite must therefore touch opaque rows too — and
   does, via the guarded swap, with no per-type knowledge.

4. **Guarded-swap safety:** for each session, walked every JSON value equal to that
   session's own uuid (~3,700 occurrences total) and checked its position. **0 occurred
   at a non-`sessionId` position.** The guarded string-token swap is empirically safe on
   the entire corpus; the guard (bail if the uuid appears off a session field) is the
   safety net for any future file that violates this.

5. **Leaf re-point target:** for every session the LAST `last-prompt` row has a
   `leafUuid` present, and it resolves to a real turn `uuid` in the file — across all 6
   versions, back to 2.1.126. The re-point step is not a 2.1.161 artifact.

## Implication

CONFIRMED. Passthrough + guarded swap + last-`last-prompt` re-point are version-robust
on the available corpus. The surgery crate encodes these as: total `.type` dispatch,
opaque raw-line retention, guarded swap with a bail on stray, and re-point off the final
`last-prompt`. A gated corpus property-test (see surgery design) re-runs findings 1/4/5
plus byte-identical round-trip on whatever corpus a dev's machine has, so version drift
surfaces automatically without committing other projects' content.

## Falsifiers to watch

- A future row type that hides a session-scoped id under a key other than `sessionId`
  (guarded swap would leave it stale — but the guard would NOT catch it, since the value
  wouldn't equal the *session* uuid; it'd be some other id). Re-survey on version bumps.
- A `last-prompt` schema change dropping `leafUuid`. Corpus test asserts it resolves.
