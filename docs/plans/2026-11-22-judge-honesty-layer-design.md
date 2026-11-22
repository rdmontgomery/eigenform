# The judge and its honesty layer — design

**Date:** 2026-11-22 · **Status:** step 1 (`crates/calibrate`) built; step 2 proposed

## Where it goes

[Workers and feeds](2026-11-12-workers-and-feeds-design.md) already reserves the slot. Its last sketched step is **System 1 routing**: "a compiled judge can answer 'does this need a model, and which one' before anything spends tokens, and defer when unsure." *Defer when unsure* is meaningless without honest probabilities, and a judge you can't retrain (Jev, an LLM behind an API, a vendor's score) only gets honest probabilities from the map you put after it. This doc builds that map and the loop that keeps it honest, and points it at the first place eigenform has both a question and a free stream of labels: the **Needs you** rail.

## Why the Needs you rail first

The rail already lists every item a feed says needs the operator, and every item ends in an action the daemon carries out (`POST /api/feeds/act`) or in expiry. So the daemon sees each item's label without anyone labelling anything. The question per feed kind is a single `noul`: *will the operator act on this item, rather than dismiss it or let it expire?*

Once the router acts on its answer, sinking or folding the items it thinks are "no", those items stop producing labels. That's the loop's real problem, not a side issue: a router that learns only from what it shows converges on its own prejudice. The audit rate and the `1/observe` weights exist for exactly this.

Two other places were considered and left for later:

- **The plan gate** has the cleanest labels (every plan ends in approve or send back), but there is nothing to do with a prediction there that doesn't defeat the gate.
- **The session forest** has hundreds of retrospective labels on day one ("did the operator reply within ten minutes of this turn ending"), but they are behavioural proxies whose meaning is fuzzy. A good second question once the loop is proven.

## The loop (built: `crates/calibrate`)

Per question, on a pinned judge version:

| step | code | what it does |
|---|---|---|
| serve and log | `Judgment` | the judge's raw `p`, and `observe`, the chance its label would ever be seen, fixed at serve time |
| label two ways | `route`, `coin` | shown items are labelled by the operator; items below the cutoff are still shown at rate `audit`, by a coin seeded on the item id so a restart doesn't re-roll it |
| fit per question | `fit`, `MapKind::for_labels` | weighted log loss, Newton, weak prior at the identity; identity under 30 labels, intercept to 300, Platt after |
| ship only if better | `refit` | holds back the newest labels; the challenger is promoted only if it beats the serving map on them, and never on fewer than `min_holdout` |
| watch | `ece`, `log_loss` | on the newest labels, weighted |

The cutoff comes from costs, `c_fp / (c_fp + c_fn)`, so changing what a miss costs needs no new labels.

One correction to how the weights were described before: selection on the judge's own score does **not** bias a correctly specified fit, because logistic regression is conditional on its input. It biases every *population* number: the base rate, the ECE, the log loss the ship gate compares, and the fit whenever the map family can't represent the true curve. The test `audit_weights_undo_the_routers_selection` shows the base rate off by more than 0.1 unweighted and within 0.02 weighted.

Beta calibration, isotonic and Venn–Abers are the next rungs; the ladder is an enum so they slot in.

## Step 2: wiring (proposed)

**The judge stays outside the daemon.** The promise is that the daemon only talks to loopback. The judge is a loopback endpoint the operator runs, configured in `~/.eigenform/judges.json` (`{ "kind": "review", "url": "http://127.0.0.1:8091/judge", "question": "…" }`), that answers `{ "p": 0.83, "model": "jev-2026-11" }`. Whether it calls Jev, a local model, or Claude headless is its business, and so is the API key. That is the honest boundary: the operator chooses to send an item's title off the machine; eigenform never does on its own.

**Labels need one contract field.** An action's `label` is free text, so the daemon can't tell "Start review" from "Dismiss". Add an optional `actions[].outcome: "act" | "dismiss"` to the feed contract (unknown fields are already ignored, so providers can send it first). An item that leaves the feed with no action is `dismiss` after a grace period.

**State.** `~/.eigenform/state/judge/<kind>/<model>.jsonl` holds the judgments; `map.json` beside it holds the serving map and the last `Refit`. A refit runs on each new label once there are enough, and a new judge `model` string starts a fresh map at the identity.

**The rail.** Items the router shows sort by honest probability within the section; items it doesn't fold into a "probably not" line with a count, still one click away. An audited item looks like any other. The inspect dock gets a small reliability readout per kind: labels, serving map, holdout ECE.

## Open

- Volume. At 10–30 items a day per kind, the intercept is all the labels can afford for the first weeks. That's fine; it's the rung for "only the base rate moved".
- Fan-out. Five micro-judgments per item and a stacked logistic combiner over their logits is the same `fit` with more inputs. Not before the single `noul` has a working loop.
