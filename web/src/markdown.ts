// markdown.ts — a small, dependency-free markdown renderer for the assistant prose.
// Two pure layers (an AST: parseBlocks + inline) plus a thin DOM layer
// (renderMarkdown). The split keeps the parser testable under `node --test` and
// makes the engine a SEAM: a future remark/AST pipeline could replace the two pure
// functions and renderMarkdown would barely change. No innerHTML — every node is
// built with el(), so Claude's prose can never inject markup.
//
// Scope (v1): **bold**, *italic*, `code`, ~~strike~~, ATX headings, bullet/ordered
// lists, fenced code, paragraphs. We deliberately do NOT treat `_` as emphasis —
// this is a coding workbench and snake_case identifiers are everywhere.
import { el } from "./dom.ts";

export type Inline =
  | { type: "text"; value: string }
  | { type: "strong"; children: Inline[] }
  | { type: "em"; children: Inline[] }
  | { type: "strike"; children: Inline[] }
  | { type: "code"; value: string };

export type Block =
  | { type: "p"; spans: Inline[] }
  | { type: "heading"; level: number; spans: Inline[] }
  | { type: "ul"; items: Inline[][] }
  | { type: "ol"; start: number; items: Inline[][] }
  | { type: "code"; lang: string; value: string };

const isWs = (c: string | undefined): boolean => c === undefined || /\s/.test(c);

// Find the start of a closing run of `ch` (length ≥ minLen) at or after `from` whose
// char immediately before is non-space (a valid right-flanking closer). −1 if none.
function findCloser(text: string, from: number, ch: string, minLen: number): number {
  let j = from;
  while (j < text.length) {
    if (text[j] === ch) {
      let k = j;
      while (text[k] === ch) k++;
      if (k - j >= minLen && !isWs(text[j - 1])) return j;
      j = k;
    } else {
      j++;
    }
  }
  return -1;
}

const wrap = (use: number, children: Inline[]): Inline =>
  use >= 3 ? { type: "strong", children: [{ type: "em", children }] }
  : use === 2 ? { type: "strong", children }
  : { type: "em", children };

// Try to read an emphasis/strike span opening at `i` (text[i] is '*' or '~'). Returns
// the produced nodes (with any leftover opening markers as leading text) and the index
// to continue from, or null if this run doesn't open a valid span.
function matchRun(text: string, i: number): { nodes: Inline[]; next: number } | null {
  const ch = text[i]!;
  let r = i;
  while (text[r] === ch) r++;
  const openLen = r - i;
  if (isWs(text[r])) return null; // not left-flanking — a space follows the run

  const minLen = ch === "~" ? 2 : 1;
  if (openLen < minLen) return null;
  const c = findCloser(text, r, ch, minLen);
  if (c < 0) return null;
  let cr = c;
  while (text[cr] === ch) cr++;
  const closeLen = cr - c;

  if (ch === "~") {
    // strike pairs exactly two tildes, innermost-first.
    const inner = text.slice(r, c + closeLen - 2);
    const lead = openLen - 2;
    const nodes: Inline[] = [];
    if (lead > 0) nodes.push({ type: "text", value: "~".repeat(lead) });
    nodes.push({ type: "strike", children: inline(inner) });
    return { nodes, next: c + closeLen };
  }

  // '*': pair the innermost `use` markers of each run; markers nearest the content
  // bind first, so extra closing markers stay inside `inner` for nested emphasis
  // (this is what makes ***x*** and **a *b*** nest correctly).
  const use = Math.min(openLen, closeLen, 3);
  const inner = text.slice(r, c + (closeLen - use));
  const lead = openLen - use;
  const nodes: Inline[] = [];
  if (lead > 0) nodes.push({ type: "text", value: "*".repeat(lead) });
  nodes.push(wrap(use, inline(inner)));
  return { nodes, next: c + closeLen };
}

