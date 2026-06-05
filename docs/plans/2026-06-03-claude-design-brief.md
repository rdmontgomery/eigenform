# Woland — design brief for claude.ai's design tool

Paste everything below the line into Claude's design tool. The current eigen/woland
build params (colors, fonts, 3 equal panes) are scaffolding, NOT intent — the brief says
so explicitly.

---

**Design the look, feel, and core affordances of a browser workbench for working with AI coding-agent sessions as a living, editable document.**

**Identity (provisional — help us think about it).** The command-line tool is called **eigen** (after eigenforms / fixed points of self-referential systems — von Foerster, Spencer-Brown). The browser surface is currently codenamed **woland** (after Bulgakov's daemon-host in *The Master and Margarita*). We're unsure which name should front the product, whether one should recede to "the engine," or whether something new fits better. **Treat naming/identity as a first-class deliverable**: propose 2–3 wordmark + identity directions, each justified by the lineage below, alongside the UI.

**Intellectual lineage — please honor this precisely, it is the design's spine, not decoration:**
- **Edward Tufte** governs the *surface*: maximal data-ink, zero chartjunk, micro/macro readings, sparklines, honest marginalia, information at the resolution of a single line. Every number shown must earn its ink.
- **Borges** governs the *structure*: "The Garden of Forking Paths." The session is a tree of timelines; every edit forks a new path and the old path still exists. Nothing is lost, only branched.
- **Bulgakov** governs the *soul*. The novel's line "manuscripts don't burn" is literal here: the source document can never be destroyed (every edit is a copy, never a mutation). And in the novel the author burns his manuscript *in a stove* — our central reference panel is called the **Furnace**. So design a tension: a Manuscript that cannot burn, beside a Furnace where the burning (of compute, of cache) actually happens.
- **Spencer-Brown & the cyberneticians** (Ashby, Bateson, von Foerster, Beer) govern the *logic*: a fork is *drawing a distinction*; a resumed session is a form re-entering itself (an eigenform); an edit is "a difference that makes a difference," and we want to show exactly how much difference each one makes. The user is an observer inside the loop they're steering.

**What it is.** A single-page, focused, instrument-like web app. An AI coding session is an append-only transcript. The product re-renders that transcript as a beautiful, *editable* document. Core surfaces:

- **The Manuscript (the star).** The session rendered as a calm, typographic, readable document — collapsible exchanges, quiet role markers, prose-like. Crucially it is a *writing surface*, not a chat log:
  1. **Leaf input** — at the document's end (the "leaf") you continue the session by typing. Cheap, safe.
  2. **Edit-any-turn in place** — click any earlier turn and rewrite it like editing prose. Never a popup "edit message" modal — it must feel like correcting a manuscript.
  3. **Fork-on-edit** — committing an edit forks a new timeline at that point and drops everything after; the original is never destroyed. Present this as *safe and generative*, the Borgesian branch, not a destructive action.

- **The Furnace (a fidelity instrument, secondary).** A raw terminal showing the real agent's ground-truth output. Its job is *calibration* — the builder checks that the Manuscript faithfully re-renders the agent even as the agent's output format drifts over time. Design it as a **collapsible reference channel**: a small live status glyph (a "norgie") most of the time, expandable to the full raw stream when fidelity needs checking, and dwellable for users who prefer the raw terminal. Give its frame real care — it's the second-order channel that keeps the projection honest.

- **The Forest (navigation).** A list of sessions to move between — but think of it as a *garden of forking paths*: small-multiple glyphs of each session's shape and its branches.

**The signature problem — the "burn indicator" (Tufte meets Bulgakov).** Editing the leaf (the end) is nearly free. Editing an early turn in a long session discards everything downstream and re-warms the model's cache — an expensive "burn." This cost is computable, and we want it *felt at the moment of editing*, as Tufte-grade information design, not buried in a panel:
- a quiet resting representation of edit-cost across the whole document (e.g. a margin sparkline or cost-gradient — cool/cheap near the leaf, warm/expensive deep in the past);
- a precise inline cost readout that surfaces when a turn is focused for editing ("editing here discards 37 turns · re-warms ~N context");
- a confirmation moment before a costly fork that makes the trade legible without nagging.
The "burn" is the Furnace's heat reaching the Manuscript — amber, reserved, meaningful. It signifies the *cost of creation*, not destruction (nothing is ever destroyed). Make this the most carefully designed moment in the product.

**Aesthetic.** Literary and instrument-like; the restraint of a fine book crossed with the seriousness of an operations console (think Beer's Cybersyn, Tufte's pages, Spencer-Brown's single mark). Dark is fine but not assumed — choose what serves legibility. Restrained accents with clear semantics: one register for the human, one for the agent, amber reserved for cost/burn and the "leaf." Generous, prose-like leading in the Manuscript; dense and exact in the Furnace. Minimal chrome — the surfaces are the UI.

**Do NOT treat our current build as a constraint:** it happens to be near-black `#0b0b0e`, 13px `ui-monospace`, three equal columns, glyphs ● ◇ ·. That's scaffolding we grabbed to start, not intent. Feel free to discard all of it.

**Sample transcript — render the design against this, not lorem-ipsum.** This is a 6-exchange session, oldest at top, the **leaf** at the bottom. The cost annotations in brackets are what the burn indicator should surface; they are not literal UI copy — design how they're shown.

```
session 8f3ad1 · woland · 6 exchanges

── exchange 1 ──────────────────────  [edit cost: BURNS — discards 5 turns · re-warms ~38k ctx]
● user      Let's render the JSONL transcript as collapsible exchanges in the right pane.
◇ assistant I'll add a session_html() renderer that groups each user turn with its
            assistant + system replies into one <details>. Source of truth stays the
            JSONL — we never mutate it.
· system    8.2s · 4 files read · 1 file written

── exchange 2 ──────────────────────  [edit cost: BURNS — discards 4 turns · re-warms ~31k ctx]
● user      Good. Now follow the session live so the pane updates as it writes.
◇ assistant Wired an SSE channel on /api/watch/:uuid; the pane re-fetches on each
            write and auto-scrolls unless you've scrolled up.
            ⏚ tool  Edit  web/src/main.ts  (+22 −3)
· system    5.6s

── exchange 3 ──────────────────────  [edit cost: warm — discards 3 turns · re-warms ~22k ctx]
● user      The pane flickers on every keystroke during a live turn.
◇ assistant The re-fetch fires per-line. I'll debounce to the JSONL's flush boundary
            instead of every append.
· system    3.1s

── exchange 4 ──────────────────────  [edit cost: cooler — discards 2 turns · re-warms ~14k ctx]
● user      Now let me edit a turn in place and have it fork.
◇ assistant That's edit_then_fork: re-author turn N, drop N+1…end, write a NEW
            session file, leave the original untouched. Copy-on-fork — the source
            can't burn even in principle.
            ⏚ tool  Bash  cargo test surgery::fork  →  ok, 12 passed
· system    11.4s

── exchange 5 ──────────────────────  [edit cost: cool — discards 1 turn · re-warms ~6k ctx]
● user      Show the burn cost in the margin as I hover each turn.
◇ assistant Added a per-turn cost: turns discarded downstream × cache-warm estimate,
            shaded cool→warm from the leaf backward.
· system    4.0s

── exchange 6 ──────────────────────  [edit cost: ~free — this is the leaf · nothing downstream]
● user      ▏                                              ← leaf · type here to continue
```

Notes for the design: exchange 1 is the **expensive** end (editing it throws away the whole session and re-warms the most context — show the strongest "burn" treatment here); the leaf at exchange 6 is **free** (cool/quiet). The gradient from free (bottom) to burning (top) is the core thing the resting whole-document view should make legible at a glance. Tool calls (`⏚ tool …`) and system timings (`· system 8.2s`) are first-class transcript content and need their own restrained, Tufte-quiet treatment — present but never shouting over the prose.

**Produce:**
1. The **overall layout** with the Manuscript primary, the Furnace collapsed to its norgie (and one shot expanded), and the Forest as forking-path navigation.
2. The **edit-in-place interaction**: a turn at rest, on hover, and mid-edit as inline prose (use exchange 3 or 4 from the sample).
3. The **burn indicator**: resting whole-document cost view (exchange 1 hot, leaf cold), focused-turn cost readout, and the costly-fork confirmation.
4. A **design-language sheet**: identity/wordmark options, palette with semantics, type, the marks for leaf / fork / burn.

Give me **two or three distinct directions** for the overall feel before refining — span the range from austere cybernetic instrument to warm literary editing surface.
