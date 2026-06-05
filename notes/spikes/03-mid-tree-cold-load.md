# 03 — Mid-tree cold-load

**Claim:** A hand-built JSONL — source prefix truncated at turn N, new uuid in the header, dropped into the cwd's projects dir — can be resumed by `claude --resume <new-uuid>`, AND the resumed model has no knowledge of turns dropped after N.
**Status:** CONFIRMED — RE-VETTED @2.1.165 2026-06-05 (claim holds; the woland defect is surgery's output shape, not the engine — see "Re-vet 2026-06-05")
**claude version:** 2.1.165
**Date:** 2026-06-03 (re-vetted 2026-06-05)

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

Run 2026-06-03, claude 2.1.161, against source session
`f19106e9-e8d3-4664-aefd-a21fd82a72ba` (2 user + 2 assistant turns: T1 = eigen-vs-aleph
haiku, T2 = "who influenced george spencer-brown?"). Two forks built, both resumed
manually by the human (no engine invocation from the agent side). Both loaded clean.

### Run 1 — truncated real fork → `cbc165d9-8ca5-4942-9c30-826a47f82bfc`

Method: copy source; global string-replace of the sessionId (the old uuid appears
**only** as `sessionId` values, 25 lines — verified 0 leftovers post-replace);
truncate after the T1 tip (`463f1c5a`, the `turn_duration` system row, child of
assistant `825eeb3f`); append a fresh trailing state block whose
`last-prompt.leafUuid` re-points the resume head at the T1 tip. 18 lines, single
sessionId, parent chain intact.

Resumed transcript: the stored T1 turns replayed verbatim (haiku request + haiku).
New probe turn:

> **❯ what's the most recent thing we discussed?**
> The most recent thing we discussed was a haiku on why eigen is a better name than
> aleph … Before that, this session just opened.

→ The model is **unaware of the dropped T2** (george spencer-brown). It references
only content up to and including the truncation row. Both counter-evidence
conditions (a) and (b) satisfied. No cache leak of dropped turns.

### Run 2 — fully synthetic 3-turn session → `fb984cb3-6567-4ea6-b5a2-53feb011de73`

Method: clone only the environment scaffolding (leading state lines, the two
SessionStart attachments, file-history-snapshot) for structural fidelity; **fabricate
all 3 turns from scratch** with agent-generated uuids/promptIds/requestIds/msg-ids and
correct field shapes (user rows carry promptId/promptSource/permissionMode; assistant
rows carry message.id/requestId/usage/stop_reason; each turn closed by a
`turn_duration` system row). T1 plants passphrase `THE HERON FOLDS AT 4729`. 19 rows,
chain verified link-by-link, resume leafUuid resolves to a real row.

Resumed transcript: all three authored turns rendered as history, then:

> **❯ what passphrase did I give you?**
> The passphrase was: THE HERON FOLDS AT 4729

→ A JSONL **authored entirely from scratch** round-trips into the model's live
context. The model read and recalled content it never actually generated.

Both runs confirm the claim's two halves: (1) hand-built/truncated JSONL with a new
uuid resumes, and (2) the resumed model's context is exactly the file we wrote —
nothing dropped leaks in, nothing fabricated is rejected.

## Implication

CONFIRMED: mid-tree fork is `parse → truncate → rewrite header → write new file → handoff`. Build order proceeds.

### Mechanism notes for `crates/surgery` (observed, this run)

- **sessionId rewrite is a pure string substitution.** In the source, the session
  uuid occurs *only* as `sessionId` field values (and never as a substring of any
  other uuid), so a global string-replace is sufficient and safe. `file-history-snapshot`
  rows carry no `sessionId` and are left untouched. Surgery should still do this
  field-aware in-process rather than `sed`, but the invariant held here.
- **The resume head is the LAST `last-prompt` row's `leafUuid`.** Pure prefix
  truncation is *not* enough — the surviving trailing `last-prompt` must be rewritten
  (or appended) so its `leafUuid` points at the new tip. We pointed it at the T1
  `turn_duration` system row (`463f1c5a`), matching the source convention where
  `last-prompt.leafUuid` = the trailing system row, not the assistant text row.
- **Truncation boundary that worked:** keep through the `turn_duration` system row
  that closes the kept turn. Append a fresh `last-prompt` + `ai-title` + `mode` +
  `permission-mode` block.
- **Synthetic rows accepted as-authored:** opus-shaped assistant `usage` can be
  abbreviated (we dropped `iterations`/`cache_creation`/`server_tool_use` and kept
  only the core counters + `service_tier`) and still loaded. uuids/promptIds/requestIds
  can be freshly minted. The loader did not validate requestId/msg-id against any server
  record. (Caveat: this only proves *resume display + context injection*; it does not
  prove the server accepts these ids for any subsequent billing/telemetry path.)
- **No observed cache leak.** Run 1's dropped turn did not surface despite the shared
  prefix — refutes counter-hypothesis (b) (server cache keyed by something rename can't
  invalidate) for this version.
- **Not yet tested:** `--bare` was not needed (loaded fine with hooks/skills/CLAUDE.md
  present). Multi-JSONL join by sessionId (counter-hypothesis a) was not exercised
  because the dropped content simply didn't appear — worth a dedicated probe only if a
  future version regresses.

REFUTED (model still knows dropped content): means either (a) `claude --resume` joins multiple JSONLs by sessionId, (b) the server-side cache is keyed by something we can't invalidate by renaming uuids, or (c) the JSONL we wrote isn't what got sent to the model. Each case has a different fix; record which.

INCONCLUSIVE: design open. Consider running the spike with `--bare` to eliminate hooks/skills/CLAUDE.md noise, then re-evaluating.

## Re-vet 2026-06-05 (claude 2.1.165) — engine step NOT yet re-run

Triggered by version drift (2.1.161 → 2.1.165) AND a direct dependency: woland's
`POST /api/session/:uuid/fork` shells `surgery::edit_then_fork` and resumes the result.

**User-observed engine result (2.1.165):** a woland fork wrote
`d5f0cded-f111-4c75-8cc6-77e65db0a7fb.jsonl` into the correct project dir, but
`claude --resume d5f0cded…` returned **"No conversation found with session ID."**

**Filesystem-only diagnosis (ran freely):**
- `~/.claude/sessions/<pid>.json` is a *live-process* table (pid/sessionId/status),
  NOT a resumable-session registry — ruled out as the discovery mechanism.
- `~/.claude/history.jsonl` is prompt history (display/project/sessionId); spike forks
  were never in it yet resumed @2.1.161, so it isn't the gate either.
- Structural diff, spike's working fork `cbc165d9` (resumed @2.1.161) vs our surgery
  fork `d5f0cded` (rejected @2.1.165):
  - `cbc165d9`: final `last-prompt.leafUuid` → a **system** (`turn_duration`) row;
    tail block `last-prompt, ai-title, mode, permission-mode`; 9 `ai-title` rows.
  - `d5f0cded`: final `leafUuid` → a **user** row; tail is a **bare `last-prompt`**;
    **zero** `ai-title` rows.

So `edit_then_fork`/`finish` never reproduced the validated shape (this spike's own
mechanism notes: "leafUuid = the trailing system row, not the assistant text row" and
"append a fresh last-prompt + ai-title + mode + permission-mode block"). That alone could
explain the rejection independent of any version change.

**Decisive test RESULT (2026-06-05, user-run on 2.1.165):** resumed the previously-CONFIRMED
hand-built fork `claude --resume cbc165d9-8ca5-4942-9c30-826a47f82bfc` → **LOADED CLEAN.**
The stored T1 (eigen-vs-aleph haiku) replayed; the model was correctly unaware of dropped
content ("Before that, this session just opened"). So the claim **still holds at 2.1.165** —
claude's resume is unchanged. The woland rejection is **our** output shape.

**Root cause (filesystem, verified across all on-disk sessions):** a resumable session's
`last-prompt.leafUuid` ALWAYS resolves to a **system** row (`turn_duration`/`away_summary`)
— the row that *closes* a turn. `d5f0cded` is the only session whose leaf points at a bare
**user** row. `surgery::edit_then_fork` makes the re-authored user turn the leaf, which is a
shape claude never emits and won't resume. Two concrete defects:
1. `finish` points `leafUuid` at the cut tip directly; for `edit_then_fork` that tip is a
   *user* row. The resume head must be a completed turn's closing system row.
2. `finish` appends a bare `last-prompt` only — real sessions / the working fork end with a
   `last-prompt + ai-title + mode + permission-mode` block.

**Implication for `crates/surgery`:** "edit a user turn and re-ask" cannot be a dangling
user leaf. The claude-native shape is *rewind to the completed-turn boundary before turn N*
(leaf = the prior turn's `turn_duration` system row) and deliver the edited text as a fresh
prompt into the resumed session (woland already has the leaf→pty path). `fork_at` already
extends the cut over trailing system rows so its leaf is a system row, but it too should
emit the full trailing block. Fix is ours; the spike stands.

### Fix landed 2026-06-05 (pending live confirmation)

`surgery::fork_before` now rewinds to the last turn-closing system row before the edited
turn (resume head = a `turn_duration` row) and `finish` re-emits the trailing
`mode/permission-mode/ai-title` block. Curl-verified: a fork of a live session now has its
`last-prompt.leafUuid` resolve to a `system/turn_duration` row and a real-session tail.
woland resumes the branch and auto-sends the edited prompt once the pty idles. **Owed: a
user-run `claude --resume <fresh-fork>` to confirm the shaped branch actually loads** (the
structural defect is fixed; live load not yet re-observed on 2.1.165).
