// The session graph panel at the top of the inspect dock: the live sessions and
// their immediate kin, as a lineage outline (who launched whom) or a message ring
// (who messaged whom). Data: GET /api/graph (see crates/daemon/src/graph.rs).
//
// Nodes are HTML links to /s/<uuid>, so a click takes the app's own session-link
// path (shell.ts intercepts them) and never navigates the page. Edges are SVG
// drawn underneath. Text goes in via textContent only.

import { el } from "./dom.ts";
import { icon } from "./icons.ts";
import {
  arcPath,
  edgeRecency,
  edgeWidth,
  lineageOutline,
  ringLayout,
  type GraphNode,
  type SessionGraph,
} from "./graph-layout.ts";

type View = "lineage" | "messages";

const LS_VIEW = "eigenform:term:graph-view:v1";
const LS_COLLAPSED = "eigenform:term:graph-collapsed:v1";
const SVG_NS = "http://www.w3.org/2000/svg";
const ROW_H = 24;
const INDENT = 16;
const RING_PAD = 64;

export interface SessionGraphHandle {
  /** Re-fetch the graph (the forest changed). */
  refresh(): void;
  /** Mark the active tab's session. */
  setActive(uuid: string | null): void;
  close(): void;
}

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

function readLS(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeLS(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Private mode: the choice just won't survive a reload.
  }
}

function tooltip(n: GraphNode): string {
  if (n.ghost) return `${n.label}\nexited before eigenform saw it; known only by its peer name`;
  return [n.label, n.title && n.title !== n.label ? n.title : null, n.cwd, n.live ? n.state : "exited"]
    .filter(Boolean)
    .join("\n");
}

