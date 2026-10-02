/**
 * annotator.ts — the DOM half of plan annotation (pure core: annotate.ts).
 *
 * Renders a markdown artifact as plain-text blocks in eigenform's own origin (never
 * innerHTML: the source is agent-written), lets the reader select text and mark it
 * (comment / delete / replace), and compiles the marks into one critique that is
 * STAGED into the active terminal's input, never sent. Marks persist per
 * (session, file) in localStorage and re-anchor when the agent revises the file.
 */

import { mdBlocks, reanchor, segments, compileFeedback } from "./annotate.ts";
import { el } from "./dom.ts";
import type { Annotation, AnnotationKind, Block } from "./annotate.ts";

export interface AnnotatorOpts {
  uuid: string;
  path: string;
  /** Same-origin URL of the raw markdown source (`/artifact/…?raw=1`). */
  rawUrl: string;
  /** Type the critique into the active terminal, unsent. False if there's no live pty. */
  stage(text: string): boolean;
  /** Override the localStorage key the marks persist under (default: session + path). */
  storageKey?: string;
  /**
   * Plan-gate mode: Claude is waiting on this plan (ExitPlanMode). The footer offers
   * Approve / Send back / Decide in terminal instead of staging. Send back delivers the
   * compiled critique as the plan feedback Claude revises against.
   */
  review?: {
    decide(kind: "approve" | "send_back" | "terminal", message?: string): Promise<boolean>;
  };
  /** First line of the compiled critique (default names the file and asks for a revision). */
  header?: string;
}

export interface AnnotatorHandle {
  /** Re-fetch the source and re-anchor (the agent may have revised it). */
  refresh(): void;
  close(): void;
}

interface Saved {
  anns: Annotation[];
  general: string;
}

const LS_PREFIX = "eigenform:annot:v1:";

function load(key: string): Saved {
  try {
    const raw = localStorage.getItem(key);
    if (raw) {
      const v = JSON.parse(raw) as Saved;
      if (Array.isArray(v.anns)) return { anns: v.anns, general: v.general ?? "" };
    }
  } catch {
    // unreadable or blocked storage — start empty
  }
  return { anns: [], general: "" };
}

function save(key: string, s: Saved) {
  try {
    if (s.anns.length === 0 && s.general.trim() === "") localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(s));
  } catch {
    // best-effort
  }
}

const KIND_LABEL: Record<AnnotationKind, string> = { comment: "comment", delete: "delete", replace: "replace" };

