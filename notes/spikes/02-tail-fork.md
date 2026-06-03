# 02 — Tail fork via `--fork-session`

**Claim:** `claude --resume <uuid> --fork-session` creates a new session with a new uuid whose JSONL begins as a copy/derivation of the source session's history, and which the engine resumes cleanly without the user having to write any JSONL bytes.
**Status:** PENDING
**claude version:** 2.1.161
**Date:** —

## Procedure

In a fresh terminal (not nested under another `claude`):

1. Pick a source session uuid. Use the latest in any project, e.g.:

   ```
   ls -t ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl | head -1
   ```

   Note: `<SRC_UUID>` is the filename without `.jsonl`.

2. Snapshot the projects directory before forking:

   ```
   ls ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl | sort > /tmp/eigen-spike-02-before.txt
   ```

3. Fork:

   ```
   cd ~/projects/eigen
   claude --resume <SRC_UUID> --fork-session
   ```

   Inside the resumed session, send one user turn (e.g. `who am I talking to right now?`) and `/exit`.

4. Snapshot after:

   ```
   ls ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl | sort > /tmp/eigen-spike-02-after.txt
   diff /tmp/eigen-spike-02-before.txt /tmp/eigen-spike-02-after.txt
   ```

   New JSONL filename(s) = `<NEW_UUID>.jsonl`.

5. Compare contents:

   ```
   wc -l ~/.claude/projects/-home-rdmontgomery-projects-eigen/<SRC_UUID>.jsonl \
         ~/.claude/projects/-home-rdmontgomery-projects-eigen/<NEW_UUID>.jsonl

   head -3 ~/.claude/projects/-home-rdmontgomery-projects-eigen/<NEW_UUID>.jsonl
   ```

   Inspect: does the new file copy the source's turns? Does the new header's `sessionId` match `<NEW_UUID>`? Does `leafUuid` point at the original session's tip or at the new turn?

## Result

(paste real output here)

## Implication

If CONFIRMED: tail fork in `eigen surgery fork --tail <uuid>` is a shell-out, zero JSONL writes on our side. Mid-tree fork (spike 03) is the only operation that touches bytes.

If REFUTED (e.g. `--fork-session` writes a different file shape than expected, or doesn't create a separate file at all): the thin-layer plan changes. We may need to parse and replicate whatever `--fork-session` does ourselves to keep our forked sessions resumable.
