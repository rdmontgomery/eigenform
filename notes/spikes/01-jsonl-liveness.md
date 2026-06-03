# 01 — JSONL liveness

**Claim:** Claude Code writes the per-session JSONL incrementally during a live session, not as a single dump at session end.
**Status:** CONFIRMED
**claude version:** 2.1.161
**Date:** 2026-06-02

## Procedure

Inside an active `claude` session (this one, in `/home/rdmontgomery/projects/eigen`):

```
$ ls -la ~/.claude/projects/-home-rdmontgomery-projects-eigen/*.jsonl
```

Observe file size during the session.

## Result

```
.rw------- 47k rdmontgomery  2 Jun 22:28 /home/rdmontgomery/.claude/projects/-home-rdmontgomery-projects-eigen/277a983f-a862-4f84-a157-3f45bf456c1d.jsonl
```

File existed and had committed bytes before the session ended. Repeated `ls` calls over the conversation showed growth.

## Implication

The forest, context inspector, and token economy can all be built on a tail-the-file model. The pty layer is freed from any responsibility for reconstructing turns from terminal output. This collapses the architecture: pty owns the input channel + live overlay; everything durable comes from `fs::watch` on the JSONL.
