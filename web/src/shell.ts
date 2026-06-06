// shell.ts — chrome around the Manuscript: the masthead (woland fronts, eigen the
// engine) with the light/dark toggle, the Forest as forking-path navigation, the
// Furnace fidelity channel (norgie collapsed ↔ the real pty expanded), and the two
// considered moments — the cold-fork confirm and the fork toast.
import { el, svg } from "./dom.ts";
import { wolandMark, norgie, forestGlyph } from "./marks.ts";
import { type ForestEntry, type Session } from "./data.ts";
import { type CacheReading, type ForkReading, fmtK, fmtClock } from "./cooling.ts";
import { type ThemeName } from "./theme.ts";
import { type Density } from "./prefs.ts";

export function buildMasthead(
  session: Session,
  theme: ThemeName,
  density: Density,
  onTheme: () => void,
  onDensity: () => void,
): { node: HTMLElement; setTheme(t: ThemeName): void; setDensity(d: Density): void; setSession(s: Session): void } {
  const swatch = el("span", { class: "swatch" });
  const label = el("span", { text: theme });
  const btn = el("button", { class: "ghost theme-toggle", title: "toggle paper / furnace", onclick: onTheme }, swatch, label);
  const densityBtn = el("button", { class: `ghost density-toggle${density === "compact" ? " on" : ""}`, title: "toggle compact density", onclick: onDensity }, "Aa");
  const sess = el("div", { class: "sess" });
  const setSession = (s: Session): void => {
    sess.replaceChildren("session ", el("b", { text: s.id }), ` · ${s.total} turns · ${s.branches}⑂ · viewing ${s.windowStart}–${s.total}`);
  };
  setSession(session);
  const node = el("div", { class: "masthead" },
    el("div", { class: "brand" }, wolandMark(22), el("div", { style: "display:flex;align-items:baseline;gap:9px" },
      el("span", { class: "name", text: "woland" }), el("span", { class: "engine", text: "eigen engine" }))),
    el("div", { class: "vrule" }),
    sess,
    el("div", { class: "spacer" }),
    el("div", { class: "live" }, el("span", { class: "dot" }), "LIVE"),
    densityBtn,
    btn);
  return {
    node,
    setTheme: (t) => { label.textContent = t; },
    setDensity: (d) => densityBtn.classList.toggle("on", d === "compact"),
    setSession,
  };
}

export function buildForest(
  onSelect: (entry: ForestEntry) => void,
  onNew: (cwd: string) => void,
): { node: HTMLElement; fill(entries: ForestEntry[]): void; setProjectDirs(dirs: string[]): void } {
  const list = el("div");

  // new-session picker: a ⊕ in the header reveals an inline directory input backed
  // by the /api/projects datalist; Enter launches `claude` there (?new=<cwd>).
  const dirs = el("datalist", { id: "project-dirs" });
  const input = el("input", { class: "newsess-input", list: "project-dirs", placeholder: "directory to launch from…" }) as HTMLInputElement;
  input.setAttribute("autocomplete", "off");
  const picker = el("div", { class: "newsess", style: "display:none" }, input, dirs);
  const close = (): void => { picker.style.display = "none"; input.value = ""; };
  const launch = (): void => { const cwd = input.value.trim(); if (cwd) { onNew(cwd); close(); } };
  input.addEventListener("keydown", (ev) => {
    const k = ev as KeyboardEvent;
    if (k.key === "Enter") { k.preventDefault(); launch(); }
    if (k.key === "Escape") { k.preventDefault(); close(); }
  });
  const plus = el("button", { class: "newsess-btn", title: "new session", onclick: () => {
    const open = picker.style.display === "none";
    picker.style.display = open ? "block" : "none";
    if (open) input.focus(); else input.value = "";
  } }, "⊕");

  const node = el("div", { class: "forest" },
    el("div", { class: "forest-head" },
      el("div", { class: "eyebrow", text: "Forest · forking paths" }), plus),
    picker, list);

  function fill(entries: ForestEntry[]): void {
    list.replaceChildren();
    if (!entries.length) { list.appendChild(el("div", { class: "eyebrow", style: "padding:0 20px", text: "no sessions" })); return; }
    for (const s of entries) {
      const row = el("div", { class: `row${s.active ? " active" : ""}`, dataset: { id: s.id }, onclick: () => onSelect(s) },
        el("span", { class: "nm", text: s.name }),
        el("div", { class: "glyphline" },
          forestGlyph(s.shape, s.branches, s.active, s.active ? "var(--agent)" : "var(--faint)", "var(--amber)", 150, 22),
          el("span", { class: "gmeta", text: `${s.turns}·${s.branches}⑂` })));
      list.appendChild(row);
    }
  }
  function setProjectDirs(ds: string[]): void {
    dirs.replaceChildren(...ds.map((d) => el("option", { value: d })));
  }
  return { node, fill, setProjectDirs };
}

