# 11 — snake_case `session_id` (claude 2.1.199+)

**Claim:** claude 2.1.199 adds a snake_case `session_id` field on assistant rows,
carried alongside the existing camelCase `sessionId` with the same value. Surgery
must rewrite **both** keys, or forking a 2.1.199 session fails: the second key
whose value equals the session id trips `rewrite_session_fields`' stray-occurrence
guard and the swap refuses.
**Status:** CONFIRMED
**claude version:** 2.1.199 (Claude Code)
**Date:** 2026-07-02

## Why this matters

`surgery::fork_before` mints a fresh session id and calls `rewrite_session_id` on
every row to swap old → new. That rewrite treats only `sessionId` as an
id-bearing key and flags any *other* key whose full value equals the old id as a
stray, then REFUSES (the deliberate spike-07 safety guard against blind-swapping
an id that appears somewhere unexpected). On 2.1.199 the new `session_id` field
holds exactly that id, so `fork_before → finish → rewrite_session_id` errors with
`StrayOccurrence` on every 2.1.199 row — breaking the per-turn fork and the
downgrade-recovery feature that builds on it. The corpus property test
(`corpus_round_trips_and_guards_cleanly_across_versions`) failed on precisely this
row.

## Procedure / Evidence

Scan the local `~/.claude/projects` corpus for the snake_case field and the
version of each file that carries it:

```
$ grep -rl '"session_id"' ~/.claude/projects | while read f; do
    grep -o '"version":"[^"]*"' "$f" | head -1; done | sort -u
```

## Result

```
snake_case "session_id" appears only in: "version":"2.1.199"
```

Every version from 2.1.138 → 2.1.198 present in the corpus (2.1.138, 2.1.161–181,
2.1.191–198) has `sessionId` only; the snake_case twin first appears in 2.1.199.

## Implication

The swappable-key set in `rewrite_session_fields` now includes both spellings:

```rust
if key == Some("sessionId") || key == Some("session_id") {
```

The stray guard is otherwise unchanged — any *other* key whose value equals the
session id is still flagged and the swap still refuses, so genuinely stray
occurrences (a tool that printed the id under some unrelated key) remain refused.
Backwards compat holds: pre-2.1.199 rows have no `session_id` field, so the new
arm never fires for them. The corpus test is green across 2.1.138 → 2.1.199.
