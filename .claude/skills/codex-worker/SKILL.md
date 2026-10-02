---
name: codex-worker
description: Delegate a self-contained coding task to an OpenAI Codex worker (`codex exec`) running in its own git worktree, then verify and correct it — or have Codex review your own diff. Use when the user asks for Codex, a second model, a second opinion, a cross-model review, or parallel workers on independent tasks. Not for small edits you can make directly.
---

# codex-worker

Codex is a different foundation model with different blind spots. Use it for two things:
work done **in parallel**, and **adversarial checking** of your own work. Both only pay
off if you verify. A worker's report is a claim, not evidence.

`codex-worker` is the executable next to this file. Call it by that path, or as
`codex-worker` if it's on PATH (`just install-codex-skill` in the eigenform repo puts it
there and links this skill into `~/.claude/skills`). It needs `codex` on PATH, logged in.

Every `spawn`/`resume`/`review` prints `codex-thread: <id>`. Leave that line in the tool
output: eigenform reads it and nests the worker's live transcript under your Bash call,
so the user can watch it and open it with `codex resume <id>`.

## The loop

1. **Write the assignment.** Codex has none of your context. It sees the repo and your
   prompt, nothing else. Put the assignment in a file and pass `@file`. Include:
   - the goal in one sentence, and why it matters;
   - the files and directories in scope, and what must not be touched;
   - the **acceptance command**: the exact test/lint/build that must pass;
   - known pitfalls you've already found;
   - "Commit nothing. When done, end with: SUMMARY, FILES CHANGED, VERIFICATION (each
     command you ran and its result), OPEN QUESTIONS."

2. **Spawn.** One worker per independent task, at most 3 at once unless the user says
   otherwise. Name it after the task.

   ```sh
   codex-worker spawn retry-fix -- @/tmp/retry-fix.md
   codex-worker spawn retry-fix --effort high --base main -- @/tmp/retry-fix.md
   ```

   Default sandbox is `workspace-write` inside the worktree (`.codex-worktrees/<name>`,
   branch `codex/<name>`). Codex's sandbox blocks network by default. If the acceptance
   command needs the network, ask the user before using `--sandbox danger-full-access`.
   Never bypass the sandbox on your own initiative.

3. **Wait without polling.** Run `codex-worker wait <name>` as a Bash call with
   `run_in_background: true`. You'll be notified when it exits. Do other work meanwhile.
   Don't loop on `status` with sleep.

4. **Verify. This is the job.**
   - `codex-worker result <name>`: read the report, then distrust it.
   - `codex-worker diff <name>`: read every hunk. Look for scope creep, deleted or
     weakened tests, swallowed errors, and hardcoded expected values.
   - Run the acceptance command **yourself** in the worktree (`cd .codex-worktrees/<name>`).
     If the worker says "tests pass" and you didn't see them pass, they didn't.

5. **Correct.** Give the specific failure, not "try again":

   ```sh
   codex-worker resume retry-fix -- "cargo test -p net retry still fails: <paste>. The backoff must cap at 30s; don't change the test."
   ```

   Then go back to step 3. After three correction rounds without convergence, stop. Take
   the task back or ask the user. Don't spiral.

6. **Integrate or discard.** Pull accepted changes into the working branch (copy the diff
   or `git -C .codex-worktrees/<name> commit` then cherry-pick), and re-run the checks
   there. Then `codex-worker rm <name> [--force] [--delete-branch]`.

## Cross-model review

Before you call your own non-trivial change done, have Codex review it:

```sh
codex-worker review --uncommitted
codex-worker review --base main -- "focus on concurrency and error paths"
```

Treat each finding as a bug report to **verify**, not an order. Reproduce it or refute it
from the code.

## Report the divergence

When you and Codex disagree (an approach, a diagnosis, a finding you refuted), say so in
your reply to the user. Give what Codex claimed, what you claimed, and what you verified.
That disagreement is the point of running a second model; don't smooth it over.

## Reference

```
codex-worker spawn <name> [--base ref] [--model m] [--effort e] [--sandbox mode] [--schema file] [--here] -- <prompt|@file|->
codex-worker resume <name> -- <message|@file|->
codex-worker wait <name> [--timeout secs]
codex-worker status [<name>]
codex-worker result <name>
codex-worker diff <name>
codex-worker review [--base ref | --uncommitted | --commit sha] [-- instructions]
codex-worker rm <name> [--force] [--delete-branch]
```

`--here` runs in the current checkout with a read-only sandbox. Use it for
investigation, not edits. State lives in `.git/codex-workers/<name>/`: the event stream,
each prompt sent, the final message, and stderr. Read `stderr.log` there when a run fails
to start.
