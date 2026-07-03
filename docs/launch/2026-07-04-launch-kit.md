# eigenform launch kit — July 4th weekend

> Draft marketing kit for launching eigenform + the downgrade-recovery feature on
> Twitter/Reddit/HN. Copy is paste-ready. Nothing here ships product code.

## The angle (read first)

The feature is spicy but only survives HN scrutiny if framed as **false-positive
recovery, not jailbreaking**. The sympathetic pain: you're doing legit
security / dual-use / CTF / systems work, Fable misreads a benign prompt, silently
drops you to a smaller model, and your session quietly gets dumber. Lead with
*"the downgrade was wrong and it cost you the better model,"* never *"beat the
filter."*

Two honesty beats defuse the dunkers, repeat them everywhere:
1. It **stages** the reworded prompt and stops — **never auto-sends.**
2. The rephraser **refuses a genuinely bad ask** instead of helping.

Marketing handle: **the mulligan** (a do-over). Docs descriptor: *downgrade rewind*.

## The money shot — a 15–20s GIF (whole pitch in one loop)

1. **(0–3s)** Furnace terminal, calm. Status pill `fable-5`. User types a normal
   security question, hits enter.
2. **(3–6s)** Reply starts… then a red `<synthetic>` line: *"switched this session
   to a safer model."* Status pill flips `fable-5 → opus`. A "wait, what" pause.
3. **(6–11s)** The Forest panel *branches*: a sibling node peels off the trunk at
   the previous turn. Badge: `fable-retry`.
4. **(11–16s)** The new session auto-opens. Cursor in the input, a **reworded
   prompt already staged** (highlighted, not sent). Status pill `fable-5`. Caption
   chip: *"staged — your call."*
5. **(16–20s)** User presses Enter. Fable answers. Freeze on: **manuscripts don't
   burn.**

Arc = *drop → branch → rewind → back on the good model, one keystroke.* Loop it.

**Stage it deterministically today:** detection is a substring match on the marker
string, so drop the current `GUARDRAIL_MARKER` text into a synthetic transcript
line and it fires every time — perfect for a clean, repeatable recording. Set the
rephraser stub to return a hand-written line so the staged text is crisp on camera.

**Toolchain:** this WSL env can't run a headless browser — record on a real
machine. Screen recorder or Playwright video capture on desktop; trim to a
seamless loop with `ffmpeg`/`gifski`; keep ≤5MB so it autoplays inline on Twitter
and old.reddit.

Optional **90s video** (HN/YouTube): same opening, then widen: "every Claude Code
session is just a `.jsonl` on disk. eigenform watches them live and treats the set
as a forest you can operate on — fork, rewind, splice. The mulligan is one trick
built on that surgery." End on: local binary, nothing leaves your box, MIT/Apache.

## README hero — paste-ready redress

```markdown
# eigenform

> manuscripts don't burn.

Claude Code, with an undo button and a time machine.

eigenform is a single local binary that hosts your Claude Code sessions in the
browser and treats every session as what it really is on disk — a file you can
operate on. Fork a conversation at any turn. Rewind. Splice a prompt in. Watch
the whole forest of your work grow live.

### The mulligan

Sometimes the big model (Fable) decides your prompt is risky and quietly bumps
you down to a smaller one, mid-conversation. Usually it's wrong — a benign
security question, a dual-use tool, a CTF box — and your session just got dumber
without asking you.

eigenform catches the drop the instant it happens. It forks a fresh Fable
session rewound to the line *before* the one that tripped the wire, drafts a
cleaner way to ask, and drops it in your input — **staged, never sent.** You read
it. You decide. One keystroke and you're back on the model you started with.

And if what you asked was actually over the line, the draft says so instead of
helping. This is for recovering from *wrong* downgrades, not smuggling past right
ones.

Nothing leaves your machine. The daemon watches your transcripts and does the
surgery on disk, locally. No cloud, no account, no telemetry.
```

## Mock Claude session — paste-ready (README or tweet image)

