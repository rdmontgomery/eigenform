/**
 * annotate.ts — pure core of plan annotation (plannotator-style): parse a markdown
 * source into blocks, anchor the reader's marks (comment / delete / replace) to quoted
 * text, re-anchor them when the agent revises the file, and compile them into ONE
 * critique that is STAGED into the terminal input — never sent. The human releases it.
 *
 * Safety: the annotator renders agent-written markdown in eigenform's own origin, so
 * it never goes near innerHTML. Blocks are plain text (rendered with textContent);
 * inline markdown stays as source, which also makes every quote findable in the file.
 *
 * PURE: no DOM — tested with node --test (annotate.test.ts).
 */

export type BlockType = "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "li" | "code" | "quote" | "table";

export interface Block {
  type: BlockType;
  text: string;
  /** List nesting depth (li only), 0-based. */
  depth: number;
  /** The list marker as written (`-`, `1.`, …); li only, "" otherwise. */
  marker?: string;
  /** Text of the nearest heading above (itself, for a heading); "" before any. */
  section: string;
}

/** Split a markdown source into annotatable blocks. Not a full CommonMark parser —
 *  the granularity a reader marks up a plan at: headings, paragraphs, list items,
 *  fenced code, quotes, tables. */
export function mdBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let section = "";
  let para: string[] = [];
  let quote: string[] = [];
  let table: string[] = [];

  const flushPara = () => {
    if (para.length) out.push({ type: "p", text: para.join(" "), depth: 0, section });
    para = [];
  };
  const flushQuote = () => {
    if (quote.length) out.push({ type: "quote", text: quote.join(" "), depth: 0, section });
    quote = [];
  };
  const flushTable = () => {
    if (table.length) out.push({ type: "table", text: table.join("\n"), depth: 0, section });
    table = [];
  };
  const flushAll = () => {
    flushPara();
    flushQuote();
    flushTable();
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const fence = /^\s*(```+|~~~+)/.exec(line);
    if (fence) {
      flushAll();
      const close = fence[1]!;
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.trimStart().startsWith(close)) body.push(lines[i++]!);
      out.push({ type: "code", text: body.join("\n"), depth: 0, section });
      continue;
    }
    if (line.trim() === "") {
      flushAll();
      continue;
    }
    if (/^\s{0,3}([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flushAll();
      continue; // thematic break
    }
    const h = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (h) {
      flushAll();
      section = h[2]!;
      out.push({ type: `h${h[1]!.length}` as BlockType, text: h[2]!, depth: 0, section });
      continue;
    }
    const li = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/.exec(line);
    if (li) {
      flushAll();
      out.push({
        type: "li",
        text: li[3]!,
        depth: Math.floor(li[1]!.replace(/\t/g, "  ").length / 2),
        marker: /\d/.test(li[2]!) ? li[2]! : "•",
        section,
      });
      continue;
    }
    if (/^\s*>/.test(line)) {
      flushPara();
      flushTable();
      quote.push(line.replace(/^\s*>\s?/, ""));
      continue;
    }
    if (/^\s*\|/.test(line)) {
      flushPara();
      flushQuote();
      table.push(line.trim());
      continue;
    }
    // A continuation line of the previous list item (indented, no blank between).
    const prev = out[out.length - 1];
    if (prev && prev.type === "li" && para.length === 0 && /^\s+\S/.test(line)) {
      prev.text += " " + line.trim();
      continue;
    }
    flushQuote();
    flushTable();
    para.push(line.trim());
  }
  flushAll();
  return out;
}

export type AnnotationKind = "comment" | "delete" | "replace";

export interface Annotation {
  id: string;
  kind: AnnotationKind;
  /** The marked text, verbatim from the block. */
  quote: string;
  /** Block index the quote was found in (re-resolved on re-anchor). */
  block: number;
  /** Char offset of the quote within that block's text. */
  start: number;
  /** Section heading at mark time — kept so an orphaned mark still says where it was. */
  section: string;
  note?: string;
  replacement?: string;
  /** Set by reanchor when the quote no longer appears in the source. */
  orphaned?: boolean;
}

/**
 * Re-resolve marks against (possibly revised) blocks: same block and offset if the
 * quote is still there; else the first occurrence in the same block; else anywhere;
 * else orphaned (kept — the critique of removed text can still matter).
 */
export function reanchor(anns: Annotation[], blocks: Block[]): Annotation[] {
  return anns.map((a) => {
    const at = blocks[a.block];
    if (at && at.text.substr(a.start, a.quote.length) === a.quote) return { ...a, orphaned: false };
    if (at) {
      const j = at.text.indexOf(a.quote);
      if (j >= 0) return { ...a, start: j, orphaned: false };
    }
    for (let b = 0; b < blocks.length; b++) {
      const j = blocks[b]!.text.indexOf(a.quote);
      if (j >= 0) return { ...a, block: b, start: j, section: blocks[b]!.section, orphaned: false };
    }
    return { ...a, orphaned: true };
  });
}

/** A block's text cut into runs, each either plain or covered by one mark. Overlaps
 *  resolve to the earlier-starting mark. */
export function segments(text: string, anns: Annotation[]): { text: string; ann?: Annotation }[] {
  const marks = anns
    .filter((a) => !a.orphaned && a.quote.length > 0)
    .sort((x, y) => x.start - y.start);
  const out: { text: string; ann?: Annotation }[] = [];
  let pos = 0;
  for (const m of marks) {
    if (m.start < pos) continue; // overlapped
    if (m.start > pos) out.push({ text: text.slice(pos, m.start) });
    out.push({ text: text.slice(m.start, m.start + m.quote.length), ann: m });
    pos = m.start + m.quote.length;
  }
  if (pos < text.length) out.push({ text: text.slice(pos) });
  return out;
}

const QUOTE_MAX = 160;

function clip(s: string): string {
  const one = s.replace(/\s+/g, " ").trim();
  return one.length > QUOTE_MAX ? one.slice(0, QUOTE_MAX - 1) + "…" : one;
}

/**
 * Compile marks into one critique, in document order. The header names the file so
 * the agent edits the right thing; each item quotes the text it's about.
 */
export function compileFeedback(path: string, anns: Annotation[], general: string): string {
  const name = path.split("/").pop() ?? path;
  const ordered = [...anns].sort((a, b) =>
    a.orphaned === b.orphaned ? a.block - b.block || a.start - b.start : a.orphaned ? 1 : -1,
  );
  const lines: string[] = [`Review notes on ${name} (${path}). Address each, then show me the revised file:`];
  ordered.forEach((a, i) => {
    const where = a.section ? ` [§ ${clip(a.section)}]` : "";
    const q = `"${clip(a.quote)}"`;
    const gone = a.orphaned ? " (no longer in the file)" : "";
    let what: string;
    switch (a.kind) {
      case "delete":
        what = `delete this.${a.note ? ` ${clip(a.note)}` : ""}`;
        break;
      case "replace":
        what = `replace with "${clip(a.replacement ?? "")}".${a.note ? ` ${clip(a.note)}` : ""}`;
        break;
      default:
        what = clip(a.note ?? "");
    }
    lines.push(`${i + 1}.${where} ${q}${gone} → ${what}`);
  });
  const g = general.trim();
  if (g) lines.push(`Overall: ${g.replace(/\s+/g, " ")}`);
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Staging — the "never sent" invariant
// ---------------------------------------------------------------------------

const PASTE_START = "\x1b[200~";
const PASTE_END = "\x1b[201~";

/**
 * The bytes to type into a pty so `text` lands in the TUI's input WITHOUT submitting.
 *
 * - Every C0/C1 control character except newline and tab is stripped first, so nothing in
 *   the text (an ESC, a `\x1b[201~` inside a note) can end a paste early or drive
 *   the TUI.
 * - With bracketed paste enabled (claude and codex both enable it), the text goes
 *   inside a paste bracket: newlines are paste content, not Enter.
 * - Without it, any CR/LF would submit — so the text is flattened to one line.
 *
 * No path through this function emits a bare CR or LF outside a paste bracket.
 */
export function stagedPayload(text: string, bracketedPaste: boolean): string {
  // eslint-disable-next-line no-control-regex
  const clean = text.replace(/\r\n?/g, "\n").replace(/[\x00-\x08\x0b-\x1f\x7f-\x9f]/g, "");
  if (bracketedPaste) return PASTE_START + clean.replace(/\n/g, "\r") + PASTE_END;
  return clean.replace(/\s*\n\s*/g, " / ").trim();
}
