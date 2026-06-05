// manuscript.ts — the session as an editable document. Fold lives in the gutter
// (caret + turn number); edit lives on the prose ink — different targets, so they
// never collide. Committing an edit forks (drops downstream, keeps the original);
// the cost is read from the live furnace clock. The leaf is a real input that
// re-lights the cache. Hover affordances are pure CSS; only fold/edit rebuild a turn.
import { el, svg } from "./dom.ts";
import { roleMark, leafMark } from "./marks.ts";
import { mindMargin } from "./mind.ts";
import { type Session, type Exchange, type Tool, MIND, MIND_DELTAS } from "./data.ts";
import { type CacheReading, dropsAt, prefixTokensTo, fmtK, fmtClock } from "./cooling.ts";

export interface ManuscriptOpts {
  getCache: () => CacheReading;
  onCommit: (n: number, text: string) => void; // main decides warm-fork vs cold-confirm
  onLeafSend: (text: string) => void;
}

export class Manuscript {
  readonly node: HTMLElement;
  private col: HTMLElement;
  private session: Session;
  private opts: ManuscriptOpts;
  private folded = new Set<number>();
  private editingN: number | null = null;
  private editingEl: HTMLElement | null = null;
  private lensOn = false;
  private exNodes = new Map<number, HTMLElement>();
  private leaf: { node: HTMLElement; update(c: CacheReading): void };

  constructor(session: Session, opts: ManuscriptOpts) {
    this.session = session;
    this.opts = opts;
    this.col = el("div", { class: "ms-col" });
    for (const e of session.exchanges) {
      if (e.leaf) continue;
      const node = this.buildExchange(e);
      this.exNodes.set(e.n, node);
      this.col.appendChild(node);
    }
    this.leaf = this.buildLeaf();
    this.col.appendChild(this.leaf.node);
    this.node = el("div", { class: "ms-scroll" }, this.col);
  }

  setLens(on: boolean): void {
    this.lensOn = on;
    for (const e of this.session.exchanges) if (!e.leaf) this.rebuild(e.n);
  }

  // Swap in a different session (e.g. one chosen from the Forest) and rebuild the column.
  setSession(session: Session): void {
    this.session = session;
    this.folded.clear();
    this.editingN = null;
    this.exNodes.clear();
    this.col.replaceChildren();
    for (const e of session.exchanges) {
      if (e.leaf) continue;
      const node = this.buildExchange(e);
      this.exNodes.set(e.n, node);
      this.col.appendChild(node);
    }
    this.leaf = this.buildLeaf();
    this.col.appendChild(this.leaf.node);
  }

  get scroller(): HTMLElement {
    return this.node;
  }

  closeEdit(): void {
    const n = this.editingN;
    this.editingN = null;
    this.editingEl = null;
    if (n != null) this.rebuild(n);
  }

  // read the live contenteditable text and hand it up for the (real or stub) fork
  private commitEdit(n: number): void {
    const text = (this.editingEl?.textContent ?? "").trim();
    this.opts.onCommit(n, text);
  }

  tick(cache: CacheReading): void {
    this.leaf.update(cache);
  }

  private exchange(n: number): Exchange {
    return this.session.exchanges.find((e) => e.n === n)!;
  }

  private rebuild(n: number): void {
    const fresh = this.buildExchange(this.exchange(n));
    this.exNodes.get(n)!.replaceWith(fresh);
    this.exNodes.set(n, fresh);
  }

  private toggleFold(n: number): void {
    if (this.folded.has(n)) this.folded.delete(n);
    else this.folded.add(n);
    this.rebuild(n);
  }

  private openEdit(n: number): void {
    if (this.editingN != null && this.editingN !== n) {
      const prev = this.editingN;
      this.editingN = null;
      this.rebuild(prev);
    }
    this.editingN = n;
    this.rebuild(n);
  }

  private gutter(e: Exchange): HTMLElement {
    return el("div", { class: "gutter", onclick: (ev) => { ev.stopPropagation(); this.toggleFold(e.n); } },
      el("span", { class: "caret", text: "▸" }),
      el("span", { class: "num tnum", text: String(e.n) }));
  }