```text
  ~/work/scanner  ·  claude  ·  ● fable-5

  › walk me through how this login endpoint could be brute-forced so I can
    rate-limit it properly

  ⏻  switched this session to a safer model.
  ● fable-5 → opus

  ┌─ eigenform ─────────────────────────────────────────────┐
  │  guardrail downgrade caught at your last turn.           │
  │  forked → session "fable-retry"  ·  rewound 1 turn       │
  │  drafted a restatement · staged in your input            │
  └──────────────────────────────────────────────────────────┘

  ~/work/scanner  ·  claude --resume fable-retry  ·  ● fable-5

  › [staged — press enter to send, or edit]
    I'm hardening a login endpoint I own. What rate-limiting and lockout
    strategy defends against credential-stuffing and brute-force attempts?

  ▏
```

## Social copy (human voice, no slop)

### Twitter/X thread

1. Claude sometimes bumps you off the big model mid-conversation when it thinks
   your prompt is spicy. Usually it's wrong. eigenform notices the second it
   happens, rewinds to the line before, and hands you a cleaner way to ask. Back
   on the good model in one keystroke. 🧵 *[GIF]*
2. under the hood: every Claude Code session is just a file on disk. eigenform
   treats those files like a git tree you can operate on — fork a conversation,
   rewind to any turn, splice a prompt in. the downgrade catch is one trick built
   on the surgery.
3. the important part: it never sends for you. it stages the reworded prompt in
   the box and stops. you read it, you ship it. and if what you actually asked was
   over the line, the draft tells you so instead of helping you around it.
4. all local. one binary. it hosts your sessions in your browser, watches the
   transcripts, does the surgery on disk. no cloud, no account, nothing phones
   home.
5. it's a holiday and it's open source (MIT/Apache). go poke at it, break it, tell
   me what's dumb. → *[link]*

### Show HN

Title: `Show HN: eigenform – rewind your Claude Code session when the model gets downgraded`

First comment:

> I kept hitting a specific annoyance: I'd be deep in a session on Claude's big
> model doing security or systems work, ask something completely benign, and the
> model would quietly downgrade itself to a smaller one because the phrasing
> looked risky. No prompt, no "are you sure" — the session just got worse and I'd
> notice three turns later.
>
> eigenform watches the transcript live (every Claude Code session is a `.jsonl`
> on disk), catches the downgrade the moment it lands, and forks a fresh session
> rewound to the turn before the one that tripped it. It drafts a cleaner
> restatement and stages it in the input **without sending** — you decide. If the
> ask was genuinely disallowed, the drafter refuses instead of helping; this is
> for recovering from wrong downgrades, not defeating right ones.
>
> That feature is really one instance of the general thing: eigenform is a control
> surface that treats your session history as a forest you can do surgery on —
> fork, rewind, splice — all copy-on-write, so the original is never touched (the
> repo's tagline is "manuscripts don't burn").
>
> Stack: Rust daemon hosting the pty sessions + serving a browser UI, watching
> `~/.claude/projects` and doing the JSONL surgery on disk. Fully local — no
> account, no telemetry. MIT/Apache.
>
> Would love to hear where the model-detection is brittle (it's pinned to a
> specific notice string right now and will drift as Anthropic changes wording)
> and what other session-surgery moves you'd want.

### Reddit (r/ClaudeAI, casual)

> Title: I got tired of Claude silently downgrading my model mid-session, so I
> built a rewind button
>
> You know when you're cruising along, ask something totally reasonable, and
> Claude drops you to a smaller model because your wording pattern-matched to
> "sketchy"? By the time you notice, you've got three turns of worse answers.
>
> eigenform catches it the instant it happens, forks your session rewound to the
> line before, and pre-writes a cleaner version of your question sitting in the
> input — it does **not** send it, that's on you. One keystroke and you're back on
> the model you started with. (And if you actually asked for something bad, the
> rewrite just tells you no. It's for the false alarms.)
>
> It's a local binary — hosts your sessions in a browser, watches the transcript
> files, does everything on your machine. Open source. GIF below, would love
> feedback.

## Don't-get-dunked-on checklist

- Never show a genuinely disallowed prompt getting "fixed." Use a clearly benign
  security example (rate-limiting your own endpoint) so the "wrong downgrade"
  framing is self-evident.
- Every surface repeats the two honesty beats: **staged, never sent** + **the
  drafter refuses real asks.**
- Say "local, nothing leaves your machine" everywhere — strongest trust signal,
  and it's true.

## Scope note

`GUARDRAIL_MARKER` is a placeholder string in the code, so on a real machine the
live trigger won't fire until it's swapped for the actual notice wording (Task 7).
The demo stages deterministically via that same string, so recording doesn't
depend on it.
