// mind.ts — "the Mind": what is resident in the model's context right now. The same
// object the cache holds and the same tokens that re-warm when the furnace goes cold,
// so cost and contents are one surface. A persistent strip pinned under the clock,
// with the full ledger dropping down as a floating panel reachable from any scroll.
import { el } from "./dom.ts";
import { mindGlyph } from "./marks.ts";
import { MIND, MIND_DELTAS, mindGroupColor } from "./data.ts";
import { fmtK } from "./cooling.ts";

export interface MindHandle {
  node: HTMLElement;
  setOpen(open: boolean): void;
  isOpen(): boolean;
}

export function buildMind(onToggle: () => void): MindHandle {
  const groups = MIND.groups;

  const resbar = el("div", { class: "resbar" },
    ...groups.map((g) => el("i", { title: `${g.label} · ${fmtK(g.tok)}`, style: `flex:${g.tok};background:${mindGroupColor(groups.indexOf(g), groups.length)}` })));

  const moreLabel = el("span", { class: "more", text: "ledger ▸" });
  const strip = el("div", { class: "strip", onclick: onToggle },
    el("span", { class: "caret", text: "▸" }),
    mindGlyph("var(--ink)", 15),
    el("span", { class: "title", text: "The Mind" }),
    resbar,
    el("span", { class: "total tnum", text: fmtK(MIND.total) }),
    el("span", { class: "state show-warm", text: "resident · = the re-warm" }),
    el("span", { class: "state show-cold", text: "= cold · re-reads in full" }),
    moreLabel);

  const ledger = el("div", { class: "ledger", style: "display:none" },
    ...groups.map((g, i) => {
      const items = el("div", { class: "items", style: "display:none" },
        ...g.items.map((it) => el("span", { class: "chip", text: it })));
      const exp = el("span", { class: "exp", text: g.items.length ? "▸" : "" });
      const head = el("div", { class: "ghead", onclick: () => {
        if (!g.items.length) return;
        const showing = items.style.display !== "none";
        items.style.display = showing ? "none" : "flex";
        exp.textContent = showing ? "▸" : "▾";
      } },
        el("span", { class: "sw", style: `background:${mindGroupColor(i, groups.length)}` }),
        el("span", { class: "lab", text: g.label }),
        g.count != null ? el("span", { class: "cnt", text: `${g.count}${g.unit ?? ""}` }) : null,
        el("span", { class: "note", text: g.note }),
        el("span", { class: "spacer" }),
        el("div", { class: "track" }, el("i", { style: `width:${Math.round((g.tok / MIND.total) * 100)}%;background:${mindGroupColor(i, groups.length)}` })),
        el("span", { class: "tk tnum", text: fmtK(g.tok) }),
        exp);
      return el("div", { class: "g" }, head, items);
    }),
    el("div", { class: "foot" },
      "This is the cached prefix — the AI’s working mind. Keep the furnace lit and it stays resident for pennies; let it cool and all ",
      el("b", { text: fmtK(MIND.total) }),
      " are re-read at full price. Evicting anything here is a fork — copy-on-write, the original kept."));

  const node = el("div", { class: "mind" }, strip, ledger);
  let open = false;

  function setOpen(o: boolean): void {
    open = o;
    node.classList.toggle("open", open);
    ledger.style.display = open ? "block" : "none";
    moreLabel.textContent = open ? "close ▾" : "ledger ▸";
  }

  return { node, setOpen, isOpen: () => open };
}

// the per-turn margin diff (Mind lens on): what entered/left the mind at this turn.
export function mindMargin(n: number): HTMLElement {
  const d = MIND_DELTAS[n];
  if (!d) return el("div", { class: "mdiff" }, el("div", { class: "none", text: "· no change" }));
  return el("div", { class: "mdiff" },
    ...d.map((x) => el("div", { class: "row" },
      el("span", { class: x.s === "+" ? "s" : "s rem", text: x.s }),
      el("span", { class: "lab", text: x.label }),
      el("span", { class: "tk", text: fmtK(x.tok) }))));
}
