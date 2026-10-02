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

## Adjacent: the artifact pane (step 1 built)

The same dialectic has a second channel, the human's. Step 1 is built: a split pane beside the terminal that renders the HTML, SVG, markdown and images the active session wrote or edited, nested subagent and Codex-worker writes included (`GET /api/session/:uuid/artifacts`, `crates/daemon/src/artifacts.rs`, `webterm/src/artifacts.ts`). It follows the newest write unless you pin one, and reloads when the file's mtime moves.

**Isolation.** `origin_is_local` admits *any* localhost origin, so a second port would not have isolated anything: a page there could open `/pty` and get a shell. Instead every `/artifact/…` response carries `Content-Security-Policy: sandbox …` without `allow-same-origin`, and the iframe repeats the sandbox. The document gets an opaque `null` origin even when opened directly in a tab. Verified in Chromium: the pty socket is refused, the `/api` read is blocked, and parent and storage are blocked. File scope covers only the session's own writes. HTML/SVG may pull non-hidden siblings (relative assets), markdown only sibling images, so a `plan.md` at a repo root doesn't expose the repo. Symlinks resolve before the check.

**Step 2 (built): annotations → staged prompt.** The pencil on a markdown artifact swaps the iframe for the annotator (`webterm/src/annotate.ts` pure core, `annotator.ts` DOM). The source renders as plain-text blocks in eigenform's origin, never via innerHTML, since the markdown is agent-written. Inline markdown stays as source, so every quote is findable in the file. Select text, then comment, delete or replace it. Marks persist per (session, file), and re-anchor by quote when the agent revises the file. A mark whose text vanished is kept as an orphan, because a critique of removed text can still matter. **Stage in terminal** compiles the marks into one critique in document order and types it into the input. It is never sent:

- the text goes inside a bracketed paste when the TUI enabled it (claude and codex both do), so its newlines are paste content, not Enter;
- without bracketed paste, it's flattened to one line;
- C0/C1 control characters are stripped first, so a note containing `\x1b[201~` can't close the paste early.

Verified end to end. A stand-in TUI enabling `?2004h` logged its raw stdin: one bracketed paste, no ESC inside it, and no CR/LF outside it.

**Step 3 (built): the plan gate.** Claude Code's plan approval, answered from the pane, plannotator-style. `eigenform plan-gate` prints an HTTP `PermissionRequest` hook (matcher `ExitPlanMode`, timeout 1800s) to merge into `~/.claude/settings.json`. The daemon (`crates/daemon/src/plan_gate.rs`) parks the hook request and holds it open. The pane opens to a **Plan review** of that session: the plan in the annotator, with **Approve** (`allow`), **Send back** (`deny` with the compiled critique as `message`), and **Decide in terminal** (empty 2xx, so Claude Code's own prompt). The rail tags the session `plan review` while it waits.

It's the first place Claude waits on eigenform, so every failure degrades to the normal TUI prompt: the daemon down (non-2xx), the hold timeout (1740s, answered as no decision just before Claude Code's 1800s cutoff), or the hook cancelled (the parked review drops itself). The gate can delay a plan; it can't wedge or silently approve one. The decision route requires a local Origin **and** a JSON content type, so a cross-site form post can't approve a plan. The hook route refuses any browser Origin that isn't local. Verified end to end with curl standing in for Claude Code: held until **Send back**, then answered `deny` with the critique. Spike 17 pins the Claude-side half: what `tool_input` carries, and whether the deny `message` reaches Claude as feedback.
