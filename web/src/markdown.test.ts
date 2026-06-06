// Tests for the pure markdown AST — the parser that turns Claude's prose into the
// block/inline tree renderMarkdown() paints. Pure data in, pure data out (no DOM),
// so it earns real tests. The DOM layer (renderMarkdown) is verified in-app.
// Run: `node --test` (native TS). This is the swappable seam — a future remark
// pipeline would produce an equivalent tree and the DOM layer wouldn't change.
import { test } from "node:test";
import assert from "node:assert/strict";
import { inline, parseBlocks, type Inline, type Block } from "./markdown.ts";

// ── inline ─────────────────────────────────────────────────────────────────
test("inline: plain text is a single text node", () => {
  assert.deepEqual(inline("hello world"), [{ type: "text", value: "hello world" }]);
});

test("inline: **bold** becomes a strong node", () => {
  assert.deepEqual(inline("a **bold** b"), [
    { type: "text", value: "a " },
    { type: "strong", children: [{ type: "text", value: "bold" }] },
    { type: "text", value: " b" },
  ]);
});

test("inline: *italic* becomes an em node", () => {
  assert.deepEqual(inline("an *italic* word"), [
    { type: "text", value: "an " },
    { type: "em", children: [{ type: "text", value: "italic" }] },
    { type: "text", value: " word" },
  ]);
});

test("inline: `code` becomes a literal code node (no nested parsing)", () => {
  assert.deepEqual(inline("call `foo(**x**)` now"), [
    { type: "text", value: "call " },
    { type: "code", value: "foo(**x**)" },
    { type: "text", value: " now" },
  ]);
});

test("inline: ~~strike~~ becomes a strike node", () => {
  assert.deepEqual(inline("~~gone~~"), [
    { type: "strike", children: [{ type: "text", value: "gone" }] },
  ]);
});

test("inline: bold containing italic nests", () => {
  assert.deepEqual(inline("**bold *and italic***"), [
    {
      type: "strong",
      children: [
        { type: "text", value: "bold " },
        { type: "em", children: [{ type: "text", value: "and italic" }] },
      ],
    },
  ]);
});

test("inline: a lone unmatched * is literal text, never throws", () => {
  assert.deepEqual(inline("2 * 3 = 6 and a lone *"), [
    { type: "text", value: "2 * 3 = 6 and a lone *" },
  ]);
});

test("inline: snake_case identifiers are NOT emphasized (code-context safety)", () => {
  assert.deepEqual(inline("call some_function_name(x)"), [
    { type: "text", value: "call some_function_name(x)" },
  ]);
});

test("inline: spaced asterisks (a * b) are literal, not emphasis (flanking)", () => {
  assert.deepEqual(inline("a * b * c"), [{ type: "text", value: "a * b * c" }]);
});

test("inline: empty string yields no nodes", () => {
  assert.deepEqual(inline(""), []);
});

// ── blocks ──────────────────────────────────────────────────────────────────
test("parseBlocks: a single line is one paragraph", () => {
  assert.deepEqual(parseBlocks("just a line"), [
    { type: "p", spans: [{ type: "text", value: "just a line" }] },
  ]);
});

test("parseBlocks: a blank line separates two paragraphs", () => {
  const blocks = parseBlocks("first para\n\nsecond para");
  assert.deepEqual(blocks, [
    { type: "p", spans: [{ type: "text", value: "first para" }] },
    { type: "p", spans: [{ type: "text", value: "second para" }] },
  ]);
});

test("parseBlocks: a soft newline stays inside one paragraph", () => {
  assert.deepEqual(parseBlocks("line one\nline two"), [
    { type: "p", spans: [{ type: "text", value: "line one\nline two" }] },
  ]);
});

test("parseBlocks: ## heading becomes a heading block with level", () => {
  assert.deepEqual(parseBlocks("## Section"), [
    { type: "heading", level: 2, spans: [{ type: "text", value: "Section" }] },
  ]);
});

test("parseBlocks: heading runs inline parsing on its text", () => {
  assert.deepEqual(parseBlocks("### A **bold** head"), [
    {
      type: "heading",
      level: 3,
      spans: [
        { type: "text", value: "A " },
        { type: "strong", children: [{ type: "text", value: "bold" }] },
        { type: "text", value: " head" },
      ],
    },
  ]);
});

test("parseBlocks: consecutive dash lines become one bullet list", () => {
  assert.deepEqual(parseBlocks("- one\n- two\n- three"), [
    {
      type: "ul",
      items: [
        [{ type: "text", value: "one" }],
        [{ type: "text", value: "two" }],
        [{ type: "text", value: "three" }],
      ],
    },
  ]);
});

test("parseBlocks: *, +, and - all open bullet lists", () => {
  assert.deepEqual(parseBlocks("* a\n+ b"), [
    {
      type: "ul",
      items: [
        [{ type: "text", value: "a" }],
        [{ type: "text", value: "b" }],
      ],
    },
  ]);
});

test("parseBlocks: numbered lines become an ordered list carrying its start", () => {
  assert.deepEqual(parseBlocks("3. third\n4. fourth"), [
    {
      type: "ol",
      start: 3,
      items: [
        [{ type: "text", value: "third" }],
        [{ type: "text", value: "fourth" }],
      ],
    },
  ]);
});

test("parseBlocks: list items run inline parsing", () => {
  assert.deepEqual(parseBlocks("- a `code` item"), [
    {
      type: "ul",
      items: [[
        { type: "text", value: "a " },
        { type: "code", value: "code" },
        { type: "text", value: " item" },
      ]],
    },
  ]);
});

test("parseBlocks: a fenced block captures literal lines and the lang", () => {
  assert.deepEqual(parseBlocks("```ts\nconst x = **1**;\n```"), [
    { type: "code", lang: "ts", value: "const x = **1**;" },
  ]);
});

test("parseBlocks: fence content keeps markdown literal and preserves blank lines", () => {
  assert.deepEqual(parseBlocks("```\na\n\nb\n```"), [
    { type: "code", lang: "", value: "a\n\nb" },
  ]);
});

test("parseBlocks: a paragraph, a list, and a heading parse as three blocks", () => {
  const blocks = parseBlocks("Intro line\n\n- item one\n- item two\n\n## End");
  assert.equal(blocks.length, 3);
  assert.equal(blocks[0].type, "p");
  assert.equal(blocks[1].type, "ul");
  assert.equal(blocks[2].type, "heading");
});

test("parseBlocks: empty input yields no blocks", () => {
  assert.deepEqual(parseBlocks(""), []);
});
