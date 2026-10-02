# Codex workers — design

**Date:** 2026-10-02 · **Status:** first cut built; spikes 14–16 pending on a machine with `codex`

## The reframe

Claude can already drive OpenAI's Codex CLI headlessly (`codex exec`, corrected with `codex exec resume <id>`). What you lose compared with internal subagents is *visibility*: switching into the worker's session, watching it, taking it over. That loss isn't intrinsic to Codex. eigenform reads `~/.claude` only. Codex writes append-only rollouts to `~/.codex/sessions`, which are as legible as Claude's jsonl. So most of this is a **forest** problem, not an orchestration problem.

## Principles

1. **Claude orchestrates; eigenform observes.** The foundation design's "`claude` is only ever launched from inside the app, never by the daemon" extends to `codex`. Claude spawns workers through Bash (`codex-worker`). The daemon never spawns `codex` on its own. It runs `codex resume` only in a pty tab the user opens, exactly as it runs `claude --resume`.
2. **Codex's writer lock is the lease.** Codex `flock`s `$CODEX_HOME/thread-writer-locks/<id>.lock` while a process writes a thread. eigenform reads `/proc/locks` for liveness (never taking the lock), and refuses a `codex resume` tab while a worker holds it. Spike 14 checks that Codex enforces this itself.
3. **One drawer shape.** A Codex rollout renders into the same session JSON as a Claude transcript. Shell calls map to `Bash`, `apply_patch` to one `Edit`/`Write` per file, and web search to `WebSearch`, so the drawer, reach map, and preview take no Codex-specific path. The original tool name rides along as `input.codexTool`.
4. **Verification is Claude's job, not the worker's.** The skill makes this the loop's centre: read the diff, and run the acceptance command yourself.

## What's built

| piece | where | what |
|---|---|---|
| reader | `crates/codex` | enumerate rollouts, resolve id/prefix, rail facts (title, cwd, model, source → headless, turns), liveness via `/proc/locks`, rollout → drawer JSON |
| forest | `daemon::forest_json` | Codex rows join `/api/forest` tagged `engine: "codex"`; the SSE watcher also watches `~/.codex/sessions` |
| drawer | `daemon::session_json_route` | `/api/session/<thread id>/json` falls back to Codex |
| take the wheel | `daemon::pty_command` | `session=<thread id>` → `codex resume <id>` in the thread's cwd; refused while a writer holds it |
| parent link | `daemon::attach_codex_workers` | a Bash call whose output says `codex-thread: <id>` nests the worker's transcript in `tool.subagent` (agentType `codex`), the slot Agent subagents already use |
| UI | `webterm` | `codex` chip on rail rows; Bash-with-subagent and patch-diff views |
| protocol | `.claude/skills/codex-worker` | `spawn`/`resume`/`wait`/`status`/`result`/`diff`/`review`/`rm`; worktree per worker; prints the `codex-thread:` marker |

## Known gaps

- `.jsonl.zst` (compressed cold rollouts) aren't read, so very old threads drop out of the rail.
- The parent's drawer cache is keyed on the parent file. A worker still writing while Claude sits idle won't refresh the nested view until the parent's file changes (the same caveat as Agent subagents).
- No per-turn token spark for Codex rows: `spark` is one zero per user turn, so the `~N` count is right but the sparkline is flat.
- Liveness is Linux/WSL only (`/proc/locks`), matching the rest of the forest.

## Next: the divergence observatory

Send the same assignment to a Claude subagent and a Codex worker in sibling worktrees, and have eigenform diff the *trajectories*, not just the patches: files reached for (reach-map overlap), tests run, verdicts negated. The first order parameter is concrete: agreement rate on the acceptance command plus Jaccard overlap of touched files, per task, per model pair. That's a dataset of two agents with different priors on a shared task, measured rather than vibed.

## Adjacent: the artifact pane

The same dialectic has a second channel, the human's. An HTML/artifact pane split beside the terminal (an iframe over a daemon route serving a file the session wrote) plus plannotator-style annotation (select text in a plan or artifact, then comment, delete or replace) compiles to a structured critique. eigenform already has the right invariant for delivering it. Context surgery stages an edited prompt into the input and **never sends it**. Annotations would compile to that same staged prompt. Codex's review and the human's markup are both negations fed back as a turn the human releases. Design it separately.
