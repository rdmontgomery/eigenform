/**
 * reachmap.ts — The "reach map" overlay: a time-evolution spiderweb of where
 * the agent's tool calls reached.
 *
 * It consumes the same `/api/session/:uuid/json` exchanges as the drawer, runs
 * them through reach.ts, and draws a radial node-link graph: the session root is
 * the hub, and every touched target hangs off it on a ring keyed by *distance
 * from home* — workspace subdirs inner, sibling repos next, then local-elsewhere
 * / MCP servers, then off-machine web + comms outermost. A time scrubber (and a
 * play button) reveals nodes in session order, so you can watch the reach grow.
 *
 * Secret-manager and comms (Slack/email/…) MCP surfaces render in alarm red; a
 * secret-read followed by an egress draws a dashed "exfil" arc between them with
 * a banner — the spider-web-reaching-out-of-the-org shape made literal.
 *
 * It is mounted as an absolute overlay on .term-host (the terminal is not
 * reflowed) and follows the active tab, mirroring the drawer's lifecycle.
 *
 * XSS safety: all labels/details are set via textContent, never innerHTML; the
 * only innerHTML is icons.ts's static path data.
 */

import { buildReach } from "./reach.ts";
import type { ReachModel, ReachNode, ReachKind } from "./reach.ts";
import type { Exchange } from "./turns.ts";
import { icon } from "./icons.ts";

const SVG_NS = "http://www.w3.org/2000/svg";

// ---------------------------------------------------------------------------
// Geometry + palette
// ---------------------------------------------------------------------------

const VIEW = 1000;
const CX = VIEW / 2;
const CY = VIEW / 2;
const RAD = 430;

/** Distance-from-home ring per kind (1 = inside the workspace, 4 = off-box). */
const RING: Record<ReachKind, number> = {
  workspace: 1,
  repo: 2,
  agent: 2,
  shell: 2,
  external: 3,
  mcp: 3,
  secret: 3,
  web: 4,
  comms: 4,
};
const RING_FRAC: Record<number, number> = { 1: 0.3, 2: 0.52, 3: 0.74, 4: 0.93 };

/** Kind → CSS color token. Secret/comms are alarm red (the exfil surfaces). */
const COLOR: Record<ReachKind, string> = {
  workspace: "var(--ink-teal)",
  repo: "var(--ink-olive)",
  external: "var(--ink-slate)",
  web: "var(--ink-ochre)",
  mcp: "var(--ink-plum)",
  agent: "var(--ink-clay)",
  shell: "var(--tx-3)",
  secret: "var(--st-error)",
  comms: "var(--st-error)",
};

const KIND_LABEL: Record<ReachKind, string> = {
  workspace: "workspace",
  repo: "sibling repo",
  external: "elsewhere on disk",
  web: "web host",
  mcp: "MCP server",
  agent: "subagent / skill",
  shell: "shell",
  secret: "secret manager",
  comms: "comms / egress",
};

interface Pos {
  x: number;
  y: number;
}

// ---------------------------------------------------------------------------
// SVG helpers
// ---------------------------------------------------------------------------

function svg<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | number> = {},
): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}

function trunc(s: string, n: number): string {
  return s.length <= n ? s : s.slice(0, n - 1) + "…";
}

function nodeRadius(count: number): number {
  return 9 + Math.min(26, Math.sqrt(count) * 6);
}

// ---------------------------------------------------------------------------
// Session fetch (mirror drawer.ts — independent of web/)
// ---------------------------------------------------------------------------

interface SessionPayload {
  exchanges: Exchange[];
}

