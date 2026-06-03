# 06 — Project-local skill pickup

**Claim:** A SKILL.md placed at `<cwd>/.claude/skills/<skill-name>/SKILL.md` is automatically picked up by Claude Code on the next session start from that cwd, surfaced in the harness's available-skills list, and invocable via `/<skill-name>` or the Skill tool.
**Status:** PENDING
**claude version:** 2.1.161
**Date:** —

## Why this matters

`eigen skills tree` proves the skill is visible *to eigen* (the [repo] layer renders it). But that's the audit perspective. The other half — does Claude Code itself respect repo-local skills the same way it respects global and plugin ones — is what makes the skill usable as a workflow, not just an artifact.

Without this confirmed, the repo-local layer in eigen is just a documentation convention; with it, project skills become first-class.

## Procedure

1. Confirm `/home/rdmontgomery/projects/eigen/.claude/skills/vetting-claude-internals/SKILL.md` exists.
2. From a fresh shell, `cd ~/projects/eigen`.
3. `claude` (start a new interactive session).
4. Inside that session, observe the SessionStart system-reminder block of available skills. Check:
   - Does `vetting-claude-internals` appear?
   - With what namespace? Bare `vetting-claude-internals`, or `repo:vetting-claude-internals`, or some other prefix?
5. Type `/vetting-claude-internals` and see if it's accepted by the harness, or whether the harness routes it through the Skill tool.

Repeat in a different cwd (e.g. `~/projects/potnuse`) to confirm the skill is scoped to eigen, not leaking globally.

## Result

(paste real output here)

## Implication

CONFIRMED: repo-local skills are a real workflow primitive. `eigen skills audit` should flag misnaming and shadowing in `<cwd>/.claude/skills/` with full confidence that the user-facing behavior depends on what eigen sees. We can also encourage moving stable skills from `~/.claude/skills/` into individual repos when they're project-specific.

REFUTED: project-local SKILL.md is not auto-loaded. Either it requires an opt-in flag, manifest entry, or `claude` setting, or the location is wrong. Investigate: check `--bare` behavior, check whether there's a `.claude/settings.json` or `plugin.json` that toggles skill loading, check whether the convention is actually `<repo>/.claude/plugins/local/<plugin>/skills/<name>/SKILL.md` instead.

INCONCLUSIVE: skill appears but isn't invocable, or appears in some sessions but not others. Record the conditions and re-run.
