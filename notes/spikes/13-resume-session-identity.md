# 13 — Resume keeps the session id; the pid file is lazy (claude 2.1.200)

**Claim:** On claude 2.1.200, interactive `claude --resume <uuid>` (1) continues
the SAME session id, appending to the same `<uuid>.jsonl` (no new uuid is
minted), and (2) does NOT write `sessions/<pid>.json` at startup — the pid
authority appears lazily, on later activity, or not at all while idle.
Therefore staged-input ("seed") delivery must not wait on any daemon-announced
uuid or claude-internals file; **pty output quiescence** is the startup signal
that survives claude-internals churn.
**Status:** CONFIRMED
**claude version:** 2.1.200 (Claude Code)
**Date:** 2026-07-03

## Why this matters

The Fable-retry / fork-edit flows stage a prompt into a resumed branch's input.
The first implementation delivered the seed inside `onSessionUuid` — the client
handler for the daemon's `{"type":"session","uuid"}` frame — but that frame is
only ever broadcast by the fresh-session JSONL watcher, never for resumes, so
seeds were silently dropped. The obvious fix (watch `sessions/<pid>.json`,
which `host::reconcile` documents as "claude's own recognition of its process
ids", written at startup) turned out to be built on stale internals.

## Procedure

1. **Pid file is lazy.** Spawned `claude --resume` through a throwaway daemon
   (port 4399) and watched `~/.claude/sessions/`: no `<pid>.json` appeared in
   4+ minutes of the resumed session sitting idle at its input box. A pid file
   from an older 2.1.199 session exists and records `"version":"2.1.199"` —
   and its `sessionId` equals the *resumed-from* uuid, corroborating (1).
2. **Resume keeps the id.** The resumed session appended to the SAME
   `<uuid>.jsonl` at startup (observed: an `away_summary` system row; file
   mtime bumped within a second of spawn). No new JSONL appeared. A second,
   independent instance: this very analysis session was itself resumed and
   kept its original uuid.
3. **Output-quiescence seeding works end-to-end.** A ws client (mirroring
   `shell.ts`: first binary frame, then 400 ms of output silence, 15 s hard
   cap) connected `?session=<scratch-uuid>` to the throwaway daemon, typed a
   marker with no trailing newline once output settled (+505 ms), and the
   marker echoed back in the pty output — i.e. it landed in the TUI input box,
   staged and unsent.

## Implications

- **Seed delivery is client-side only:** arm on connect when the tab carries
  `seedInput`, deliver on `seedDue` (quiet ≥ `SEED_QUIET_MS`, or
  `SEED_HARD_CAP_MS` unconditionally). No daemon watcher, no session frame
  dependency, no claude-internals dependency.
- **No uuid adoption problem for resumes:** since resume keeps the id, a
  fable-retry / fork tab's `descriptor.uuid` (the branch uuid) stays correct,
  and the transcript drawer watches the right file.
- `host.rs`'s reconcile doc-comment ("a `--resume` pty whose uuid changes from
  the resumed one") describes pre-2.1.200 behaviour. Harmless today
  (first-writer-wins and the ids now agree), but don't build on it.

## Version-drift flag (vetting-claude-internals)

Spikes 10 and 12 were CONFIRMED on **2.1.199**; the installed claude is now
**2.1.200**. Spike 10 (resume model derivation) and spike 12 (silent-flip
guardrail signature) were NOT re-run here — re-vet them before shipping
features that lean on their claims.