export function buildFurnace(): { node: HTMLElement; termHost: HTMLElement; setOpen(open: boolean): void; onToggle(fn: () => void): void } {
  const termHost = el("div", { class: "term" });
  let toggleFn = (): void => {};

  // The whole collapsed bar opens the Furnace (not just the chevron), so it's always
  // reachable; the chevron is just an affordance (no own handler — it would double-toggle).
  const collapsed = el("div", { class: "collapsed-view", title: "open the Furnace", style: "display:flex;flex-direction:column;align-items:center;justify-content:space-between;height:100%;width:100%;cursor:pointer", onclick: () => toggleFn() },
    el("div", { class: "eyebrow vlabel", text: "Furnace" }),
    el("div", { style: "display:flex;flex-direction:column;align-items:center;gap:14px" },
      norgie(true), el("div", { class: "heatbar" }), el("span", { class: "parity-v", text: "parity ✓" })),
    el("span", { class: "collapse-btn", text: "‹" }));

  const expanded = el("div", { class: "expanded-view", style: "display:none;flex-direction:column;height:100%;width:100%" },
    el("div", { class: "fhead" },
      svg("svg", { width: 14, height: 14, viewBox: "0 0 16 16" },
        svg("circle", { cx: 8, cy: 8, r: 3.4, fill: "var(--amber)" }),
        svg("circle", { cx: 8, cy: 8, r: 6.4, fill: "none", stroke: "var(--amber)", "stroke-width": 1, opacity: 0.4 })),
      el("span", { class: "ftitle", text: "Furnace" }),
      el("span", { class: "fsub", text: "where the burning actually happens" }),
      el("span", { class: "spacer" }),
      el("span", { class: "parity", text: "parity ✓" }),
      el("button", { class: "expand-btn", onclick: () => toggleFn(), text: "›" })),
    termHost,
    el("div", { class: "ffoot", text: "the Manuscript re-renders this stream · check parity when the format drifts" }));

  const node = el("div", { class: "furnace collapsed" }, collapsed, expanded);

  function setOpen(open: boolean): void {
    node.classList.toggle("expanded", open);
    node.classList.toggle("collapsed", !open);
    collapsed.style.display = open ? "none" : "flex";
    expanded.style.display = open ? "flex" : "none";
  }
  return { node, termHost, setOpen, onToggle: (fn) => { toggleFn = fn; } };
}

