---
name: vetting-claude-internals
description: Use when `claude --version` differs from the version recorded on any CONFIRMED spike in notes/spikes/, or before merging design work that depends on a spike's claim. Re-runs the documented procedures, diffs against prior results, and updates the spike files honestly.
---

# Vetting Claude Code internals

eigenform's design rests on a small set of empirically verified claims about
Claude Code's internals — the JSONL format, `--fork-session` semantics,
the billing flip, cold-load behavior, the bundled-skill gap, the plugin
layouts on disk. Each is recorded in `notes/spikes/<NN>-<topic>.md` with
a `claude version:` stamp and a `Status:` (CONFIRMED / PENDING /
REFUTED / INCONCLUSIVE).

When the running `claude --version` is newer than the stamp on any
CONFIRMED spike, re-vet that spike before relying on the claim. When
about to merge work that depends on a CONFIRMED spike, re-vet it even
if the version hasn't moved.

## When to invoke

- `claude --version` no longer matches the version on a CONFIRMED spike.
- About to merge or ship work that depends on a CONFIRMED spike (e.g.
  surgery changes that depend on spike 03, daemon work that depends on
  spike 01).
- Periodic audit on a long-lived branch — spikes can decay even at the
  same version because cache behavior, plugin layout, or filesystem
  conventions shifted on the server side.

## Procedure

1. Run `claude --version` and record the result.
2. Read every file under `notes/spikes/`. Build a worklist of spikes
   whose `Status:` is CONFIRMED and whose `claude version:` stamp is
   older than the running version.
3. Also worklist any spike on which the current task directly depends,
   regardless of version drift.
4. For each spike in the worklist, re-execute its `Procedure` section
   verbatim:
   - Engine-touching steps (`claude --fork-session`, `claude --resume`,
     `claude -p`, SDK calls) require **explicit user authorization for
     this specific run**. Do not run them silently. The feedback rule
     "No unauthorized engine invocations" is load-bearing here.
   - Filesystem-only steps (`ls`, `find`, `cat`, parsing JSONL with
     `jq` / `python`) run freely.
5. Record the outcome in the spike file:
   - **No change observed:** bump the `claude version:` stamp to the
     current version, add a single line under `## Result`:
     `RE-VETTED <ISO-date> on <new-version>: unchanged.`
   - **Behavior changed but design still holds:** keep Status CONFIRMED,
     record the new observation under `## Result` (do not overwrite the
     prior one), append an `## Implication` paragraph naming the shift.
   - **Behavior changed and the design assumption is now wrong:** flip
     Status to REFUTED, document the new behavior, and add an
     `## Implication` paragraph naming which parts of
     `docs/plans/2026-06-02-eigen-foundation-design.md` need to change.
6. Update the design doc's "Empirical anchors (verified)" section if any
   anchor moved.
7. If any spike flipped to REFUTED:
   - Open a new spike note proposing a replacement claim and procedure.
   - Surface the implication explicitly in the end-of-turn report — name
     the design rules that need re-examination, do not bury this.
   - Stop downstream work that depended on the refuted claim until the
     replacement is itself CONFIRMED.

## Hard rules

- **Never run `claude -p` or `claude --print` or SDK calls for spike
  work without explicit per-invocation user authorization.** The point
  of spikes is to verify behavior the user cares about; quietly burning
  credits to verify them is worse than asking.
- **Never silently downgrade a Status.** Always record what changed and
  why. A spike that quietly flips from CONFIRMED to "well, sort of" is
  worse than one whose change is loud.
- **A spike that re-passes by accident is worse than one not re-run.**
  If the user did not actually re-execute the engine-touching steps,
  say so: "filesystem-only re-verification; engine steps not re-run."
  False confidence is the failure mode.
- **Read all of `notes/spikes/` before declaring vetting complete.** A
  spike you forgot to re-read is a spike you implicitly assumed away.

## Related

- `notes/spikes/README.md` — spike format and conventions.
- `docs/plans/2026-06-02-eigen-foundation-design.md` — the design these
  spikes ground.
- Feedback memory: "No unauthorized engine invocations" — load-bearing
  for step 4.
