# Eigen — terminal-centerpiece GUI (design)

**Date:** 2026-06-11
**Status:** Design agreed; implementation not started.
**Supersedes (in emphasis, not in code):** the woland Manuscript-centric GUI, now
paused. This app lives **beside** woland and shares the same daemon.

## 1. Intent & reframe

A second browser GUI for eigen — codename TBD. Where woland made the semantic
*Manuscript* the centerpiece and treated the terminal as a collapsed fidelity
instrument (the "Furnace"), this app **inverts that**: a flawless xterm.js terminal is
the centerpiece, and structured niceties are built *around* it — in panes, not as an
overlay (overlay-onto-terminal is a later UX addition, not v1).

The motivating frustration: hacking the claude-code pty's I/O into the Manuscript made
the Manuscript gimmicky and the terminal second-class. Inverting fixes both — the
terminal is just the terminal (rock-solid), and the niceties stop fighting it.

The deeper reframe: **the daemon is a session host, not a pty bridge.** Claude's
servers keep working for you while you navigate away. Multitasking across many live
sessions is the entire point of the tool, not a bolted-on feature.

### Relationship to woland
Decision: **keep woland fully intact; build the new app beside it**, sharing the daemon
(zero risk to the existing thing, easy A/B of the two mental models). No shared frontend
code: we *lift* the proven pieces (the xterm ↔ `/pty` WebSocket wiring, the daemon
`fetch` calls) and leave Manuscript / cooling clock / epigraphs / `woland.css` behind.

## 2. Architectural pillars

1. **Daemon-managed persistent ptys.** A registry of live ptys keyed by id; WebSockets
   attach/detach without killing `claude`; background sessions keep running; reload
   re-attaches to the live process.
2. **Sidebar = full roster, tabs = open subset.** The left rail lists *all* sessions
   (running + recent from disk) with live-state badges; tabs are the ones you've opened.
3. **Terminal-primary, transcript on demand.** Each tab is a full-width terminal; a
   structured transcript drawer slides in when you want to navigate or fork.

### Reused unchanged
Daemon session resolution; `/api/forest` & `/api/sessions` (state × liveness × recency);
`/api/session/:uuid/json` + SSE `/api/watch/:uuid`; the fork endpoint
(`POST /api/session/:uuid/fork`, copy-on-fork); and the `merge_candidates` /
`immediate_subdirs` launcher backend (merged 2026-06-10, `crates/projects`).

## 3. The daemon as session host

Today `pty_ws` calls `bridge(socket, command)` — the pty's lifetime *is* the socket's.
We break that coupling.

**Pty registry.** A daemon-global `SessionHost`: `Mutex<HashMap<PtyId, LivePty>>` owning
ptys independent of any socket. Each `LivePty` holds the `MasterPty` + child handle, a
stdin writer, a **single-viewport parsed-terminal model** (see §4), and metadata: id,
cwd, resolved session uuid (once known), spawn time, last-activity time.

**Attach protocol.**
- `GET /pty?new=<cwd>` / `?session=<uuid>` — spawn as today, but **register** the pty and
  return its `ptyId` so the client can re-attach later.
- `GET /pty?attach=<ptyId>` — re-bind a fresh WebSocket to an existing `LivePty`:
  repaint the current viewport snapshot, then stream live. Output fans out to
  zero-or-more attached sockets (detached = process keeps running, no viewers).

**Lifecycle.** A pty lives until the `claude` child exits (reap, mark `exited`, keep
briefly for a final view) or an explicit `DELETE /api/pty/:id`. Switching tabs or
reloading does **not** kill it. A GC sweep reaps long-dead exited entries. Daemon
restart loses live ptys (children die with the parent) → those sessions fall back to
`claude --resume` on next open.

**Pid authority.** `~/.claude/sessions/*.json` is the authoritative store of claude's own
recognition of its process ids. The registry is a **view over** that truth — used to
reconcile our ptys against real pids, detect liveness, and match a spawned pty → its
session uuid. The daemon `Config` already carries `sessions_dir` for `<pid>.json`.

**Discovery.** `GET /api/pty` lists live ptys (id, cwd, uuid, state, age) so the frontend
can reconcile its sidebar against what's actually running after a reload.

## 4. Terminal fidelity & re-attach (settled by spike 09)

**Spike 09 (CONFIRMED, claude 2.1.170):** the main TUI enters the **alternate-screen
buffer** (`\e[?1049h`). Consequences:

- **No scrollback ring-buffer.** In the alt-screen, no traditional terminal scrollback
  accumulates — claude owns its viewport and repaints in place. The server-side
  parsed-terminal model needs only the **current viewport grid** (one screen). On
  re-attach: serialize that grid → one clean repaint → resume live streaming. No
  raw-byte re-animation, no unbounded buffer.
- **Self-healing on resize.** Alt-screen apps redraw on `SIGWINCH`; if the re-attaching
  client is a different size, resize the pty and claude repaints the whole screen itself.
- **"Scroll to first input" in the terminal pane is claude's own feature, live only** —
  not reconstructable by us after the fact. Durable, navigable-to-origin history is the
  **JSONL transcript drawer** (§6), independent of terminal state / reload / reattach.
- **Input fidelity (acceptance criterion for "flawless").** claude negotiates mouse
  tracking (SGR 1006), bracketed paste (2004), focus reporting (1004), the **Kitty
  keyboard protocol** (`\e[>1u`), and synchronized output (2026). xterm.js config/addons
  must mirror this mode set or keys/mouse/paste will misbehave.