export function inline(text: string): Inline[] {
  const out: Inline[] = [];
  let buf = "";
  let i = 0;
  const flush = (): void => { if (buf) { out.push({ type: "text", value: buf }); buf = ""; } };

  while (i < text.length) {
    const ch = text[i];
    if (ch === "`") {
      const close = text.indexOf("`", i + 1);
      if (close > i) { flush(); out.push({ type: "code", value: text.slice(i + 1, close) }); i = close + 1; continue; }
    }
    if (ch === "*" || ch === "~") {
      const m = matchRun(text, i);
      if (m) { flush(); out.push(...m.nodes); i = m.next; continue; }
    }
    buf += ch;
    i++;
  }
  flush();
  return out;
}

const reBullet = /^\s*[-*+]\s+/;
const reOrdered = /^\s*\d+\.\s+/;
const reHeading = /^(#{1,6})\s+(.*)$/;
const reFence = /^```(.*)$/;

export function parseBlocks(text: string): Block[] {
  const lines = text.split("\n");
  const at = (k: number): string => lines[k] ?? "";
  const blocks: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = at(i);
    if (line.trim() === "") { i++; continue; }

    const fence = reFence.exec(line);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(at(i))) { body.push(at(i)); i++; }
      i++; // consume the closing fence (if any)
      blocks.push({ type: "code", lang: fence[1]!.trim(), value: body.join("\n") });
      continue;
    }

    const h = reHeading.exec(line);
    if (h) {
      blocks.push({ type: "heading", level: Math.min(h[1]!.length, 3), spans: inline(h[2]!.trim()) });
      i++;
      continue;
    }

    if (reBullet.test(line)) {
      const items: Inline[][] = [];
      while (i < lines.length && reBullet.test(at(i))) { items.push(inline(at(i).replace(reBullet, ""))); i++; }
      blocks.push({ type: "ul", items });
      continue;
    }

    if (reOrdered.test(line)) {
      const start = parseInt(/^\s*(\d+)\./.exec(line)![1]!, 10);
      const items: Inline[][] = [];
      while (i < lines.length && reOrdered.test(at(i))) { items.push(inline(at(i).replace(reOrdered, ""))); i++; }
      blocks.push({ type: "ol", start, items });
      continue;
    }

    // paragraph: gather consecutive lines until a blank line or a block-starter
    const para: string[] = [];
    while (
      i < lines.length && at(i).trim() !== "" &&
      !/^```/.test(at(i)) && !reHeading.test(at(i)) &&
      !reBullet.test(at(i)) && !reOrdered.test(at(i))
    ) {
      para.push(at(i));
      i++;
    }
    blocks.push({ type: "p", spans: inline(para.join("\n")) });
  }
  return blocks;
}

// ── DOM layer (verified in-app, not unit-tested — no DOM under node --test) ──
function renderInline(spans: Inline[]): (Node | string)[] {
  return spans.map((s) => {
    switch (s.type) {
      case "text": return s.value;
      case "code": return el("code", { text: s.value });
      case "strong": return el("strong", {}, ...renderInline(s.children));
      case "em": return el("em", {}, ...renderInline(s.children));
      case "strike": return el("s", {}, ...renderInline(s.children));
    }
  });
}

export function renderMarkdown(text: string): Node[] {
  return parseBlocks(text).map((b): Node => {
    switch (b.type) {
      case "p": return el("p", {}, ...renderInline(b.spans));
      case "heading": {
        const tag = (`h${b.level}`) as "h1" | "h2" | "h3";
        return el(tag, {}, ...renderInline(b.spans));
      }
      case "ul": return el("ul", {}, ...b.items.map((it) => el("li", {}, ...renderInline(it))));
      case "ol": {
        const ol = el("ol", {}, ...b.items.map((it) => el("li", {}, ...renderInline(it))));
        if (b.start !== 1) ol.start = b.start;
        return ol;
      }
      case "code": return el("pre", { class: "code" }, el("code", { text: b.value }));
    }
  });
}
