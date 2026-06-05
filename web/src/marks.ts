// marks.ts — the marks, as inline SVG so they scale with type and stay crisp.
// Role (filled human / hollow agent / dotted system), the leaf bud, the fork
// (drawing a distinction), the norgie, the Mind field, woland's contained flame,
// and the small Tufte glyphs for the Forest.
import { svg } from "./dom.ts";

export function roleMark(role: "user" | "assistant" | "system", color: string, size = 9): SVGElement {
  const s = size, r = s / 2 - 1;
  if (role === "user")
    return svg("svg", { width: s, height: s, viewBox: `0 0 ${s} ${s}` }, svg("circle", { cx: s / 2, cy: s / 2, r, fill: color }));
  if (role === "assistant")
    return svg("svg", { width: s, height: s, viewBox: `0 0 ${s} ${s}` }, svg("circle", { cx: s / 2, cy: s / 2, r, fill: "none", stroke: color, "stroke-width": 1.1 }));
  return svg("svg", { width: s, height: s, viewBox: `0 0 ${s} ${s}` }, svg("circle", { cx: s / 2, cy: s / 2, r: 1.2, fill: color }));
}

export function leafMark(color = "var(--amber)", size = 14): SVGElement {
  return svg("svg", { width: size, height: size, viewBox: "0 0 14 14", fill: "none" },
    svg("path", { d: "M7 13 V5", stroke: color, "stroke-width": 1.2, "stroke-linecap": "round" }),
    svg("path", { d: "M7 6 C7 3 9.4 2 11 2 C11 4.6 9.4 6 7 6 Z", fill: color, opacity: 0.9 }),
    svg("path", { d: "M7 8 C7 6 5.1 5.2 3.6 5.6 C3.9 7.6 5.3 8.3 7 8 Z", fill: color, opacity: 0.5 }));
}

export function forkMark(color = "currentColor", size = 14, sw = 1.2): SVGElement {
  return svg("svg", { width: size, height: size, viewBox: "0 0 14 14", fill: "none" },
    svg("path", { d: "M4 13 V8 C4 6 5 5 7 5", stroke: color, "stroke-width": sw, "stroke-linecap": "round" }),
    svg("path", { d: "M7 5 H11", stroke: color, "stroke-width": sw, "stroke-linecap": "round", opacity: 0.55 }),
    svg("circle", { cx: 4, cy: 13, r: 1.4, fill: color }),
    svg("circle", { cx: 11.2, cy: 5, r: 1.4, fill: color }));
}

export function mindGlyph(color = "currentColor", size = 16): SVGElement {
  return svg("svg", { width: size, height: size, viewBox: "0 0 16 16", fill: "none" },
    svg("circle", { cx: 8, cy: 8, r: 6.2, stroke: color, "stroke-width": 1.2 }),
    svg("circle", { cx: 8, cy: 8, r: 2.4, fill: color, opacity: 0.55 }),
    svg("path", { d: "M8 1.8 V5.2 M8 10.8 V14.2 M1.8 8 H5.2 M10.8 8 H14.2", stroke: color, "stroke-width": 1, opacity: 0.4 }));
}

export function wolandMark(size = 22, ink = "var(--ink)", accent = "var(--amber)"): SVGElement {
  return svg("svg", { width: size, height: size, viewBox: "0 0 24 24", fill: "none" },
    svg("rect", { x: 3.5, y: 3.5, width: 17, height: 17, rx: 2, stroke: ink, "stroke-width": 1.3 }),
    svg("path", { d: "M12 17.5 C9 17.5 8.2 14.4 9.6 12.2 C10 13.7 10.8 13.7 11.2 12.9 C11.2 10.6 12.7 9.8 12.7 8.3 C14.9 9.8 15.7 12 14.9 14.2 C15.3 13.9 15.7 13.5 15.7 12.7 C16.9 14.2 16.5 17.5 12 17.5 Z", fill: accent }));
}

// the norgie — the Furnace's resting live-status glyph; a small banked coal.
export function norgie(on = true, color = "var(--temp-color)", dim = "var(--faint)"): SVGElement {
  const c = on ? color : dim;
  return svg("svg", { width: 16, height: 16, viewBox: "0 0 16 16" },
    svg("circle", { cx: 8, cy: 8, r: 3.4, fill: c }),
    svg("circle", { cx: 8, cy: 8, r: 6.4, fill: "none", stroke: c, "stroke-width": 1, opacity: 0.35 }));
}

// a forking-path silhouette for a session in the Forest.
export function forestGlyph(shape: number[], branches: number, active: boolean, color: string, accent: string, w = 150, h = 22): SVGElement {
  const max = Math.max(...shape, 1);
  const n = shape.length;
  const pts = shape.map((v, i) => [(i / (n - 1)) * w, h - (v / max) * (h - 4) - 2] as const);
  const d = pts.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)}`).join(" ");
  const root = svg("svg", { width: w, height: h, viewBox: `0 0 ${w} ${h}`, fill: "none" },
    svg("path", { d, stroke: color, "stroke-width": active ? 1.3 : 1, opacity: active ? 1 : 0.7 }));
  for (let b = 0; b < branches; b++) {
    const i = Math.floor((n - 2) * ((b + 1) / (branches + 1))) + 1;
    const p = pts[i] ?? pts[pts.length - 1];
    if (!p) continue;
    root.appendChild(svg("path", { d: `M${p[0]} ${p[1]} l${5 + b} ${-6 - b * 2}`, stroke: accent, "stroke-width": 1, opacity: 0.8 }));
  }
  const last = pts[pts.length - 1];
  if (last) root.appendChild(svg("circle", { cx: last[0], cy: last[1], r: 1.8, fill: accent }));
  return root;
}
