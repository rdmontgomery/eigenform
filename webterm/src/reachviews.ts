/**
 * reachviews.ts — the "bands" reach view: distance-from-home lanes.
 *
 * The docked reach map is a short, wide strip that the radial spiderweb
 * (reachmap.ts) fills awkwardly — its SVG-unit labels scale down to a few
 * pixels. Bands render as plain HTML (crisp text at any size) and organize
 * reach by trust zone, top (in the workspace) to bottom (off the box):
 *
 *   workspace → repo · shell · agents → this machine → off-box · external
 *
 * The separation is the point: localhost (your own daemon) sits in "this
 * machine", never in the off-box lane with genuinely external hosts, so the
 * bottom lane is a clean "did this session actually leave the box?" signal.
 *
 * `live` is the per-node event count revealed at the current scrubber cursor,
 * so bands animate under the same transport as the spiderweb.
 */

import type { ReachModel, ReachNode, ReachKind } from "./reach.ts";

export interface ReachViewCtx {
  color: Record<ReachKind, string>;
  ring: Record<ReachKind, number>;
  trunc: (s: string, n: number) => string;
}

function elh<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}

/** Nodes reached by the current cursor, richest first. */
function revealedNodes(m: ReachModel, live: Map<string, number>): ReachNode[] {
  return m.nodes
    .filter((n) => (live.get(n.id) ?? 0) > 0)
    .sort((a, b) => (live.get(b.id) ?? 0) - (live.get(a.id) ?? 0));
}

// Distance-from-home lanes. Ring 4 (off-box) is the security signal; localhost
// classifies as ring 3 ("this machine"), so it never lands in the off-box lane.
const BAND_LABEL: Record<number, string> = {
  1: "workspace",
  2: "repo · shell · agents",
  3: "this machine",
  4: "off-box · external",
};

export function renderBands(host: HTMLElement, m: ReachModel, live: Map<string, number>, ctx: ReachViewCtx): void {
  const wrap = elh("div", "rv-bands");
  const nodes = revealedNodes(m, live);
  for (let ring = 1; ring <= 4; ring++) {
    const inBand = nodes.filter((n) => ctx.ring[n.kind] === ring);
    if (!inBand.length) continue;
    const lane = elh("div", "rv-band" + (ring === 4 ? " rv-band--off" : ""));
    const label = elh("div", "rv-band-label");
    label.textContent = `${BAND_LABEL[ring]} · ${inBand.length}`;
    const chips = elh("div", "rv-band-chips");
    for (const n of inBand) {
      const chip = elh("div", "rv-chip" + (n.sensitive ? " rv-chip--alarm" : ""));
      chip.title = n.detail;
      const dot = elh("span", "rv-dot");
      dot.style.background = ctx.color[n.kind];
      const lab = elh("span");
      lab.textContent = ctx.trunc(n.label, 30);
      const cnt = elh("span", "rv-cnt");
      cnt.textContent = "×" + (live.get(n.id) ?? 0);
      chip.append(dot, lab, cnt);
      chips.append(chip);
    }
    lane.append(label, chips);
    wrap.append(lane);
  }
  host.append(wrap);
}
