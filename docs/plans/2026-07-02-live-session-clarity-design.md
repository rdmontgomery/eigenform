# Live-session clarity — design

*2026-07-02*

## Problem

The rail (forest) and the tab strip both use a single status **dot** plus, in the
rail, a faint "· live" text. Today those two signals are **decoupled and use
mismatched vocabularies**, which makes the dot read as buggy:

- The dot's glow is a pure function of the raw `state` string
  (`dotClass` in `shell.ts`). Only `working` (expanding-ring + pulse) and
  `waiting` (pulse) animate.
- The "· live" text is a pure function of the separate `live` boolean.

Two state systems feed these with different words:

| Source | Vocabulary | Meaning |
|---|---|---|
| Registry PTY (eigenform-spawned) | `working · waiting · idle · exited` | inferred from terminal output |
| Forest / disk (incl. sessions outside eigenform) | `working · ready · recent` | inferred from the JSONL tail; `recent` = **not live** |

Consequences visible to the user:

1. A session running **outside eigenform** emits `state:"working"` (so the dot
   **glows**) but the roster forces `live:false` (so **no "· live" text**). It
   looks live but isn't labeled live.
2. Forest `ready` / `recent` aren't in the dot's `KNOWN_STATES` set, so they
   silently fall through to a dull `idle` dot — a finished-turn session looks
   identical to a dead one.

There are **no Claude hooks** wired in; turn state is inferred. This PR fixes the
*display* on the signals we already have. (Injecting real `Stop`/`Notification`
hooks for authoritative turn state is a possible follow-up, out of scope here.)

## The model: two orthogonal channels

The dot is asked to carry two independent dimensions. Split them explicitly.

**Provenance** (who owns the process) → **fill style** of the dot:

| `liveness` | Meaning | Dot fill |
|---|---|---|
| `eigenform` | live PTY eigenform spawned (attachable) | **solid** |
| `external` | live claude running outside eigenform (can't attach) | **hollow ring** |
| `none` | no live process (exited / recent / on disk) | **faint flat** |

**Activity** (turn state) → **color + animation** of the dot:

| `activity` | Meaning | Color | Animation |
|---|---|---|---|
| `working` | assistant running | running amber | expanding ring + pulse |
| `waiting` | your turn (blocked on you / turn complete) | accent | pulse |
| `idle` | live but quiet at a prompt | slate | none |

**Only live provenances animate.** A `none` dot never glows, regardless of
activity — this kills bug #1 (a dead/disk session can no longer pulse). Both
`eigenform` and `external` live sessions glow; the hollow-vs-solid fill tells
them apart at a glance (bug #2 fixed: `ready`→`waiting`, `recent`→`none`).

## Mapping

Registry PTY → `{activity, liveness}`:
- `working` → `working`, `eigenform`
- `waiting` → `waiting`, `eigenform`
- `idle`    → `idle`,    `eigenform`
- `exited`  → `idle`,    `none`

Forest → `{activity, liveness}`:
- `working` → `working`, `external`
- `ready`   → `waiting`, `external`
- `recent`  → `idle`,    `none`

## Changes

**`roster.ts`** (pure data layer)
- Add `liveness: "eigenform" | "external" | "none"` and
  `activity: "working" | "waiting" | "idle"` to `RosterRow`.
- Exported pure helpers `ptyActivity(state)` / `forestActivity(state)` so the
  renderer and the tab strip share one mapping.
- `live` boolean is **unchanged** (still `true` for registry rows, `false` for
  forest rows) so attach/launch logic and existing tests keep working.

**`shell.ts`** (rendering)
- Replace `dotClass(state)` with `dotClasses(activity, liveness)` emitting
  `dot dot--<activity> dot--<eigenform|external|dead>`.
- Rail rows: dot uses the compound classes + a `title` tooltip spelling out the
  full state; the "· live" text becomes a short turn-state tag
  (`· running` / `· your turn` / `· live`).
- Tab strip: route the tab dot through `dotClasses` (tabs are always eigenform;
  dead/exited tabs → `none`).
- Rail footer: count "working" across live sessions via `activity`/`liveness`.

**`style.css`**
- Rewrite the `.dot` block into the two-channel model: `activity` classes set a
  `--dot-color` var; provenance classes render it (solid / hollow ring / faint);
  the glow animations live on compound `provenance.activity` selectors so only
  live dots animate. The ring keyframe reads `--dot-color` so an external
  running dot rings in the right hue.

## Testing

- Unit: extend `roster.test.ts` for the `liveness`/`activity` mapping across all
  six source states (4 PTY + 3 forest, minus overlap).
- Manual: throwaway daemon + spawn an external `claude` to confirm the hollow
  ring appears and glows while eigenform sessions stay solid. (Headless Chromium
  is unavailable in this WSL env — verify via bundle grep + DOM assertions.)
