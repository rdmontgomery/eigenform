/**
 * reachviews.ts — alternative renderings of the reach model, for A/B comparison.
 *
 * The docked reach map is a short, wide strip, which the radial spiderweb
 * (reachmap.ts) fills awkwardly — its labels are SVG-unit text that scales down
 * to a few pixels. These views render as plain HTML instead, so text stays crisp
 * at any size, and each leans into a different question:
 *
 *   bands    — distance-from-home lanes (workspace → off-box): the security scan.
 *   dial     — radial "blast radius": angle = kind, radius = distance-from-home.
 *   treemap  — squarified area ∝ activity, colour = kind: what got hit, how much.
 *   timeline — kind lanes across turns: when reach expanded, and how far.
 *
 * Each renderer clears nothing and appends into `host`; the caller owns teardown.
 * `live` is the per-node event count revealed at the current scrubber cursor, so
 * these animate under the same transport as the spiderweb.
 */

import type { ReachModel, ReachNode, ReachKind } from "./reach.ts";

export interface ReachViewCtx {
  color: Record<ReachKind, string>;
  kindLabel: Record<ReachKind, string>;
  ring: Record<ReachKind, number>;
  ringFrac: Record<number, number>;
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

function chipDot(color: string): HTMLElement {
  const dot = elh("span", "rv-dot");
  dot.style.background = color;
  return dot;
}

// ── Bands: distance-from-home lanes ─────────────────────────────────────────
const BAND_LABEL: Record<number, string> = {
  1: "workspace",
  2: "repo · shell · agents",
  3: "on-disk · MCP · secrets",
  4: "off-box · network",
};

export function renderBands(host: HTMLElement, m: ReachModel, live: Map<string, number>, ctx: ReachViewCtx): void {
  const wrap = elh("div", "rv-bands");
  for (let ring = 1; ring <= 4; ring++) {
    const nodes = revealedNodes(m, live).filter((n) => ctx.ring[n.kind] === ring);
    if (!nodes.length) continue;
    const lane = elh("div", "rv-band" + (ring === 4 ? " rv-band--off" : ""));
    const label = elh("div", "rv-band-label");
    label.textContent = `${BAND_LABEL[ring]} · ${nodes.length}`;
    const chips = elh("div", "rv-band-chips");
    for (const n of nodes) {
      const chip = elh("div", "rv-chip" + (n.sensitive ? " rv-chip--alarm" : ""));
      chip.title = n.detail;
      const lab = elh("span");
      lab.textContent = ctx.trunc(n.label, 30);
      const cnt = elh("span", "rv-cnt");
      cnt.textContent = "×" + (live.get(n.id) ?? 0);
      chip.append(chipDot(ctx.color[n.kind]), lab, cnt);
      chips.append(chip);
    }
    lane.append(label, chips);
    wrap.append(lane);
  }
  host.append(wrap);
}

// ── Treemap: squarified area ∝ activity ─────────────────────────────────────
interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap over normalized weights → a rect per weight in [0,1]².
 * Weights need not sum to 1; they're normalized here. Order matches input.
 */
export function layoutTreemap(weights: number[]): Rect[] {
  const n = weights.length;
  const rects: Rect[] = new Array(n);
  if (n === 0) return rects;
  const total = weights.reduce((a, b) => a + b, 0) || 1;
  const items = weights.map((w, idx) => ({ idx, area: w / total }));

  let box: Rect = { x: 0, y: 0, w: 1, h: 1 };

  function layoutRow(row: typeof items, horizontal: boolean): void {
    const rowArea = row.reduce((a, r) => a + r.area, 0);
    if (horizontal) {
      const rowH = rowArea / box.w;
      let x = box.x;
      for (const r of row) {
        const w = r.area / (rowH || 1);
        rects[r.idx] = { x, y: box.y, w, h: rowH };
        x += w;
      }
      box = { x: box.x, y: box.y + rowH, w: box.w, h: box.h - rowH };
    } else {
      const rowW = rowArea / box.h;
      let y = box.y;
      for (const r of row) {
        const h = r.area / (rowW || 1);
        rects[r.idx] = { x: box.x, y, w: rowW, h };
        y += h;
      }
      box = { x: box.x + rowW, y: box.y, w: box.w - rowW, h: box.h };
    }
  }

  function worst(row: typeof items, side: number): number {
    const s = row.reduce((a, r) => a + r.area, 0);
    if (s === 0) return Infinity;
    const mx = Math.max(...row.map((r) => r.area));
    const mn = Math.min(...row.map((r) => r.area));
    return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
  }

  const remaining = items.slice();
  let row: typeof items = [];
  while (remaining.length) {
    const horizontal = box.w >= box.h;
    const side = horizontal ? box.w : box.h;
    const next = remaining[0];
    if (next === undefined) break;
    if (row.length === 0) {
      row.push(next);
      remaining.shift();
      continue;
    }
    if (worst(row, side) >= worst([...row, next], side)) {
      row.push(next);
      remaining.shift();
    } else {
      layoutRow(row, horizontal);
      row = [];
    }
  }
  if (row.length) layoutRow(row, box.w >= box.h);
  return rects;
}

export function renderTreemap(host: HTMLElement, m: ReachModel, live: Map<string, number>, ctx: ReachViewCtx): void {
  const nodes = revealedNodes(m, live);
  const wrap = elh("div", "rv-treemap");
  const rects = layoutTreemap(nodes.map((n) => live.get(n.id) ?? 1));
  nodes.forEach((n, i) => {
    const r = rects[i];
    if (!r) return;
    const cell = elh("div", "rv-cell" + (n.sensitive ? " rv-cell--alarm" : ""));
    cell.style.left = `${r.x * 100}%`;
    cell.style.top = `${r.y * 100}%`;
    cell.style.width = `${r.w * 100}%`;
    cell.style.height = `${r.h * 100}%`;
    cell.style.setProperty("--c", ctx.color[n.kind]);
    cell.title = n.detail;
    const lab = elh("span", "rv-cell-lab");
    lab.textContent = n.label;
    const cnt = elh("span", "rv-cell-cnt");
    cnt.textContent = "×" + (live.get(n.id) ?? 0);
    cell.append(lab, cnt);
    wrap.append(cell);
  });
  host.append(wrap);
}

// ── Timeline: kind lanes across turns ───────────────────────────────────────
export function renderTimeline(host: HTMLElement, m: ReachModel, live: Map<string, number>, ctx: ReachViewCtx): void {
  const nodes = revealedNodes(m, live);
  const wrap = elh("div", "rv-timeline");
  if (!nodes.length) {
    host.append(wrap);
    return;
  }
  const minT = Math.min(...nodes.map((n) => n.firstTurn));
  const maxT = Math.max(...nodes.map((n) => n.lastTurn), minT + 1);
  const span = Math.max(1, maxT - minT);

  // Lanes in ring order (home → off-box), then by kind, for present kinds only.
  const kinds: ReachKind[] = [];
  for (const n of nodes) if (!kinds.includes(n.kind)) kinds.push(n.kind);
  kinds.sort((a, b) => ctx.ring[a] - ctx.ring[b]);

  for (const k of kinds) {
    const lane = elh("div", "rv-tl-lane");
    const label = elh("div", "rv-tl-label");
    label.textContent = ctx.kindLabel[k];
    const track = elh("div", "rv-tl-track");
    for (const n of nodes.filter((x) => x.kind === k)) {
      const bar = elh("div", "rv-tl-bar" + (n.sensitive ? " rv-tl-bar--alarm" : ""));
      const left = ((n.firstTurn - minT) / span) * 100;
      const width = Math.max(3, ((n.lastTurn - n.firstTurn) / span) * 100);
      bar.style.left = `${left}%`;
      bar.style.width = `${width}%`;
      bar.style.setProperty("--c", ctx.color[k]);
      bar.title = `${n.detail} · turn ${n.firstTurn}–${n.lastTurn}`;
      const t = elh("span", "rv-tl-bar-lab");
      t.textContent = ctx.trunc(n.label, 22);
      bar.append(t);
      track.append(bar);
    }
    lane.append(label, track);
    wrap.append(lane);
  }
  const axis = elh("div", "rv-tl-axis");
  axis.textContent = `turn ${minT} → ${maxT}`;
  wrap.append(axis);
  host.append(wrap);
}

// ── Dial: radial blast radius (angle = kind, radius = distance-from-home) ────
export function renderDial(host: HTMLElement, m: ReachModel, live: Map<string, number>, ctx: ReachViewCtx): void {
  const wrap = elh("div", "rv-dial");
  host.append(wrap);
  const w = host.clientWidth || 320;
  const h = host.clientHeight || 200;
  const cx = w / 2;
  const cy = h / 2;
  const R = Math.max(24, Math.min(w, h) / 2 - 26);

  // Faint distance rings.
  for (let ring = 1; ring <= 4; ring++) {
    const rr = R * (ctx.ringFrac[ring] ?? 1);
    const g = elh("div", "rv-dial-ring" + (ring === 4 ? " rv-dial-ring--off" : ""));
    g.style.left = `${cx - rr}px`;
    g.style.top = `${cy - rr}px`;
    g.style.width = `${rr * 2}px`;
    g.style.height = `${rr * 2}px`;
    wrap.append(g);
  }

  const hub = elh("div", "rv-dial-hub");
  hub.style.left = `${cx}px`;
  hub.style.top = `${cy}px`;
  hub.textContent = ctx.trunc(m.rootLabel, 16);
  wrap.append(hub);

  const nodes = revealedNodes(m, live);
  const kinds: ReachKind[] = [];
  for (const n of nodes) if (!kinds.includes(n.kind)) kinds.push(n.kind);
  kinds.sort((a, b) => ctx.ring[a] - ctx.ring[b]);

  kinds.forEach((k, ki) => {
    const base = (ki + 0.5) / Math.max(1, kinds.length) * Math.PI * 2 - Math.PI / 2;
    const group = nodes.filter((n) => n.kind === k);
    group.forEach((n, j) => {
      const spread = (j - (group.length - 1) / 2) * 0.2;
      const ang = base + spread;
      const r = R * (ctx.ringFrac[ctx.ring[n.kind]] ?? 1);
      const x = cx + r * Math.cos(ang);
      const y = cy + r * Math.sin(ang);
      const chip = elh("div", "rv-dial-node" + (n.sensitive ? " rv-dial-node--alarm" : ""));
      chip.style.left = `${x}px`;
      chip.style.top = `${y}px`;
      chip.title = n.detail;
      chip.append(chipDot(ctx.color[n.kind]));
      const lab = elh("span");
      lab.textContent = ctx.trunc(n.label, 18);
      chip.append(lab);
      wrap.append(chip);
    });
  });
}
