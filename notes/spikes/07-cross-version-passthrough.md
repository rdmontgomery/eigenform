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
   does, via a structural field-targeted rewrite (see finding 4), with no per-type
   knowledge.

4. **Session-id position — CORRECTED 2026-06-03.** The first pass walked JSON values
   for *exact equality* to the session uuid and found 0 off-`sessionId`, concluding a
   string-token swap was safe. **That was a measurement artifact.** Re-checking for
   *substring* occurrences (not just exact value equality) found the session uuid buried
   inside `tool_result` / `stdout` content in **5 of 11 sessions** (15, 12, 2, 1, 1
   occurrences) — a tool had printed the session's own `<id>.jsonl` filename or file
   contents. (This project inspects `~/.claude/projects`, so its sessions quote their own
   ids routinely.) A naive string-replace would **corrupt** those rows; the original
   guard (which bails on any off-field occurrence) would **refuse to fork ~45% of real
   sessions**. The corpus property test surfaced this immediately.

   What *is* true: across all 11 sessions, **the session uuid is never the exact full
   value of a non-`sessionId` key** (`exact-other = 0`). So the correct rewrite is
   **field-targeted**: structurally walk the JSON and replace only values sitting at a
   `sessionId` key. Substring occurrences in content are left untouched (they are at
   `content`/`stdout` keys). A retained guard bails only on the `exact-other` case (a
   non-`sessionId` key whose full value is the session uuid) as an early warning for
   future schema drift. See `crates/surgery/src/lib.rs::rewrite_session_id`.

5. **Leaf re-point target:** for every session the LAST `last-prompt` row has a
   `leafUuid` present, and it resolves to a real turn `uuid` in the file — across all 6
   versions, back to 2.1.126. The re-point step is not a 2.1.161 artifact.

## Implication

CONFIRMED (with the finding-4 correction). Passthrough + field-targeted id rewrite +
last-`last-prompt` re-point are version-robust on the available corpus. The surgery crate
encodes these as: total `.type` dispatch, opaque raw-line retention, **field-targeted**
session-id rewrite (structural walk of `sessionId` keys, guard bails only on the
`exact-other` case), and re-point off the final `last-prompt`. A gated corpus
property-test (see surgery design) re-runs findings 1/4/5 plus byte-identical round-trip
on whatever corpus a dev's machine has, so version drift surfaces automatically without
committing other projects' content. The corpus test is what caught the finding-4 artifact
in the first place.

## Falsifiers to watch

- A future row that carries the session uuid as the full value of a key other than
  `sessionId` (e.g. a hypothetical `resumedFrom`). The field-targeted rewrite would leave
  it stale, but the `exact-other` guard bails and flags it for us to extend the rewrite
  rule. Re-survey on version bumps.
- A `last-prompt` schema change dropping `leafUuid`. Corpus test asserts it resolves.
- The session uuid appearing as a substring of content is EXPECTED and safe under the
  field-targeted rewrite (left untouched); only naive string-replace was vulnerable.
