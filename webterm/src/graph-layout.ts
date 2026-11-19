// Session-graph layouts, pure (no DOM): the lineage outline and the message ring.
// Both are deterministic, so a live update never shuffles nodes that didn't change.

export interface GraphNode {
  /** Session uuid, or `peer:<name>` for a sender known only by name. */
  id: string;
  label: string;
  title?: string;
  cwd?: string;
  live: boolean;
  state?: string;
  ghost: boolean;
}

export interface LineageEdge {
  from: string;
  to: string;
}

export interface MessageEdge {
  from: string;
  to: string;
  count: number;
  /** RFC 3339 time of the latest delivery. */
  last: string;
}

export interface SessionGraph {
  nodes: GraphNode[];
  lineage: LineageEdge[];
  messages: MessageEdge[];
}

/** One row of the lineage outline. */
export interface OutlineRow {
  node: GraphNode;
  depth: number;
  /** Row index of the parent, or null for a root. */
  parentRow: number | null;
  /** True for the first row of the "no recorded parent" group. */
  lonelyStart?: boolean;
}

/** Live before dead, then by label: the order siblings and loners list in. */
function byLiveThenLabel(a: GraphNode, b: GraphNode): number {
  if (a.live !== b.live) return a.live ? -1 : 1;
  return a.label.localeCompare(b.label) || a.id.localeCompare(b.id);
}

/**
 * The lineage tree as an indented outline: each tree depth-first with children
 * under their parent, trees first, then nodes with no lineage edge at all. A
 * node with two recorded parents (shouldn't happen) lists under the first one;
 * a cycle is broken where it is first met.
 */
export function lineageOutline(g: SessionGraph): OutlineRow[] {
  const byId = new Map(g.nodes.map((n) => [n.id, n]));
  const children = new Map<string, GraphNode[]>();
  const hasParent = new Set<string>();
  const linked = new Set<string>();
  for (const e of g.lineage) {
    const child = byId.get(e.to);
    if (!child || !byId.has(e.from) || hasParent.has(e.to)) continue;
    hasParent.add(e.to);
    linked.add(e.from).add(e.to);
    const list = children.get(e.from) ?? [];
    list.push(child);
    children.set(e.from, list);
  }

  const rows: OutlineRow[] = [];
  const seen = new Set<string>();
  const visit = (n: GraphNode, depth: number, parentRow: number | null) => {
    if (seen.has(n.id)) return;
    seen.add(n.id);
    const row = rows.length;
    rows.push({ node: n, depth, parentRow });
    for (const c of (children.get(n.id) ?? []).sort(byLiveThenLabel)) visit(c, depth + 1, row);
  };

  const roots = g.nodes.filter((n) => linked.has(n.id) && !hasParent.has(n.id)).sort(byLiveThenLabel);
  for (const r of roots) visit(r, 0, null);
  // Pure cycles have no root: start them at their first member.
  for (const n of [...g.nodes].sort(byLiveThenLabel)) if (linked.has(n.id)) visit(n, 0, null);

  const loners = g.nodes.filter((n) => !linked.has(n.id)).sort(byLiveThenLabel);
  loners.forEach((n, i) => {
    seen.add(n.id);
    rows.push({ node: n, depth: 0, parentRow: null, lonelyStart: i === 0 && rows.length > 0 });
  });
  return rows;
}

export interface Point {
  x: number;
  y: number;
}

/**
 * Nodes evenly on an ellipse inside a w×h box (inset by `pad`), starting at the
 * top and running clockwise, in the given order. A lone node sits in the centre.
 */
export function ringLayout(ids: string[], w: number, h: number, pad: number): Map<string, Point> {
  const out = new Map<string, Point>();
  const cx = w / 2;
  const cy = h / 2;
  if (ids.length === 1) {
    out.set(ids[0]!, { x: cx, y: cy });
    return out;
  }
  const rx = Math.max(0, w / 2 - pad);
  const ry = Math.max(0, h / 2 - pad / 2);
  ids.forEach((id, i) => {
    const a = -Math.PI / 2 + (2 * Math.PI * i) / ids.length;
    out.set(id, { x: cx + rx * Math.cos(a), y: cy + ry * Math.sin(a) });
  });
  return out;
}

/**
 * A curved path from `a` to `b` that bends to the left of its direction, so
 * A→B and B→A draw as two separate arcs. Stops `inset` short of each end so an
 * arrowhead doesn't sit under the node.
 */
export function arcPath(a: Point, b: Point, bend: number, inset: number): string {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  const ux = dx / len;
  const uy = dy / len;
  const s = { x: a.x + ux * inset, y: a.y + uy * inset };
  const e = { x: b.x - ux * inset, y: b.y - uy * inset };
  const c = { x: (a.x + b.x) / 2 + uy * bend, y: (a.y + b.y) / 2 - ux * bend };
  const r = (n: number) => Math.round(n * 10) / 10;
  return `M${r(s.x)},${r(s.y)} Q${r(c.x)},${r(c.y)} ${r(e.x)},${r(e.y)}`;
}

/** Edge stroke width: 1px for one message, growing with the log of the count. */
export function edgeWidth(count: number): number {
  return Math.min(5, 1 + Math.log2(Math.max(1, count)));
}

/** Edge opacity by how recently its last message arrived. */
export function edgeRecency(lastIso: string, nowMs: number): number {
  const t = Date.parse(lastIso);
  if (!Number.isFinite(t)) return 0.3;
  const mins = (nowMs - t) / 60_000;
  if (mins <= 15) return 1;
  if (mins <= 60) return 0.75;
  if (mins <= 24 * 60) return 0.5;
  return 0.3;
}