Reconciles with **spike 08**: the trust/permission preamble renders on the *normal*
buffer before the TUI; the main TUI is alt-screen. Two phases, both correct.

## 5. Sidebar + launcher

**Sidebar (full roster).** Two sources reconciled: **live** (registry ∩
`sessions/*.json`, sorted to top) and **recent** (`/api/forest`, even if not running).
Each row: a **state badge**, the **editable label**, a dim **cwd chip**, relative
recency. Click → open as a tab (*attach* if live, `claude --resume <uuid>` if only on
disk). The key affordance: a **live row that isn't open yet** — "claude's still working
over here, go look."

**State taxonomy:**
- **working** — pty produced output recently (registry activity timestamp).
- **waiting-for-you** — claude blocked on the selector widget (trust / permission /
  AskUserQuestion / plan), detectable from the parsed grid per **spike 08**. The badge
  that earns the tool its keep.
- **idle** — live but quiet at the prompt.
- **done / exited** — child reaped; kept briefly, then it's just a recent row.

**Launcher (new session).** A `+ New session` entry opens a picker backed by a new
endpoint `GET /api/candidates` wrapping `merge_candidates` / `immediate_subdirs`:
recents first, then unseen immediate subdirs of a configured **workspace root** (default
`~/projects`, **soft** — free-text arbitrary paths still allowed, no cage). Type-to-filter
reuses the **fuzzy-picker design** (`docs/plans/2026-06-09-woland-blocked-input-and-fuzzy-picker-design.md`).
Picking launches `/pty?new=<cwd>`; typing a new name `mkdir`s under the root, then
launches.

### Tab labeling
User-editable with a smart default. Seed = ai-title (`aiTitle` in JSONL, derived by
`eigen_forest`) → cwd folder name → first-prompt snippet → "new session" as it degrades.
A dim cwd chip always shows for context. Double-click renames; the override persists per
session uuid (localStorage for v1). Rationale: if a user always launches from
`~/projects`, every cwd tail is "projects" → no signal; the ai-title is the real
distinguisher but arrives late, so the editable smart default covers the cold-start gap.

## 6. Tab + transcript drawer

**A tab is one session.** Full-width xterm attached to the live pty, plus a slim header:
editable label, state badge, cwd chip, **drawer toggle**, close button. **Close detaches**
(the pty keeps running) — it drops back to a sidebar row, it does **not** kill claude. An
explicit kill is a separate, confirmed action (`DELETE /api/pty/:id`).

**The transcript drawer** slides in on demand (terminal stays primary; drawer
overlays/insets from one side). It renders structured JSONL from `/api/session/:uuid/json`,
kept live by SSE `/api/watch/:uuid` (auto-scroll unless you've scrolled up — woland's
pattern). Content:
- **Collapsible turns** — user and assistant turns as foldable blocks; a multi-message
  assistant turn (tool-use rounds before the next user turn) collapses as one group.
- **Tool-call drill-down** — each tool call collapsed to name + one-line summary; expand
  for full input params and result output.
- **Per-turn edit-and-fork** — a fork control on each user turn opens that turn's text for
  editing, then `POST /api/session/:uuid/fork {turn, text}` (copy-on-fork, source
  untouched), returns the new uuid, which **opens as a fresh tab** and appears in the
  sidebar.

Terminal ↔ drawer are the same session, two readings: live-raw vs structured-durable. The
drawer is the deep-history surface that satisfies "scroll to the first input."

## 7. Phasing (v1 = the spine, not feature-completeness)

1. **Session host (load-bearing backend).** Registry: spawn-and-register, attach/detach
   without kill, single-viewport snapshot on attach, resize-repaint, child reap,
   `DELETE /api/pty/:id`, `GET /api/pty`, `sessions/*.json` reconciliation. Heavy TDD.
2. **Shell + terminal.** New app beside woland; sidebar (live + recent reconciled, state
   badges), tabs, full-fidelity xterm attach. Acceptance: mouse/paste/focus/Kitty-keyboard/
   sync-output verified against a live claude.
3. **Launcher.** `GET /api/candidates`; fuzzy picker; mkdir-under-workspace; soft
   `~/projects` default.
4. **Transcript drawer.** Structured turns + SSE, collapsible turns, tool drill-down,
   per-turn edit-and-fork → new tab.

## 8. Feature backlog (recorded, not built)

- **User-turn spine + jump-nav** — a "your prompts only" filter and keyboard jump to
  prev/next user turn, because user turns get buried under long runs of assistant/tool
  turns. Small once turn structure exists; pairs naturally with fork (user turns are the
  fork points). *First feature to add after v1.*
- Overlay niceties layered *onto* the terminal (the deferred UX addition).
- Multiple named workspaces (several roots as sidebar sections).
- Cross-session search.
- Daemon-restart session recovery (beyond `--resume`).

## 9. Testing posture

- **Rust crates stay TDD** — registry lifecycle, candidates, pid reconciliation, state
  taxonomy: all unit/integration-testable headless.
- **Claude-internals behaviors get spikes** (`notes/spikes/`, e.g. 08 selector widget,
  09 alt-screen) rather than brittle e2e.
- **Frontend** — data/transform layers (label derivation, state taxonomy, candidate
  merge) unit-tested like woland's `cooling.test.ts`; the live terminal verified manually
  against real claude.
- **CLI mirror** (standing rule) — `eigen sessions` / `eigen forest` already mirror the
  roster; add a CLI surface for the candidates/launcher list so the launcher is
  reproducible from the terminal.