  private buildExchange(e: Exchange): HTMLElement {
    const folded = this.folded.has(e.n);
    const editing = this.editingN === e.n;

    if (folded) {
      const delta = MIND_DELTAS[e.n];
      return el("div", { class: "xchg folded" },
        el("div", { class: "main" },
          el("div", { class: "foldline" },
            this.gutter(e),
            el("span", { class: "ftitle", onclick: () => this.toggleFold(e.n), text: e.user }),
            e.tool ? el("span", { class: "badge", text: "⏍ 1" }) : null,
            delta && this.lensOn ? el("span", { class: "badge mind", text: `+${fmtK(delta.reduce((s, x) => s + x.tok, 0))}` }) : null)),
        el("div", { class: "margin" }));
    }

    const drops = dropsAt(e.n, this.session.total);

    // the user prose — clicking the ink edits
    const userProse = editing
      ? el("p", { class: "prose user editing" })
      : el("p", { class: "prose user", title: "click to revise", onclick: () => this.openEdit(e.n), text: e.user });
    if (editing) {
      userProse.contentEditable = "true";
      userProse.textContent = e.user;
      this.editingEl = userProse;
      userProse.addEventListener("keydown", (ev) => {
        const k = ev as KeyboardEvent;
        if (k.key === "Enter" && !k.shiftKey) { k.preventDefault(); this.commitEdit(e.n); }
        if (k.key === "Escape") { k.preventDefault(); this.closeEdit(); }
      });
      queueMicrotask(() => {
        userProse.focus();
        const sel = window.getSelection();
        const rng = document.createRange();
        rng.selectNodeContents(userProse);
        rng.collapse(false);
        sel?.removeAllRanges();
        sel?.addRange(rng);
      });
    }

    const replies = el("div", { class: "replies" },
      el("div", { class: "rolecol" }, roleMark("assistant", "var(--agent)", 7)),
      el("div", { style: "flex:1;min-width:0" },
        el("p", { class: "prose assistant", text: e.assistant ?? "" }),
        e.tool ? this.buildTool(e.tool) : null,
        e.system ? el("div", { class: "sysline" }, roleMark("system", "var(--faint)", 6), " " + e.system) : null));

    const main = el("div", { class: "main" },
      el("div", { class: "rule" },
        this.gutter(e),
        el("span", { class: "hr" }),
        el("span", { class: "hint", text: `drops ${drops} · revise the ink ↵` })),
      el("div", { style: "position:relative;padding:2px 14px;margin:0 -14px" },
        el("div", { class: "turn" }, el("div", { class: "rolecol" }, roleMark("user", "var(--ink)", 7)), userProse),
        replies,
        editing ? this.buildForkBanner(e.n) : null));

    return el("div", { class: `xchg${editing ? " editing" : ""}` }, main, el("div", { class: "margin" }, this.margin(e)));
  }

  private margin(e: Exchange): HTMLElement {
    if (this.lensOn) return mindMargin(e.n);
    const drops = dropsAt(e.n, this.session.total);
    const prefix = prefixTokensTo(e.n, this.session.total);
    return el("div", { class: "reading" },
      el("div", { class: "drops" }, "fork drops ", el("b", { text: String(drops) }), " turns", el("span", { class: "spent", text: " · already spent" })),
      el("div", { class: "temp" },
        el("span", { class: "swatch" }),
        el("span", { class: "show-warm", text: "warm · ~cheap" }),
        el("span", { class: "show-cold", text: `re-warm ~${fmtK(prefix)}` })));
  }

  private buildTool(tool: Tool): HTMLElement {
    const body = tool.detail
      ? el("div", { class: "body", style: "display:none" },
          ...tool.detail.lines.map((l) => el("div", { class: `ln ${l.c}`, text: l.t })),
          el("div", { class: "note", text: `this result is resident in the Mind · ${fmtK(tool.detail.tok)} of the ${fmtK(MIND.total)} prefix` }))
      : null;
    const caret = el("span", { class: "c", text: "▸" });
    const head = el("div", { class: "head", onclick: (ev) => {
      ev.stopPropagation();
      if (!body) return;
      const showing = body.style.display !== "none";
      body.style.display = showing ? "none" : "block";
      caret.textContent = showing ? "▸" : "▾";
    } },
      caret,
      el("span", { class: "c", text: "⏍" }),
      el("span", { class: "kind", text: tool.kind }),
      el("span", { text: tool.arg }),
      el("span", { class: "delta", text: tool.delta }),
      el("span", { class: "spacer" }),
      tool.detail ? el("span", { class: "ctx", text: `+${fmtK(tool.detail.tok)} ctx` }) : null);
    return el("div", { class: "tool" }, head, body);
  }

