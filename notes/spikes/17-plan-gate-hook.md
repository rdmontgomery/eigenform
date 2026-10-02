# 17 — plan gate hook

**Claim:** An HTTP `PermissionRequest` hook with matcher `ExitPlanMode` gates Claude Code's plan approval:
1. it fires when Claude calls ExitPlanMode in plan mode, with `tool_input` carrying the plan as `plan` (or a plan-file path field `plan_gate::plan_text` can follow);
2. `decision.behavior: "allow"` approves the plan and Claude leaves plan mode and proceeds;
3. `decision.behavior: "deny"` with `decision.message` keeps Claude in plan mode and hands it the message as plan feedback, and Claude revises and calls ExitPlanMode again (which fires the hook again);
4. an empty 2xx body (no decision), a connection error (daemon down), or a hook timeout all fall through to Claude Code's own approval prompt;
5. `"timeout": 1800` is accepted, so a review can take up to half an hour.
**Status:** PENDING. Hook fields come from the hooks reference (code.claude.com/docs/en/hooks, 2026-10). The reference doesn't say what ExitPlanMode's `tool_input` holds, or that a deny `message` reaches Claude, rather than only the user, for this tool.
**claude version:** _fill in_
**Date:** 2026-10-02

## Procedure

```sh
eigenform                                   # daemon on :4317
eigenform plan-gate                         # prints the hook; merge into ~/.claude/settings.json
# In a scratch repo, from an eigenform tab:
claude --permission-mode plan
> plan a tiny change: add a CONTRIBUTING.md with one paragraph. Don't write anything yet.

# (1) The pane opens to "Plan review". Record the request Claude Code sent:
curl -s localhost:4317/api/events | jq '.[] | select(.kind|startswith("plan-review"))'
curl -s localhost:4317/api/plan-reviews | jq          # .source: "tool_input.plan" or a file path
# (3) Mark one line, then click Send back. Watch the TUI: does Claude acknowledge the feedback, stay
#     in plan mode, revise, and present again (the pane reopens with the new plan)?
# (2) Approve the revision. Does Claude leave plan mode and start writing the file?
# (4a) Next plan: click "Decide in terminal". Expect the normal TUI approval prompt.
# (4b) `eigenform stop`, plan again. Expect the normal TUI prompt, no error.
# Paste the transcript rows around ExitPlanMode:
jq -c 'select(.message.content[]?.name? == "ExitPlanMode" or .toolUseResult?) | {type, c: .message.content}' \
  ~/.claude/projects/<proj>/<uuid>.jsonl | tail -6
```

## Result

_Paste real output. Note exactly what text Claude sees after a Send back (the tool_result content)._

## Implication

- **CONFIRMED:** the gate is plannotator-equivalent with no Claude-side changes.
- **(1) REFUTED (no plan in tool_input):** read the newest `~/.claude/plans/*.md` the session wrote instead. The transcript-derived artifacts list already has it.
- **(3) REFUTED (message not shown to Claude):** switch the hook to `PreToolUse` with `permissionDecision: "deny"` and `permissionDecisionReason`, and re-run this spike for that event.
- **(4) REFUTED (no fallback on error):** the gate must never be installed by default. Keep it opt-in behind `eigenform plan-gate`, and say so loudly.
