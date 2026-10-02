# 14 — codex concurrent resume

**Claim:** Codex allows one writer per thread. While a `codex exec` worker holds a thread, a second `codex resume <id>` (or `codex exec resume <id>`) is refused. It does not interleave appends into the same rollout or silently fork. The held lock shows in `/proc/locks`, which is what eigenform reads for liveness.
**Status:** PENDING. Source-read supports it; not yet observed on a real install.
**codex version:** _fill in: `codex --version`_
**Date:** 2026-10-02

## Why it matters

eigenform's "take the wheel" (open a Codex row in a pty with `codex resume`) and the daemon's lease refusal both assume Codex's own writer lock is the mutual exclusion. If it isn't, two writers can corrupt the worker's history while Claude is still driving it.

Source evidence (openai/codex `main`, 2026-10): `codex-rs/rollout/src/writer_lock.rs` keeps `$CODEX_HOME/thread-writer-locks/<thread_id>.lock` and takes it with `File::try_lock()` (flock). On `WouldBlock` the acquire fails with "an active writer". What a CLI does with that failure (an error exit, read-only, or a fork) is unverified.

## Procedure

```sh
# 1. A worker long enough to overlap with.
cd <some git repo>
codex-worker spawn lockprobe -- "Count to 200 slowly: run 'sleep 1' between each number using the shell tool. Edit nothing."
ID=<codex-thread from the output>

# 2. The lock as the kernel sees it (pid should be the codex exec process).
ls -la ~/.codex/thread-writer-locks/
grep FLOCK /proc/locks
stat -c '%d %i' ~/.codex/thread-writer-locks/$ID.lock   # dev, inode → match the /proc/locks entry

# 3. eigenform's view (daemon running): the row should be live with that pid.
curl -s localhost:4317/api/forest | jq '.[] | select(.engine=="codex") | {uuid, live, state, pid}'

# 4. A second writer while the first is running. Paste the exact output of each.
codex exec resume $ID "say hi"            # headless
codex resume $ID                          # interactive (in another terminal)

# 5. Rollout integrity after both: one session_meta, monotonic timestamps, no duplicate turns.
wc -l ~/.codex/sessions/*/*/*/rollout-*-$ID.jsonl
jq -r '.type' ~/.codex/sessions/*/*/*/rollout-*-$ID.jsonl | sort | uniq -c

# 6. eigenform's lease: double-click the row in the rail while the worker runs.
#    Expect a resume-refused event "codex thread is held by a running worker (pid N)".
```

## Result

_Paste real output. Don't summarise._

## Implication

- **CONFIRMED:** the lock is the lease. eigenform's refusal is UX on top of a guarantee Codex already enforces.
- **REFUTED (interleaves or forks silently):** eigenform must hold its own lease. Before `codex resume` the daemon would need to check for a running `codex-worker` pid in `.git/codex-workers/*/pid`, and `codex-worker resume` would need to refuse while a pty tab holds the thread.
- **No lock dir on this version:** liveness falls back to "never live". Pin a minimum Codex version in the README.