export function mountAnnotator(host: HTMLElement, opts: AnnotatorOpts): AnnotatorHandle {
  const key = opts.storageKey ?? `${LS_PREFIX}${opts.uuid}:${opts.path}`;
  let state = load(key);
  let blocks: Block[] = [];
  let seq = 0;

  const root = el("div", "ann");
  const doc = el("div", "ann-doc");
  const foot = el("div", "ann-foot");
  root.append(doc, foot);
  host.append(root);

  let pop: HTMLElement | null = null;
  const closePop = () => {
    pop?.remove();
    pop = null;
  };

  function persist() {
    save(key, state);
  }

  // ── document ────────────────────────────────────────────────────────────────
  function renderDoc() {
    doc.innerHTML = "";
    if (blocks.length === 0) {
      const e = el("div", "ann-empty");
      e.textContent = "This file is empty.";
      doc.append(e);
      return;
    }
    blocks.forEach((b, i) => {
      const be = el(b.type === "code" || b.type === "table" ? "pre" : "div", `ann-block ann-block--${b.type}`);
      be.dataset.i = String(i);
      if (b.type === "li") {
        be.style.setProperty("--depth", String(b.depth));
        be.dataset.marker = b.marker ?? "•";
      }
      const mine = state.anns.filter((a) => a.block === i);
      for (const s of segments(b.text, mine)) {
        if (!s.ann) {
          be.append(document.createTextNode(s.text));
          continue;
        }
        const m = el("mark", `ann-mark ann-mark--${s.ann.kind}`);
        m.textContent = s.text;
        m.title = describe(s.ann);
        m.dataset.id = s.ann.id;
        be.append(m);
      }
      doc.append(be);
    });
  }

  // ── selection → popover ────────────────────────────────────────────────────
  function blockOf(node: Node | null): HTMLElement | null {
    let n: Node | null = node;
    while (n && n !== doc) {
      if (n instanceof HTMLElement && n.dataset.i !== undefined) return n;
      n = n.parentNode;
    }
    return null;
  }

  function offsetIn(blockEl: HTMLElement, node: Node, off: number): number {
    const r = document.createRange();
    r.setStart(blockEl, 0);
    r.setEnd(node, off);
    return r.toString().length;
  }

  doc.addEventListener("mouseup", () => {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0) return;
    const range = sel.getRangeAt(0);
    const startBlock = blockOf(range.startContainer);
    if (!startBlock) return;
    const i = Number(startBlock.dataset.i);
    const b = blocks[i];
    if (!b) return;
    const start = offsetIn(startBlock, range.startContainer, range.startOffset);
    // A selection running past its block is clipped to the block it started in.
    const endBlock = blockOf(range.endContainer);
    const end = endBlock === startBlock ? offsetIn(startBlock, range.endContainer, range.endOffset) : b.text.length;
    const raw = b.text.slice(start, end);
    const lead = raw.length - raw.trimStart().length;
    const quote = raw.trim();
    if (!quote) return;
    openPop(range.getBoundingClientRect(), { block: i, start: start + lead, quote, section: b.section });
  });

  doc.addEventListener("click", (e) => {
    const m = (e.target as HTMLElement).closest?.("mark.ann-mark") as HTMLElement | null;
    if (!m || !window.getSelection()?.isCollapsed) return;
    const a = state.anns.find((x) => x.id === m.dataset.id);
    if (a) openPop(m.getBoundingClientRect(), a, a);
  });

  function openPop(
    rect: DOMRect,
    at: { block: number; start: number; quote: string; section: string },
    existing?: Annotation,
  ) {
    closePop();
    pop = el("div", "ann-pop");
    const r = root.getBoundingClientRect();
    pop.style.top = `${Math.max(4, rect.bottom - r.top + 6)}px`;
    pop.style.left = `${Math.max(4, Math.min(rect.left - r.left, r.width - 300))}px`;

    let kind: AnnotationKind = existing?.kind ?? "comment";
    const kinds = el("div", "ann-kinds");
    const kindBtns = (["comment", "delete", "replace"] as AnnotationKind[]).map((k) => {
      const btn = el("button", "ann-kind");
      btn.textContent = KIND_LABEL[k];
      btn.addEventListener("click", () => {
        kind = k;
        sync();
      });
      kinds.append(btn);
      return [k, btn] as const;
    });
    const replacement = el("textarea", "ann-input ann-input--replace");
    replacement.placeholder = "replace with…";
    replacement.rows = 2;
    replacement.value = existing?.replacement ?? at.quote;
    const note = el("textarea", "ann-input");
    note.rows = 2;
    note.value = existing?.note ?? "";
    const actions = el("div", "ann-actions");
    const saveBtn = el("button", "ann-btn ann-btn--primary");
    saveBtn.textContent = existing ? "Update" : "Mark";
    const cancel = el("button", "ann-btn");
    cancel.textContent = "Cancel";
    actions.append(cancel);
    if (existing) {
      const del = el("button", "ann-btn");
      del.textContent = "Remove";
      del.addEventListener("click", () => {
        state.anns = state.anns.filter((x) => x.id !== existing.id);
        persist();
        closePop();
        renderAll();
      });
      actions.append(del);
    }
    actions.append(saveBtn);

    const quote = el("div", "ann-pop-quote");
    quote.textContent = at.quote;
    pop.append(quote, kinds, replacement, note, actions);

    function sync() {
      for (const [k, btn] of kindBtns) btn.classList.toggle("ann-kind--on", k === kind);
      replacement.style.display = kind === "replace" ? "" : "none";
      note.placeholder = kind === "comment" ? "your comment…" : "why (optional)…";
    }
    sync();

    saveBtn.addEventListener("click", () => {
      const n = note.value.trim();
      if (kind === "comment" && !n) {
        note.focus();
        return;
      }
      const next: Annotation = {
        id: existing?.id ?? `a${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`,
        kind,
        quote: at.quote,
        block: at.block,
        start: at.start,
        section: at.section,
        ...(n ? { note: n } : {}),
        ...(kind === "replace" ? { replacement: replacement.value } : {}),
      };
      state.anns = existing ? state.anns.map((x) => (x.id === existing.id ? next : x)) : [...state.anns, next];
      persist();
      window.getSelection()?.removeAllRanges();
      closePop();
      renderAll();
    });
    cancel.addEventListener("click", closePop);
    pop.addEventListener("keydown", (e) => {
      if (e.key === "Escape") closePop();
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) saveBtn.click();
    });
    root.append(pop);
    (kind === "replace" ? replacement : note).focus();
  }

  // ── footer: marks, overall note, stage ─────────────────────────────────────
  const status = el("div", "ann-status");
  function renderFoot() {
    foot.innerHTML = "";
    const list = el("div", "ann-list");
    if (state.anns.length === 0) {
      const hint = el("div", "ann-hint");
      hint.textContent = "Select text in the plan to comment on it, strike it, or replace it.";
      list.append(hint);
    }
    for (const a of state.anns) {
      const row = el("button", `ann-item${a.orphaned ? " ann-item--orphan" : ""}`);
      row.title = a.orphaned ? "This text is no longer in the file; the note still compiles." : "Edit";
      const k = el("span", `ann-item-kind ann-item-kind--${a.kind}`);
      k.textContent = KIND_LABEL[a.kind];
      const q = el("span", "ann-item-quote");
      q.textContent = a.quote;
      const n = el("span", "ann-item-note");
      n.textContent = a.kind === "replace" ? `→ ${a.replacement ?? ""}` : (a.note ?? "");
      row.append(k, q, n);
      row.addEventListener("click", () => {
        const m = doc.querySelector(`mark[data-id="${CSS.escape(a.id)}"]`);
        if (m) {
          m.scrollIntoView({ block: "center", behavior: "smooth" });
          openPop(m.getBoundingClientRect(), a, a);
        } else {
          openPop(row.getBoundingClientRect(), a, a);
        }
      });
      list.append(row);
    }
    const general = el("textarea", "ann-input ann-general");
    general.rows = 2;
    general.placeholder = "Overall note (optional)…";
    general.value = state.general;
    general.addEventListener("input", () => {
      state.general = general.value;
      persist();
      if (!opts.review) stageBtn.disabled = state.anns.length === 0 && state.general.trim() === "";
    });
    const actions = el("div", "ann-actions");
    const clear = el("button", "ann-btn");
    clear.textContent = "Clear";
    clear.addEventListener("click", () => {
      if (state.anns.length && !confirm(`Discard ${state.anns.length} mark(s)?`)) return;
      state = { anns: [], general: "" };
      persist();
      status.textContent = "";
      renderAll();
    });
    const empty = state.anns.length === 0 && state.general.trim() === "";
    const n = state.anns.length;
    const stageBtn = el("button", "ann-btn ann-btn--primary");
    stageBtn.textContent = `Stage in terminal${n ? ` (${n})` : ""}`;
    stageBtn.title = "Types the compiled critique into the terminal input. It is NOT sent: review it there, then press Enter.";
    stageBtn.disabled = empty;
    stageBtn.addEventListener("click", () => {
      const text = compileFeedback(opts.path, state.anns, state.general, opts.header);
      if (!opts.stage(text)) {
        status.textContent = "No live terminal in this tab to stage into.";
        return;
      }
      // Staged: the critique now lives in the input. Clear so a second click can't
      // stage a duplicate.
      state = { anns: [], general: "" };
      persist();
      renderAll();
      status.textContent = "Staged in the terminal, not sent. Review it there and press Enter.";
    });

    if (opts.review) {
      const review = opts.review;
      const decide = async (kind: "approve" | "send_back" | "terminal", message?: string) => {
        for (const b of [terminalBtn, sendBtn, approveBtn]) b.disabled = true;
        if (await review.decide(kind, message)) {
          state = { anns: [], general: "" };
          persist();
          status.textContent =
            kind === "approve"
              ? "Approved. Claude is proceeding."
              : kind === "send_back"
                ? "Sent back. Claude is revising the plan against your notes."
                : "Handed to the terminal's own approval prompt.";
        } else {
          status.textContent = "This review is no longer pending (decided elsewhere, or Claude stopped waiting).";
        }
      };
      const terminalBtn = el("button", "ann-btn");
      terminalBtn.textContent = "Decide in terminal";
      terminalBtn.title = "No decision here: Claude Code shows its own approval prompt.";
      terminalBtn.addEventListener("click", () => void decide("terminal"));
      const sendBtn = el("button", "ann-btn");
      sendBtn.textContent = `Send back${n ? ` (${n})` : ""}`;
      sendBtn.title = "Reject the plan with your marks as the feedback Claude revises against.";
      sendBtn.disabled = empty;
      sendBtn.addEventListener("click", () =>
        void decide("send_back", compileFeedback(opts.path, state.anns, state.general, opts.header)),
      );
      const approveBtn = el("button", "ann-btn ann-btn--primary");
      approveBtn.textContent = "Approve";
      approveBtn.addEventListener("click", () => {
        if (!empty && !confirm(`Approve and discard your ${n} mark(s)? (Send back delivers them.)`)) return;
        void decide("approve");
      });
      general.addEventListener("input", () => {
        sendBtn.disabled = state.anns.length === 0 && state.general.trim() === "";
      });
      actions.append(clear, terminalBtn, sendBtn, approveBtn);
      foot.append(list, general, actions, status);
      return;
    }
    actions.append(clear, stageBtn);
    foot.append(list, general, actions, status);
  }

  function renderAll() {
    renderDoc();
    renderFoot();
  }

  async function refresh() {
    const mine = ++seq;
    try {
      const res = await fetch(opts.rawUrl, { cache: "no-store" });
      const src = res.ok ? await res.text() : "";
      if (mine !== seq) return;
      blocks = mdBlocks(src);
      state.anns = reanchor(state.anns, blocks);
      persist();
      renderAll();
    } catch {
      // keep the last render
    }
  }

  renderAll();
  void refresh();
  return {
    refresh: () => void refresh(),
    close() {
      seq++;
      closePop();
      root.remove();
    },
  };
}

function describe(a: Annotation): string {
  if (a.kind === "delete") return `delete${a.note ? `: ${a.note}` : ""}`;
  if (a.kind === "replace") return `replace with: ${a.replacement ?? ""}${a.note ? ` (${a.note})` : ""}`;
  return a.note ?? "";
}