export function buildColdConfirm(fork: ForkReading, cache: CacheReading, onConfirm: () => void, onCancel: () => void, session: Session): HTMLElement {
  const diagram = svg("svg", { width: "100%", height: 56, viewBox: "0 0 460 56" },
    svg("path", { d: "M6 38 H120", stroke: "var(--faint)", "stroke-width": 1.4 }),
    svg("path", { d: "M120 38 H430", stroke: "var(--faint)", "stroke-width": 1.4, "stroke-dasharray": "2 3", opacity: 0.6 }),
    svg("path", { d: "M120 38 C170 38 170 16 220 16 H430", stroke: "var(--cold)", "stroke-width": 1.6, fill: "none" }),
    svg("circle", { cx: 120, cy: 38, r: 3.2, fill: "var(--cold)" }),
    svg("circle", { cx: 430, cy: 16, r: 3.4, fill: "var(--cold)" }),
    svg("circle", { cx: 430, cy: 38, r: 2.6, fill: "var(--faint)" }),
    textNode("296", "11", "var(--cold)", "new path · re-warms from cold"),
    textNode("296", "52", "var(--faint)", `${session.id} · kept, untouched`));

  const card = el("div", { class: "confirm", onclick: (e) => e.stopPropagation() },
    el("div", { class: "top" }, el("span", { class: "dot", style: "background:var(--cold)" }), el("span", { class: "eyebrow", text: "furnace cold · re-warm required" }), el("span", { class: "spacer" }), el("span", { class: "tn", text: `turn ${fork.n} / ${session.total}` })),
    el("div", { class: "lead" }, "The furnace went out ", el("span", { class: "mono", text: fmtClock(cache.idle) }), " ago. Forking now re-warms its prefix from ", el("i", { text: "cold" }), "."),
    el("div", { class: "deliberate", text: "You spent the cache window deliberating — that’s the trade: token economy for control." }),
    el("div", { class: "figures" },
      el("div", {}, el("div", { class: "big", text: String(fork.drops) }), el("div", { class: "cap" }, "turns dropped ", el("span", { class: "faint", text: "· already spent" }))),
      el("div", { class: "vr" }),
      el("div", {}, el("div", { class: "big cold", text: `~${fmtK(fork.prefix)}` }), el("div", { class: "cap" }, "re-warmed from cold ", el("span", { class: "faint", text: `· prefix to turn ${fork.n}` }))),
      el("div", { class: "spacer" })),
    diagram,
    el("div", { class: "actions" },
      el("button", { class: "btn-cold", onclick: onConfirm }, "Fork from cold"),
      el("button", { class: "btn-secondary", onclick: onCancel }, "keep deliberating"),
      el("span", { class: "spacer" }),
      el("span", { class: "note", text: "manuscripts don’t burn" })));

  return el("div", { class: "scrim", onclick: onCancel }, card);
}

export interface ToastInfo { kind: "send" | "fork"; n?: number; drops?: number; prefix?: number; cold?: boolean; }

export function buildForkToast(info: ToastInfo, session: Session): HTMLElement {
  if (info.kind === "send") {
    return el("div", { class: "toast" },
      svg("svg", { width: 15, height: 15, viewBox: "0 0 14 14", fill: "none" },
        svg("path", { d: "M7 13 V5", stroke: "var(--amber)", "stroke-width": 1.2, "stroke-linecap": "round" }),
        svg("path", { d: "M7 6 C7 3 9.4 2 11 2 C11 4.6 9.4 6 7 6 Z", fill: "var(--amber)", opacity: 0.9 })),
      el("span", { class: "msg", text: "Sent — furnace re-lit, cache hot" }));
  }
  return el("div", { class: "toast" },
    svg("svg", { width: 15, height: 15, viewBox: "0 0 14 14", fill: "none" },
      svg("path", { d: "M4 13 V8 C4 6 5 5 7 5", stroke: "var(--temp-color)", "stroke-width": 1.2, "stroke-linecap": "round" }),
      svg("circle", { cx: 11.2, cy: 5, r: 1.4, fill: "var(--temp-color)" })),
    el("span", { class: "msg" }, `Forked at turn ${info.n} → `, el("span", { class: "mono", style: "color:var(--temp-color)", text: "new path" })),
    el("span", { class: "meta", text: `dropped ${info.drops} · ${info.cold ? `re-warm ~${fmtK(info.prefix ?? 0)}` : "warm · cheap"}` }),
    el("span", { class: "kept", text: `${session.id} kept` }));
}

function textNode(x: string, y: string, fill: string, text: string): SVGElement {
  const t = svg("text", { x, y, fill, "font-family": "var(--mono)", "font-size": 10 });
  t.textContent = text;
  return t;
}
