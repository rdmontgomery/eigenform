# 03 — Mid-tree cold-load

**Claim:** A hand-built JSONL — source prefix truncated at turn N, new uuid in the header, dropped into the cwd's projects dir — can be resumed by `claude --resume <new-uuid>`, AND the resumed model has no knowledge of turns dropped after N.
**Status:** PENDING
**claude version:** 2.1.161
**Date:** —

**This is the load-bearing spike. If REFUTED, the entire mid-tree fork approach changes.**

## Procedure

1. **Pick a source session with a memorable mid-conversation fact.** E.g. one where you told the assistant "remember the number 7392" at turn N, and at turn N+M asked something else.

   ```
   SRC=~/.claude/projects/-home-rdmontgomery-projects-eigen/<SRC_UUID>.jsonl
   wc -l "$SRC"
   ```

2. **Identify the truncation turn uuid.** Read the JSONL and find the row whose payload is the last one you want preserved. Note its `uuid`.

3. **Build the forked JSONL by hand:**

   ```bash
   NEW_UUID=$(uuidgen | tr 'A-Z' 'a-z')
   DST=~/.claude/projects/-home-rdmontgomery-projects-eigen/${NEW_UUID}.jsonl

   # Rewrite header line(s) to use NEW_UUID, then copy turns up to and including the truncation row.
   # Inspect SRC headers first — see notes/spikes/01 for the observed shapes.

   jq -c --arg sid "$NEW_UUID" --arg leaf "<TRUNC_TURN_UUID>" '
     if .type == "last-prompt" then .sessionId = $sid | .leafUuid = $leaf
     elif .sessionId then .sessionId = $sid
     else . end
   ' "$SRC" > /tmp/spike-03-rewritten.jsonl

   # Then truncate at the chosen row.
   #   awk-based truncation that stops after emitting the row whose .uuid matches TRUNC_TURN_UUID:
   python3 -c "
   import json, sys, os
   trunc = '<TRUNC_TURN_UUID>'
   with open('/tmp/spike-03-rewritten.jsonl') as f, open(os.path.expanduser('$DST'), 'w') as out:
       stop = False
       for line in f:
           if stop: break
           out.write(line)
           try:
               row = json.loads(line)
           except Exception:
               continue
           if row.get('uuid') == trunc:
               stop = True
   "

   wc -l "$DST"
   ```

   (The python+jq combo here is procedural cruft for the spike. Production surgery does this in-process.)

4. **Resume:**

   ```
   cd ~/projects/eigen
   claude --resume ${NEW_UUID}
   ```

   Ask: *"What was the last thing I told you to remember?"* and *"What's the most recent thing we discussed?"*

5. **Compare against the original session** (resume separately, ask same questions).

## Result

(paste real output here)

Note: log both responses verbatim. Cache TTL could mean the model "remembers" because the prefix is still cached server-side and the new uuid is being treated as cache-hit territory. Counter-evidence to look for: (a) model is unaware of turns we dropped, (b) model only references content up to and including the truncation row.

## Implication

CONFIRMED: mid-tree fork is `parse → truncate → rewrite header → write new file → handoff`. Build order proceeds.

REFUTED (model still knows dropped content): means either (a) `claude --resume` joins multiple JSONLs by sessionId, (b) the server-side cache is keyed by something we can't invalidate by renaming uuids, or (c) the JSONL we wrote isn't what got sent to the model. Each case has a different fix; record which.

INCONCLUSIVE: design open. Consider running the spike with `--bare` to eliminate hooks/skills/CLAUDE.md noise, then re-evaluating.
