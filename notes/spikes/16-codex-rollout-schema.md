# 16 — codex rollout schema

**Claim:** The on-disk shapes `eigenform-codex` reads hold on the installed Codex:
1. Rollouts live at `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<YYYY-MM-DDThh-mm-ss>-<thread_id>.jsonl`. A reverted thread appends `_<rollout_id>`. Cold files may become `.jsonl.zst`, which are skipped.
2. Each line is `{timestamp, [ordinal], type, payload}`, with `type` ∈ `session_meta | response_item | event_msg | turn_context | compacted | …`.
3. `session_meta.payload` carries `id`, `cwd`, `originator`, `cli_version`, `source` (`"exec"`, `"cli"`, `"vscode"`, `"mcp"`, or `{"subagent":…}`), and optionally `parent_thread_id` / `forked_from_id`.
4. User turns appear as `event_msg {type:"user_message", message}`. Turn boundaries are `event_msg task_started` / `task_complete` (aliases `turn_started` / `turn_complete`), and `turn_aborted`.
5. `response_item` payloads: `message {role, content:[{type:"output_text"|"input_text", text}]}`, `function_call {name, arguments:<json string>, call_id}`, `function_call_output {call_id, output:<string|content items>}`, `custom_tool_call {name:"apply_patch", input:<patch>, call_id}`, `custom_tool_call_output`, `local_shell_call {action:{command[]}}`, `web_search_call {action:{query}}`.
6. `turn_context.payload.model` names the model.
7. `codex exec --json` emits `{"type":"thread.started","thread_id":…}` first. `codex-worker` reads that line.
**Status:** PENDING. Read from openai/codex `main` source (`codex-rs/protocol`, `codex-rs/history`, `codex-rs/rollout`, `codex-rs/exec`), 2026-10. Not yet checked against a real rollout.
**codex version:** _fill in_
**Date:** 2026-10-02

## Procedure

```sh
codex exec "list the files here, then write hello.txt containing hi" -s workspace-write --json | head -3   # (7)
R=$(ls -t ~/.codex/sessions/*/*/*/rollout-*.jsonl | head -1); echo "$R"                                      # (1)
head -1 "$R" | jq '{type, keys: (.payload | keys)}'                                                        # (2)(3)
jq -r '[.type, (.payload.type // "")] | join(" ")' "$R" | sort | uniq -c                                  # (4)(5)
jq -c 'select(.type=="turn_context") | .payload.model' "$R" | tail -1                                    # (6)
jq -c 'select(.type=="response_item" and .payload.type=="function_call") | .payload | {name, arguments}' "$R" | head -3
# The tool names that occur (shell / exec_command / shell_command / apply_patch …), compared with
# the mapping in crates/codex/src/lib.rs::tools_of:
jq -r 'select(.type=="response_item") | .payload.name // empty' ~/.codex/sessions/*/*/*/*.jsonl | sort | uniq -c

# End to end through eigenform (daemon running):
ID=$(basename "$R" .jsonl | sed 's/^rollout-.\{19\}-//; s/_.*//')
curl -s localhost:4317/api/forest | jq ".[] | select(.uuid==\"$ID\")"
curl -s localhost:4317/api/session/$ID/json | jq '.exchanges[] | {user, assistant, kind: .tool.kind, arg: .tool.arg}'
```

## Result

_Paste real output. Note any tool name not handled by `tools_of`: it renders under its raw name, which is honest but misses the reach map._

## Implication

A field that moved shows up as an empty title, a missing model, or a raw-named tool in the drawer, not a crash. The parser is `serde_json::Value` all the way down. Fix `crates/codex/src/lib.rs` and its fixture in `crates/codex/tests/rollout.rs` together. Re-run this spike whenever `codex --version` changes, just as `vetting-claude-internals` re-runs the Claude spikes.