  private buildForkBanner(n: number): HTMLElement {
    const cache = this.opts.getCache();
    const drops = dropsAt(n, this.session.total);
    const prefix = prefixTokensTo(n, this.session.total);
    const lead = el("p", {},
      "Committing ", el("i", { text: "forks a new path" }), " — drops the ",
      el("span", { class: "mono", text: String(drops) }), " turns below.",
      el("span", { class: "show-cold" }, " The furnace is ", el("i", { text: "cold" }), ": the branch re-warms ", el("span", { class: "mono", text: `~${fmtK(prefix)}` }), " from a cold prefix."),
      el("span", { class: "show-warm" }, " Cache is warm — commit within ", el("span", { class: "mono", text: fmtClock(cache.remaining) }), " and it stays cheap."));
    return el("div", { class: "forkbanner" },
      el("div", { class: "lead" }, forkMarkInline(), lead),
      el("div", { class: "actions" },
        el("button", { class: "btn-primary", onclick: () => this.commitEdit(n) }, "Revise & fork ↵"),
        el("button", { class: "btn-secondary", onclick: () => this.closeEdit() }, "esc"),
        el("span", { class: "spacer" }),
        el("span", { class: "note", text: "you’re off the clock while you edit" })));
  }

  private buildLeaf(): { node: HTMLElement; update(c: CacheReading): void } {
    const ta = el("textarea", { rows: 1, placeholder: "continue the session… (typing feeds the furnace)" }) as HTMLTextAreaElement;
    const send = el("button", { class: "send", onclick: () => fire() }, "send ↵");
    const foot = el("div", { class: "foot" });
    const caret = el("span", { class: "caret wb-caret" });

    const fire = (): void => {
      const v = ta.value.trim();
      if (!v) return;
      this.opts.onLeafSend(v);
      ta.value = "";
      send.classList.remove("ready");
    };
    ta.addEventListener("input", () => send.classList.toggle("ready", ta.value.trim().length > 0));
    ta.addEventListener("keydown", (ev) => {
      const k = ev as KeyboardEvent;
      if (k.key === "Enter" && !k.shiftKey) { k.preventDefault(); fire(); }
    });

    const node = el("div", { class: "leaf" },
      el("div", { style: "flex:1;min-width:0" },
        el("div", { class: "rule" },
          el("span", { class: "n tnum", text: String(this.session.total) }),
          el("span", { class: "hr" }),
          el("span", { class: "tag", text: "the leaf · free · live" })),
        el("div", { class: "input" },
          el("div", { class: "leafcol" }, leafMark("var(--amber)", 16)),
          el("div", { class: "field" }, caret, ta, send)),
        foot),
      el("div", { class: "margin" }));

    const update = (c: CacheReading): void => {
      caret.classList.toggle("wb-caret", !c.cold);
      foot.classList.toggle("cold", c.cold);
      foot.textContent = c.cold
        ? `furnace cold — this message re-warms ~${fmtK(c.reWarmFull)} before it runs`
        : `furnace ${c.label} · sending now keeps the cache hot · cold in ${fmtClock(c.remaining)}`;
    };
    return { node, update };
  }
}

function forkMarkInline(): SVGElement {
  return svg("svg", { width: 15, height: 15, viewBox: "0 0 14 14", fill: "none" },
    svg("path", { d: "M4 13 V8 C4 6 5 5 7 5", stroke: "var(--temp-color)", "stroke-width": 1.2, "stroke-linecap": "round" }),
    svg("path", { d: "M7 5 H11", stroke: "var(--temp-color)", "stroke-width": 1.2, "stroke-linecap": "round", opacity: 0.55 }),
    svg("circle", { cx: 4, cy: 13, r: 1.4, fill: "var(--temp-color)" }),
    svg("circle", { cx: 11.2, cy: 5, r: 1.4, fill: "var(--temp-color)" }));
}
