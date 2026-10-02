// Tests for annotate.ts — plan annotation core.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { mdBlocks, reanchor, segments, compileFeedback, stagedPayload } from "./annotate.ts";
import type { Annotation } from "./annotate.ts";

const PLAN = `# Retry fix

Replace the fixed sleep with
exponential backoff.

## Steps

1. Add a backoff schedule
   capped at 30s
2. Delete the old sleep

\`\`\`rust
sleep(10);
\`\`\`

> risky: touches the hot path

| step | owner |
|-|-|
| 1 | codex |

---
`;

test("mdBlocks: headings, joined paragraphs, list continuations, code, quotes, tables", () => {
  const b = mdBlocks(PLAN);
  assert.deepEqual(
    b.map((x) => x.type),
    ["h1", "p", "h2", "li", "li", "code", "quote", "table"],
  );
  assert.equal(b[1]!.text, "Replace the fixed sleep with exponential backoff.");
  assert.equal(b[3]!.text, "Add a backoff schedule capped at 30s");
  assert.equal(b[3]!.section, "Steps");
  assert.equal(b[3]!.marker, "1.");
  assert.equal(b[5]!.text, "sleep(10);");
  assert.equal(b[7]!.text.split("\n").length, 3);
});

function ann(o: Partial<Annotation> & { quote: string; block: number; start: number }): Annotation {
  return { id: o.quote, kind: "comment", section: "", ...o };
}

test("reanchor: follows a quote that moved, orphans one that vanished", () => {
  const before = mdBlocks(PLAN);
  const a = ann({ quote: "capped at 30s", block: 3, start: before[3]!.text.indexOf("capped at 30s") });
  const b = ann({ quote: "Delete the old sleep", block: 4, start: 0 });
  const revised = mdBlocks(PLAN.replace("# Retry fix\n", "# Retry fix\n\nContext first.\n").replace("2. Delete the old sleep\n", ""));
  const [ra, rb] = reanchor([a, b], revised);
  assert.equal(ra!.orphaned, false);
  assert.equal(revised[ra!.block]!.text.substr(ra!.start, ra!.quote.length), "capped at 30s");
  assert.equal(rb!.orphaned, true);
});

test("segments: cuts a block into plain and marked runs, dropping overlaps", () => {
  const text = "Replace the fixed sleep with exponential backoff.";
  const m1 = ann({ quote: "fixed sleep", block: 0, start: 12 });
  const m2 = ann({ quote: "sleep with", block: 0, start: 18 }); // overlaps m1
  const segs = segments(text, [m2, m1]);
  assert.deepEqual(segs.map((s) => [s.text, s.ann?.quote ?? null]), [
    ["Replace the ", null],
    ["fixed sleep", "fixed sleep"],
    [" with exponential backoff.", null],
  ]);
  assert.equal(segs.map((s) => s.text).join(""), text);
});

test("compileFeedback: document order, kinds, section, orphans last, overall note", () => {
  const out = compileFeedback(
    "/w/plan.md",
    [
      ann({ quote: "Delete the old sleep", block: 4, start: 0, kind: "replace", replacement: "Keep the sleep as a floor", section: "Steps" }),
      ann({ quote: "gone text", block: 0, start: 0, kind: "delete", orphaned: true }),
      ann({ quote: "fixed sleep", block: 1, start: 12, note: "why not jitter?", section: "Retry fix" }),
    ],
    "  good direction\n otherwise ",
  );
  const lines = out.split("\n");
  assert.match(lines[0]!, /plan\.md \(\/w\/plan\.md\)/);
  assert.equal(lines[1], '1. [§ Retry fix] "fixed sleep" → why not jitter?');
  assert.equal(lines[2], '2. [§ Steps] "Delete the old sleep" → replace with "Keep the sleep as a floor".');
  assert.equal(lines[3], '3. "gone text" (no longer in the file) → delete this.');
  assert.equal(lines[4], "Overall: good direction otherwise");
});

// The "never sent" invariant: no CR/LF may reach the pty outside a paste bracket, and
// nothing in the text can close the bracket early.
test("stagedPayload: bracketed paste wraps multi-line text; newlines stay inside", () => {
  const p = stagedPayload("a\nb\r\nc", true);
  assert.equal(p, "\x1b[200~a\rb\rc\x1b[201~");
});

test("stagedPayload: an embedded paste-end or ESC can't break out of the bracket", () => {
  const p = stagedPayload("note \x1b[201~\rrm -rf ~\n", true);
  const inner = p.slice("\x1b[200~".length, -"\x1b[201~".length);
  assert.ok(!inner.includes("\x1b"), "no ESC inside the bracket");
  assert.equal(p.indexOf("\x1b[201~"), p.length - "\x1b[201~".length, "only the final paste-end");
});

test("stagedPayload: without bracketed paste, flattened to one line — no CR/LF at all", () => {
  const p = stagedPayload("line one\nline two\r\nthree\x9b31m", false);
  assert.ok(!/[\r\n]/.test(p));
  assert.ok(!/[\x00-\x1f\x7f-\x9f]/.test(p));
  assert.equal(p, "line one / line two / three31m");
});
