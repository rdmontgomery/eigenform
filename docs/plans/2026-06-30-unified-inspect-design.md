# crates/inspect — unified config inventory (v0.1)

Status: shipped 2026-06-30. Implements the CLI foundation of the aleph-parity
"call to action": a single config model with many faces, token-budgeted,
projected through the existing render-crate `View` IR. Browser surface and
`$EDITOR` actionability are deferred (see "Deferred" below).

## Why

aleph's `inspect` was one rich data model rendered three ways (token-budgeted CLI
summary, machine JSON, interactive tree). eigenform's equivalent had fragmented
into independent `skills` and `memory` crates, each with a bespoke `render_tree`
that `println!`s a static, untokened, name+path text dump. The smarts
(layer-shadowing — which skill WINS across cwd/repo/global/plugin — and plugin
namespacing) already existed; they were just trapped behind `println!`.

The fix was **not** to walk back eigenform's browser-first bet, but to apply
eigenform's own multi-renderer render-crate pattern to a unified config model and
give it token weights.

## Principle

Same projection split the session view already uses: a **domain crate collects**
an internal model, **`eigenform-render` projects** it to text / json / (later)
html. `eigenform-inspect` plays the role `eigenform-forest` plays for sessions.

```
eigenform-inspect::collect / collect_all_projects  →  InspectData
eigenform-render::inspect_view(&InspectData)        →  View   →  render_text
eigenform-render::inspect_json(&InspectData)        →  String (JSON)
```

`InspectData` is an internal, refactor-freely model — **not a published schema**
(same posture as the render View). The JSON projection exists now because it is
cheap and proves the multi-renderer claim; html lands when the browser consumes
it.

## Model

```rust
struct InspectData  { layers: Vec<InspectLayer> }
struct InspectLayer { label: String, skills: Vec<SkillItem>, memory: Vec<MemoryItem> }
struct SkillItem    { name, description, path, size, tokens, wins, namespaced }
struct MemoryItem   { name, description, kind, path, size, tokens }
```

One `InspectLayer` per resolution slot (`global`, `plugin:<p>`, `repo` /
`repo:<project>`), each carrying the skills it contributes and (for repo layers)
the project's auto-memory. `InspectData::tokens()` / `InspectLayer::tokens()` sum
the estimates so every level — total, layer, group, entry — shows a `~N tok`
weight.

### Two entry points

- `collect(home, cwd)` — one resolution context. Shadowing is meaningful here, so
  `SkillItem::wins` is computed (highest-precedence bare-name contribution wins;
  plugins are namespaced and always win in their namespace). This is the default;
  it is also where `skills tree` / `memory tree` live.
- `collect_all_projects(home)` — a flat machine-wide inventory. Cross-project
  shadowing is **undefined** (project A's repo can't shadow B's), so `wins` is
  not computed there; plugin namespacing still is.

The shadowing/namespacing logic mirrors `skills::render_tree` so the legacy tree
and the unified model agree on who wins (`ShadowMap` in `crates/inspect`).

## Token budgeting

`~4 bytes/token` (`estimate_tokens`), the cheap dependency-free rule of thumb —
a budgeting aid for a context-surgery tool, not a billing oracle. Added to
`Skill`/`MemoryEntry` at scan time (`size` bytes + `tokens`) and surfaced
everywhere: the unified view, the enriched `skills tree`/`memory tree` text, and
the JSON. Formatting (`fmt_tokens`): `~N tok` under 1k, `~N.Nk tok` above.

## CLI

```
eigenform inspect [--cwd <path>] [--all-projects] [--render text|json]
```

`--render html` errors clearly (deferred until the browser consumes it), matching
the session view's deferral. `skills list` / `memory list` no longer `bail!`
without `--all-projects`: the single-project view is the common case, so the
no-flag invocation now defaults to the current context (the papercut aleph didn't
have).

## Deferred (follow-ups, not regressions)

These are the parts of the call-to-action this slice intentionally leaves for
later, in priority order:

1. **Browser surface.** webterm's inspect is session-centric (reach map +
   transcript). The unified model + `inspect_json` are exactly the data a
   navigable config tree in webterm would consume — a projects × skill-names
   matrix with tinted override cells, drill-down to content, token weights. The
   SSE infra and the JSON projection are ready; the UI is not built yet. This is
   where the `inspect_html` projection lands.
2. **`$EDITOR` / actionability.** aleph let you `e` to edit the file under the
   cursor. eigenform inspection is read-only. Closing the browse→edit→refresh
   loop (in the terminal and the browser) is the next actionability step.
3. **agents + CLAUDE.md + settings + MCP layers.** aleph's model also captured
   agents, CLAUDE.md, settings.json, and MCP config per layer. `InspectLayer`
   has room to grow these as sibling vectors alongside `skills`/`memory`.
