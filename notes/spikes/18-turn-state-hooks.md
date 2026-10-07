# 18 — turn-state hooks (is a claude pty safe to kill?)

**Claim:** HTTP hooks passed with `claude --settings <file>` let the daemon tell whether killing a claude pty loses work:
1. `--settings` hooks merge with project/user hooks rather than replacing them;
2. `UserPromptSubmit` opens a turn and `Stop` closes it, including turns Claude starts on its own;
3. `PermissionRequest` (and, about 6s later, `Notification` `permission_prompt`) fires while the turn is blocked on approval;
4. an Esc interrupt closes the turn with a hook event;
5. `Stop` waits for background work (background Bash, background subagents);
6. background Bash is visible in the process tree, and background subagents are visible through `SubagentStart`/`SubagentStop`;
7. `Notification` `idle_prompt` means nothing is running.
**Status:** CONFIRMED for 1, 2, 3 and 6. REFUTED for 4, 5 and 7.
**claude version:** 2.1.289
**Date:** 2026-10-05

## Procedure

Linux cloud container, interactive claude in a tmux pty (160×45), with an isolated `CLAUDE_CONFIG_DIR` so the user's real config isn't touched. The listener (`18-turn-state-hooks-listener.py`) logs one JSON line per hook POST.

```sh
python3 18-turn-state-hooks-listener.py 47321 hooks.log &
# hooks.json: one http hook per event → http://127.0.0.1:47321/<event>, timeout 5
#   UserPromptSubmit Stop StopFailure PermissionRequest Notification
#   SubagentStart SubagentStop PreToolUse PostToolUse SessionEnd
# proj/.claude/settings.json: a project-level command hook on Stop that appends PROJECT_STOP_HOOK to a file
cd proj && tmux new-session -d -s sp -x 160 -y 45 "CLAUDE_CONFIG_DIR=$PWD/../cfg claude --settings ../hooks.json"
```

Scenarios run in order in one session (permission mode: manual):
A. `reply with just the word ok`
B. a foreground `python3 -c 'import time; time.sleep(45)'`: approve the prompt, then Esc while it runs
C. the same command with `run_in_background: true`, ending the turn immediately; then wait for it to finish
D. one `general-purpose` subagent with `run_in_background: true` (a 600-word essay, no tools), ending the turn immediately