async function fetchSession(uuid: string): Promise<SessionPayload | null> {
  try {
    const res = await fetch(`/api/session/${encodeURIComponent(uuid)}/json`);
    if (!res.ok) return null;
    return (await res.json()) as SessionPayload;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface ReachHandle {
  close(): void;
}

export interface ReachOptions {
  /** Session working directory — anchors the workspace vs sibling vs external rings. */
  root?: string;
  /** Called when the overlay dismisses itself (✕ / backdrop / Esc). */
  onClose?: () => void;
}

/**
 * Mount the reach-map overlay for `uuid` into `hostEl` (.term-host).
 * Caller must call .close() on tab-close or toggle-off.
 */
export function mountReachMap(hostEl: HTMLElement, uuid: string, opts: ReachOptions = {}): ReachHandle {
  let closed = false;

  // ── Structure: backdrop + panel ──────────────────────────────────────────
  const root = el("div", "reachmap");

  const head = el("div", "reachmap-head");
  const titleWrap = el("div", "reachmap-titlewrap");
  const title = el("span", "reachmap-title");
  title.textContent = "Reach";
  const summary = el("span", "reachmap-summary");
  titleWrap.append(title, summary);

  const closeBtn = el("button", "reachmap-close");
  closeBtn.title = "Close (Esc)";
  closeBtn.append(icon("x", 15));
  head.append(titleWrap, closeBtn);

  // The scene: SVG graph + a flagged-exfil banner.
  const scene = el("div", "reachmap-scene");
  const canvas = svg("svg", { viewBox: `0 0 ${VIEW} ${VIEW}`, class: "reachmap-svg" });
  canvas.setAttribute("preserveAspectRatio", "xMidYMid meet");
  const gEdges = svg("g");
  const gExfil = svg("g");
  const gNodes = svg("g");
  const gHub = svg("g");
  canvas.append(gEdges, gExfil, gNodes, gHub);
  const banner = el("div", "reachmap-banner");
  const tooltip = el("div", "reachmap-tooltip");
  const empty = el("div", "reachmap-empty");
  empty.textContent = "no reach yet — the agent hasn't used any tools";
  scene.append(canvas, banner, tooltip, empty);

  // Controls: legend + scrubber + play.
  const controls = el("div", "reachmap-controls");
  const legend = el("div", "reachmap-legend");
  const transport = el("div", "reachmap-transport");
  const playBtn = el("button", "reachmap-play");
  const scrub = el("input", "reachmap-scrub");
  scrub.type = "range";
  scrub.min = "0";
  const turnReadout = el("span", "reachmap-turn");
  transport.append(playBtn, scrub, turnReadout);
  controls.append(legend, transport);

  root.append(head, scene, controls);
  hostEl.append(root);

  // ── State ────────────────────────────────────────────────────────────────
  let model: ReachModel | null = null;
  let positions = new Map<string, Pos>();
  let cursor = 0; // number of events revealed
  let playing = false;
  let timer: number | null = null;

  // ── Layout (computed once per model) ──────────────────────────────────────
  function layout(m: ReachModel) {
    positions = new Map();
    const byRing = new Map<number, ReachNode[]>();
    for (const n of m.nodes) {
      const d = RING[n.kind];
      const list = byRing.get(d) ?? [];
      list.push(n);
      byRing.set(d, list);
    }
    for (const [d, list] of byRing) {
      list.sort((a, b) => a.firstSeq - b.firstSeq);
      const r = (RING_FRAC[d] ?? 0.93) * RAD;
      const phase = d * 0.7; // per-ring twist so spokes don't stack
      const n = list.length;
      list.forEach((node, i) => {
        const a = phase + (i / Math.max(1, n)) * Math.PI * 2;
        positions.set(node.id, { x: CX + r * Math.cos(a), y: CY + r * Math.sin(a) });
      });
    }
  }

  // ── Legend (only kinds present) ───────────────────────────────────────────
  function renderLegend(m: ReachModel) {
    legend.innerHTML = "";
    const present: ReachKind[] = [];
    for (const n of m.nodes) if (!present.includes(n.kind)) present.push(n.kind);
    const order: ReachKind[] = ["workspace", "repo", "external", "shell", "agent", "mcp", "web", "secret", "comms"];
    for (const k of order) {
      if (!present.includes(k)) continue;
      const item = el("span", "reachmap-legitem");
      const dot = el("span", "reachmap-legdot");
      dot.style.background = COLOR[k];
      const lab = el("span");
      lab.textContent = KIND_LABEL[k];
      item.append(dot, lab);
      legend.append(item);
    }
  }

  // ── Draw the hub (once per model) ─────────────────────────────────────────
  function drawHub(m: ReachModel) {
    gHub.innerHTML = "";
    gHub.append(svg("circle", { cx: CX, cy: CY, r: 26, class: "reachmap-hub-disc" }));
    const t = svg("text", { x: CX, y: CY + 44, class: "reachmap-hub-label" });
    t.textContent = trunc(m.rootLabel, 22);
    gHub.append(t);
  }

  // ── Tooltip ───────────────────────────────────────────────────────────────
  function showTip(node: ReachNode, liveCount: number) {
    tooltip.innerHTML = "";
    const h = el("div", "reachmap-tip-head");
    h.textContent = node.label;
    const meta = el("div", "reachmap-tip-meta");
    meta.textContent = `${KIND_LABEL[node.kind]} · ${node.actions.join(", ")} · ${liveCount}× · turn ${node.firstTurn}–${node.lastTurn}`;
    const det = el("div", "reachmap-tip-detail");
    det.textContent = node.detail;
    tooltip.append(h, meta, det);
    tooltip.classList.add("reachmap-tooltip--on");
  }
  function hideTip() {
    tooltip.classList.remove("reachmap-tooltip--on");
  }

  // ── Draw the graph at a given cursor ──────────────────────────────────────
  function draw() {
    if (!model) return;
    const m = model;
    gEdges.innerHTML = "";
    gNodes.innerHTML = "";
    gExfil.innerHTML = "";

    // events[0..cursor) are revealed; tally per-node "live" counts.
    const revealed = m.events.slice(0, cursor);
    const live = new Map<string, number>();
    for (const e of revealed) live.set(e.node, (live.get(e.node) ?? 0) + 1);
    const activeNode = cursor > 0 ? m.events[cursor - 1]?.node : undefined;

    for (const node of m.nodes) {
      const c = live.get(node.id);
      if (!c) continue; // not reached yet at this cursor
      const p = positions.get(node.id);
      if (!p) continue;
      const isActive = node.id === activeNode;

      // edge: dashed when the node is an egress surface (leaving the box).
      const egress = node.kind === "web" || node.kind === "comms" || node.actions.includes("network") || node.actions.includes("write");
      const edge = svg("line", {
        x1: CX,
        y1: CY,
        x2: p.x,
        y2: p.y,
        stroke: COLOR[node.kind],
        "stroke-width": 1 + Math.min(4, c * 0.6),
        "stroke-opacity": isActive ? 0.9 : 0.4,
      });
      if (egress) edge.setAttribute("stroke-dasharray", "6 5");
      gEdges.append(edge);

      // node disc
      const g = svg("g", { class: "reachmap-node" + (isActive ? " reachmap-node--active" : "") });
      const disc = svg("circle", {
        cx: p.x,
        cy: p.y,
        r: nodeRadius(c),
        fill: COLOR[node.kind],
        "fill-opacity": 0.85,
      });
      if (node.sensitive) {
        disc.setAttribute("stroke", "var(--st-error)");
        disc.setAttribute("stroke-width", "3");
        g.classList.add("reachmap-node--sensitive");
      }
      g.append(disc);

      const label = svg("text", {
        x: p.x,
        y: p.y - nodeRadius(c) - 6,
        class: "reachmap-node-label",
        "text-anchor": "middle",
      });
      label.textContent = trunc(node.label, 20);
      g.append(label);

      g.addEventListener("mouseenter", () => showTip(node, c));
      g.addEventListener("mouseleave", hideTip);
      gNodes.append(g);
    }

    drawExfil(m, live);
    updateReadout(m, revealed.length);
  }

  // ── Exfil arc + banner ────────────────────────────────────────────────────
  function drawExfil(m: ReachModel, live: Map<string, number>) {
    banner.classList.remove("reachmap-banner--on");
    if (!m.exfil) return;
    // Only once both ends are revealed.
    if (!live.get(m.exfil.from) || !live.get(m.exfil.to)) return;
    const a = positions.get(m.exfil.from);
    const b = positions.get(m.exfil.to);
    if (a && b) {
      const mx = (a.x + b.x) / 2;
      const my = (a.y + b.y) / 2;
      // bow the arc outward from the hub for legibility
      const ox = mx + (mx - CX) * 0.25;
      const oy = my + (my - CY) * 0.25;
      const path = svg("path", {
        d: `M ${a.x} ${a.y} Q ${ox} ${oy} ${b.x} ${b.y}`,
        class: "reachmap-exfil-arc",
      });
      gExfil.append(path);
    }
    banner.textContent = `⚠ possible exfil — secret read (turn ${m.exfil.fromTurn}) then egress (turn ${m.exfil.toTurn})`;
    banner.classList.add("reachmap-banner--on");
  }

  function updateReadout(m: ReachModel, shown: number) {
    const activeTurn = cursor > 0 ? (m.events[cursor - 1]?.turn ?? 0) : 0;
    const reached = new Set(m.events.slice(0, cursor).map((e) => e.node)).size;
    turnReadout.textContent = cursor === 0 ? "start" : `turn ${activeTurn} · ${shown}/${m.events.length} calls · ${reached} reached`;
  }

  // ── Transport ─────────────────────────────────────────────────────────────
  function setCursor(c: number) {
    if (!model) return;
    cursor = Math.max(0, Math.min(model.events.length, c));
    scrub.value = String(cursor);
    draw();
    if (cursor >= model.events.length) stop();
  }

  function tick() {
    if (!model) return;
    if (cursor >= model.events.length) {
      stop();
      return;
    }
    setCursor(cursor + 1);
  }

  function play() {
    if (!model || model.events.length === 0) return;
    if (cursor >= model.events.length) cursor = 0; // replay from the top
    playing = true;
    renderPlayBtn();
    timer = window.setInterval(tick, 650);
  }
  function stop() {
    playing = false;
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
    renderPlayBtn();
  }
  function renderPlayBtn() {
    playBtn.innerHTML = "";
    playBtn.append(icon(playing ? "stop" : "play", 13));
    playBtn.title = playing ? "Pause" : "Play reach evolution";
  }

  playBtn.addEventListener("click", () => (playing ? stop() : play()));
  scrub.addEventListener("input", () => {
    stop();
    setCursor(Number(scrub.value));
  });

  // ── Data load + apply ─────────────────────────────────────────────────────
  function apply(m: ReachModel) {
    model = m;
    layout(m);
    drawHub(m);
    renderLegend(m);
    summary.textContent =
      m.nodes.length === 0
        ? "no tool activity"
        : `${m.events.length} tool ${m.events.length === 1 ? "call" : "calls"} · ${m.nodes.length} ${m.nodes.length === 1 ? "target" : "targets"}`;
    scrub.max = String(m.events.length);
    empty.style.display = m.nodes.length === 0 ? "" : "none";
    // Default: show the whole map; scrubbing rewinds time.
    cursor = m.events.length;
    scrub.value = String(cursor);
    draw();
  }

  async function load() {
    if (closed) return;
    const session = await fetchSession(uuid);
    if (closed || !session) return;
    // apply() parks the cursor at the live edge so new tool calls appear as the
    // session runs — the reach-map analogue of the drawer's stick-to-bottom.
    apply(buildReach(session.exchanges, opts.root ? { root: opts.root } : {}));
  }

  void load();

  // ── SSE live updates ──────────────────────────────────────────────────────
  const es = new EventSource(`/api/watch/${encodeURIComponent(uuid)}`);
  es.onmessage = () => void load();
  es.addEventListener("change", () => void load());
  es.onerror = () => {
    /* EventSource auto-reconnects. */
  };

  // ── Dismiss affordances ───────────────────────────────────────────────────
  function dismiss() {
    opts.onClose?.();
  }
  closeBtn.addEventListener("click", dismiss);
  root.addEventListener("mousedown", (e) => {
    if (e.target === root) dismiss(); // backdrop click
  });
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      dismiss();
    }
  }
  document.addEventListener("keydown", onKey);

  renderPlayBtn();

  // ── Teardown ──────────────────────────────────────────────────────────────
  return {
    close() {
      if (closed) return;
      closed = true;
      stop();
      es.close();
      document.removeEventListener("keydown", onKey);
      root.remove();
    },
  };
}