export function mountSessionGraph(hostEl: HTMLElement): SessionGraphHandle {
  const root = el("div", "sgraph");
  const head = el("div", "sgraph-head");
  head.tabIndex = 0;
  head.setAttribute("role", "button");
  const caret = el("span", "sgraph-caret");
  caret.append(icon("chevron", 13));
  const title = el("span", "sgraph-title");
  title.textContent = "Sessions";
  const summary = el("span", "sgraph-summary");
  const toggle = el("div", "sgraph-views");
  const viewBtn = (v: View, label: string) => {
    const b = el("button", "sgraph-view");
    b.textContent = label;
    b.dataset.view = v;
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      setView(v);
    });
    return b;
  };
  toggle.append(viewBtn("lineage", "Lineage"), viewBtn("messages", "Messages"));
  head.append(caret, title, summary, toggle);
  const body = el("div", "sgraph-body scroll");
  root.append(head, body);
  hostEl.append(root);

  let view: View = readLS(LS_VIEW) === "messages" ? "messages" : "lineage";
  let collapsed = readLS(LS_COLLAPSED) === "1";
  let graph: SessionGraph | null = null;
  let active: string | null = null;
  let closed = false;

  function setView(v: View) {
    view = v;
    writeLS(LS_VIEW, v);
    render();
  }

  function setCollapsed(c: boolean) {
    collapsed = c;
    writeLS(LS_COLLAPSED, c ? "1" : "0");
    root.classList.toggle("sgraph--collapsed", c);
    head.setAttribute("aria-expanded", String(!c));
    head.title = c ? "Show session graph" : "Hide session graph";
    if (!c) render();
  }
  head.addEventListener("click", () => setCollapsed(!collapsed));
  head.addEventListener("keydown", (e) => {
    if (e.target === head && (e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      setCollapsed(!collapsed);
    }
  });

  function nodeEl(n: GraphNode): HTMLElement {
    const cls = [
      "sg-node",
      n.live ? `sg-node--${n.state ?? "live"}` : "sg-node--dead",
      n.ghost ? "sg-node--ghost" : "",
      n.id === active ? "sg-node--active" : "",
    ]
      .filter(Boolean)
      .join(" ");
    const a = n.ghost ? el("span", cls) : el("a", cls);
    if (a instanceof HTMLAnchorElement) a.href = `/s/${encodeURIComponent(n.id)}`;
    a.title = tooltip(n);
    a.dataset.id = n.id;
    const dot = el("span", "sg-dot");
    const label = el("span", "sg-label");
    label.textContent = n.label;
    a.append(dot, label);
    return a;
  }

  function empty(text: string): HTMLElement {
    const e = el("div", "sgraph-empty");
    e.textContent = text;
    return e;
  }

  function renderLineage(g: SessionGraph) {
    const rows = lineageOutline(g);
    const wrap = el("div", "sg-outline");
    wrap.style.height = `${rows.length * ROW_H}px`;
    const lines = svg("svg", { class: "sg-edges", width: "100%", height: rows.length * ROW_H });
    const dotX = (depth: number) => 14 + depth * INDENT;
    const midY = (row: number) => row * ROW_H + ROW_H / 2;
    rows.forEach((r, i) => {
      const n = nodeEl(r.node);
      n.classList.add("sg-node--row");
      n.style.top = `${i * ROW_H}px`;
      n.style.left = `${dotX(r.depth) - 6}px`;
      if (r.lonelyStart) {
        const rule = el("div", "sg-divider");
        rule.style.top = `${i * ROW_H}px`;
        rule.textContent = "no recorded parent";
        wrap.append(rule);
        n.classList.add("sg-node--after-divider");
      }
      wrap.append(n);
      if (r.parentRow !== null) {
        const px = dotX(rows[r.parentRow]!.depth);
        lines.append(
          svg("path", {
            d: `M${px},${midY(r.parentRow) + 5} V${midY(i)} H${dotX(r.depth) - 6}`,
            class: "sg-lineage-edge",
          }),
        );
      }
    });
    wrap.prepend(lines);
    if (g.lineage.length === 0) {
      body.append(empty("No lineage recorded yet. Sessions launched with POST /api/pty and a \"parent\" show here."));
    }
    body.append(wrap);
  }

  function renderMessages(g: SessionGraph) {
    if (g.messages.length === 0) {
      body.append(empty("No peer messages between these sessions in the last 72 hours."));
    }
    const w = Math.max(200, body.clientWidth);
    const h = Math.max(160, body.clientHeight - (g.messages.length === 0 ? 40 : 0));
    // Ring order follows the lineage outline, so families sit together.
    const order = lineageOutline(g).map((r) => r.node.id);
    const pos = ringLayout(order, w, h, RING_PAD);
    const scene = el("div", "sg-ring");
    scene.style.height = `${h}px`;
    const edges = svg("svg", { class: "sg-edges", width: w, height: h, viewBox: `0 0 ${w} ${h}` });
    const defs = svg("defs", {});
    const marker = svg("marker", {
      id: "sg-arrow",
      viewBox: "0 0 8 8",
      refX: 7,
      refY: 4,
      markerWidth: 6,
      markerHeight: 6,
      orient: "auto-start-reverse",
      markerUnits: "userSpaceOnUse",
    });
    marker.append(svg("path", { d: "M0,0 L8,4 L0,8 z", class: "sg-arrowhead" }));
    defs.append(marker);
    edges.append(defs);
    const now = Date.now();
    for (const m of g.messages) {
      const a = pos.get(m.from);
      const b = pos.get(m.to);
      if (!a || !b) continue;
      const path = svg("path", {
        d: arcPath(a, b, 18, 14),
        class: "sg-msg-edge",
        "stroke-width": edgeWidth(m.count),
        "marker-end": "url(#sg-arrow)",
      });
      path.style.opacity = String(edgeRecency(m.last, now));
      path.dataset.from = m.from;
      path.dataset.to = m.to;
      const tip = svg("title", {});
      const from = g.nodes.find((n) => n.id === m.from)?.label ?? m.from;
      const to = g.nodes.find((n) => n.id === m.to)?.label ?? m.to;
      tip.textContent = `${from} → ${to}: ${m.count} message${m.count === 1 ? "" : "s"}, last ${new Date(m.last).toLocaleString()}`;
      path.append(tip);
      edges.append(path);
    }
    scene.append(edges);
    for (const n of g.nodes) {
      const p = pos.get(n.id);
      if (!p) continue;
      const e = nodeEl(n);
      e.classList.add("sg-node--ring");
      e.style.left = `${p.x}px`;
      e.style.top = `${p.y}px`;
      e.addEventListener("mouseenter", () => {
        scene.classList.add("sg-ring--focus");
        for (const path of edges.querySelectorAll<SVGPathElement>(".sg-msg-edge")) {
          path.classList.toggle("sg-hot", path.dataset.from === n.id || path.dataset.to === n.id);
        }
      });
      e.addEventListener("mouseleave", () => scene.classList.remove("sg-ring--focus"));
      scene.append(e);
    }
    body.append(scene);
  }

  function render() {
    for (const b of toggle.querySelectorAll<HTMLButtonElement>(".sgraph-view")) {
      b.classList.toggle("sgraph-view--on", b.dataset.view === view);
    }
    if (!graph) {
      summary.textContent = "";
      return;
    }
    const live = graph.nodes.filter((n) => n.live).length;
    const edges = view === "lineage" ? graph.lineage.length : graph.messages.length;
    summary.textContent = `${live} live · ${edges} ${view === "lineage" ? "launch" : "link"}${edges === 1 ? "" : "s"}`;
    if (collapsed) return;
    const scroll = body.scrollTop;
    body.replaceChildren();
    body.classList.toggle("sgraph-body--ring", view === "messages");
    if (graph.nodes.length === 0) {
      body.append(empty("No live sessions."));
    } else if (view === "lineage") {
      renderLineage(graph);
    } else {
      renderMessages(graph);
    }
    body.scrollTop = scroll;
  }

  let inflight = false;
  let again = false;
  async function load() {
    if (inflight) {
      again = true;
      return;
    }
    inflight = true;
    try {
      const res = await fetch("/api/graph");
      if (res.ok && !closed) {
        graph = (await res.json()) as SessionGraph;
        render();
      }
    } catch {
      // Daemon restarting: the next forest push retries.
    } finally {
      inflight = false;
      if (again && !closed) {
        again = false;
        void load();
      }
    }
  }

  // The ring fills the body, so re-lay it out when the dock or split resizes.
  let lastW = 0;
  let lastH = 0;
  const ro = new ResizeObserver(() => {
    if (view !== "messages") return;
    const w = body.clientWidth;
    const h = body.clientHeight;
    if (Math.abs(w - lastW) < 2 && Math.abs(h - lastH) < 2) return;
    lastW = w;
    lastH = h;
    render();
  });
  ro.observe(body);

  setCollapsed(collapsed);
  render();
  void load();

  // A busy session pushes the forest several times a second; one graph fetch a
  // second keeps up without rescanning on every push.
  let pending: ReturnType<typeof setTimeout> | null = null;

  return {
    refresh() {
      if (pending || closed) return;
      pending = setTimeout(() => {
        pending = null;
        void load();
      }, 1000);
    },
    setActive(uuid) {
      if (uuid === active) return;
      active = uuid;
      render();
    },
    close() {
      closed = true;
      if (pending) clearTimeout(pending);
      ro.disconnect();
      root.remove();
    },
  };
}