The process tree was read with `pstree -p <claude pid>` (Claude's own threads filtered out).

## Result

A. Plain turn. The project-level hook also fired, so `--settings` merges.
```
{"t": 1791212270.9,  "hook_event_name": "UserPromptSubmit"}
{"t": 1791212273.55, "hook_event_name": "Stop", "stop_hook_active": false}
$ cat project-hook.log
PROJECT_STOP_HOOK
```

B. Permission prompt, then an Esc interrupt.
```
{"t": 1791212355.78, "hook_event_name": "UserPromptSubmit"}
{"t": 1791212357.86, "hook_event_name": "PreToolUse", "tool_name": "Bash", ...}
{"t": 1791212357.92, "hook_event_name": "PermissionRequest", "tool_name": "Bash", ...}
{"t": 1791212363.92, "hook_event_name": "Notification", "notification_type": "permission_prompt", "message": "Claude needs your permission"}
--- tree during the foreground tool:
claude(679)-+-bash(1119)---python3(1120)
  1119 /bin/bash -c source .../cfg/shell-snapshots/snapshot-bash-....sh ... && eval 'python3 -c ...'
--- after Esc: no further hook events (no Stop, no PostToolUse), tree empty.
    No idle_prompt either in the following 75s.
--- transcript tail:
assistant  tool_use  Bash
user       tool_result "The user doesn't want to proceed with this tool use. The tool use was rejected (..."
user       text "[Request interrupted by user for tool use]"
```

C. Background Bash.
```
{"t": 1791212493.54, "hook_event_name": "UserPromptSubmit"}
{"t": 1791212495.18, "hook_event_name": "PreToolUse", "tool_name": "Bash", ...}
{"t": 1791212495.24, "hook_event_name": "PermissionRequest", ...}
{"t": 1791212501.23, "hook_event_name": "Notification", "notification_type": "permission_prompt", ...}
{"t": 1791212509.08, "hook_event_name": "PostToolUse", "tool_name": "Bash", ...}
{"t": 1791212510.24, "hook_event_name": "Stop"}                       <- the shell is still running
--- tree after Stop:
claude(679)-+-bash(1218)---python3(1219)
TUI footer: "done 3:01 PM · 1 shell still running"
{"t": 1791212570.27, "hook_event_name": "Notification", "notification_type": "idle_prompt", "message": "Claude is waiting for your input"}   <- shell still running
{"t": 1791212609.15, "hook_event_name": "UserPromptSubmit"}           <- shell finished; Claude starts its own turn
{"t": 1791212610.99, "hook_event_name": "Stop"}
```

D. Background subagent. It runs in-process, so nothing shows in the process tree.
```
{"t": 1791212642.19, "hook_event_name": "UserPromptSubmit"}
{"t": 1791212644.29, "hook_event_name": "PreToolUse", "tool_name": "Agent", ...}
{"t": 1791212644.37, "hook_event_name": "SubagentStart", "agent_id": "a202dd4839f66b362", "agent_type": "general-purpose"}
{"t": 1791212644.37, "hook_event_name": "PostToolUse", "tool_name": "Agent", ...}
{"t": 1791212646.03, "hook_event_name": "Stop"}                       <- the subagent is still running
--- pstree: no children
{"t": 1791212658.33, "hook_event_name": "SubagentStop", "agent_id": "a202dd4839f66b362", "agent_type": "general-purpose"}
{"t": 1791212658.46, "hook_event_name": "UserPromptSubmit"}           <- Claude's own follow-up turn
{"t": 1791212665.82, "hook_event_name": "Stop"}
```

Caveat: this container injects host-managed env and settings. A `sleep 40` was blocked by a host rule, and `session_id` came through as the parent cloud session's id. Re-run on a laptop before relying on `session_id` → pty mapping.

## Implication

A pty is **safe to kill** only when all of these hold:
- **Turn closed:** the last turn event is `Stop`/`StopFailure`, *or* the transcript's last entry is `[Request interrupted by user…]`. An Esc interrupt fires no hook. The transcript closes it, and the forest already tails the transcript (spike 01).
- **No subagents:** the count of `SubagentStart` minus `SubagentStop`, keyed by `agent_id`, is 0.
- **No Bash children:** claude has no child whose command line carries `shell-snapshots/snapshot-` (how the Bash tool launches every shell, foreground or background). That marker keeps MCP stdio servers, which are also children, from counting. Not yet tested with an MCP server attached.
- **Known state:** the daemon has seen at least one hook event from this pty since it spawned it. Otherwise the state is unknown and close falls back to detach.

`idle_prompt` and `Stop` on their own are not enough (C, D). The screen classifier (`host.rs` `classify`) is unrelated to this and stays as the status-dot signal only.

## Follow-up (2026-10-07, implemented in `crates/daemon/src/turns.rs`)

A resumed session that's opened and closed without a prompt fires no turn hook, so it would never count as seen. `SessionStart` accepts only command hooks, so the daemon forwards it with `curl … --data-binary @- <url>` and `-o /dev/null`, because SessionStart stdout becomes Claude's context. Verified end to end through the daemon (claude 2.1.289, real ptys driven over `/pty`):

```
[fresh, no prompt yet]                 if_safe → 204
[foreground tool running]              if_safe → 409 a turn is in progress
[after Esc]                            if_safe → 204
[turn ended, background shell running] if_safe → 409 1 shell(s) still running
[background shell finished]            if_safe → 204
```
