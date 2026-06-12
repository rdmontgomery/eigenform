// Tests for toolview.ts — pure per-type tool presentation.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { toolView, miniDiff, toolsSummary } from "./toolview.ts";
import type { Tool } from "./turns.ts";

function tool(overrides: Partial<Tool> & { kind: string }): Tool {
  return { arg: "", delta: "", ...overrides };
}

// ---------------------------------------------------------------------------
// kind → type mapping
// ---------------------------------------------------------------------------

test("toolView: maps real kinds to types", () => {
  assert.equal(toolView(tool({ kind: "Bash" })).type, "bash");
  assert.equal(toolView(tool({ kind: "Read" })).type, "read");
  assert.equal(toolView(tool({ kind: "Edit" })).type, "edit");
  assert.equal(toolView(tool({ kind: "Write" })).type, "write");
  assert.equal(toolView(tool({ kind: "Grep" })).type, "grep");
  assert.equal(toolView(tool({ kind: "Glob" })).type, "grep");
  assert.equal(toolView(tool({ kind: "WebFetch" })).type, "fetch");
  assert.equal(toolView(tool({ kind: "WebSearch" })).type, "fetch");
  assert.equal(toolView(tool({ kind: "TodoWrite" })).type, "todo");
  assert.equal(toolView(tool({ kind: "Skill" })).type, "skill");
  assert.equal(toolView(tool({ kind: "Task" })).type, "task");
  assert.equal(toolView(tool({ kind: "TaskCreate" })).type, "task");
  assert.equal(toolView(tool({ kind: "AskUserQuestion" })).type, "other");
  assert.equal(toolView(tool({ kind: "ToolSearch" })).type, "other");
});

// ---------------------------------------------------------------------------
// headlines
// ---------------------------------------------------------------------------

test("toolView: bash headline is the command's first line", () => {
  const v = toolView(tool({
    kind: "Bash",
    arg: "long arg",
    input: { command: "npm test\necho done", description: "run tests" },
  }));
  assert.equal(v.headline, "npm test");
  assert.equal(v.mono, true);
});

test("toolView: read headline is basename plus line range from offset/limit", () => {
  const v = toolView(tool({
    kind: "Read",
    input: { file_path: "/home/u/proj/src/main.ts", offset: 10, limit: 20 },
  }));
  assert.equal(v.headline, "main.ts · lines 10–29");
});

test("toolView: read headline without offset/limit is just the basename", () => {
  const v = toolView(tool({ kind: "Read", input: { file_path: "/a/b/style.css" } }));
  assert.equal(v.headline, "style.css");
});

test("toolView: edit headline is the file basename; accessory is the diff stat", () => {
  const v = toolView(tool({
    kind: "Edit",
    input: { file_path: "/p/style.css", old_string: "a\nb", new_string: "a\nc\nd" },
  }));
  assert.equal(v.headline, "style.css");
  assert.deepEqual(v.accessory, { kind: "stat", add: 2, del: 1 });
});

test("toolView: write accessory counts all content lines as additions", () => {
  const v = toolView(tool({
    kind: "Write",
    input: { file_path: "/p/new.ts", content: "l1\nl2\nl3" },
  }));
  assert.deepEqual(v.accessory, { kind: "stat", add: 3, del: 0 });
});

test("toolView: grep headline is the pattern; accessory counts output lines", () => {
  const v = toolView(tool({
    kind: "Grep",
    input: { pattern: "foo.*bar" },
    output: "a.ts:1:foo bar\nb.ts:9:foo bar",
  }));
  assert.equal(v.headline, "foo.*bar");
  assert.deepEqual(v.accessory, { kind: "count", n: 2 });
});

test("toolView: todo headline is done/total; items map statuses", () => {
  const v = toolView(tool({
    kind: "TodoWrite",
    input: { todos: [
      { content: "one", status: "completed" },
      { content: "two", status: "in_progress" },
      { content: "three", status: "pending" },
    ] },
  }));
  assert.equal(v.headline, "1/3 complete");
  assert.equal(v.body.kind, "todos");
  if (v.body.kind === "todos") {
    assert.deepEqual(v.body.items.map((i) => i.s), ["done", "doing", "todo"]);
  }
});

test("toolView: skill headline prefers input.skill; unknown kinds fall back to arg + raw body", () => {
  assert.equal(toolView(tool({ kind: "Skill", input: { skill: "loop" }, arg: "x" })).headline, "loop");
  const other = toolView(tool({ kind: "ToolSearch", arg: "select:Foo" }));
  assert.equal(other.headline, "select:Foo");
  assert.equal(other.body.kind, "raw");
});

test("toolView: malformed input (string where object expected) degrades to arg + raw", () => {
  const v = toolView(tool({ kind: "Bash", arg: "fallback", input: "not-an-object" }));
  assert.equal(v.headline, "fallback");
  assert.equal(v.body.kind, "raw");
});

// ---------------------------------------------------------------------------
// miniDiff
// ---------------------------------------------------------------------------

test("miniDiff: trims common leading/trailing lines", () => {
  const d = miniDiff("keep\nold line\nkeep2", "keep\nnew line\nkeep2");
  assert.deepEqual(d.lines, [
    { sign: "-", text: "old line" },
    { sign: "+", text: "new line" },
  ]);
  assert.equal(d.truncated, 0);
});

test("miniDiff: pure insertion yields only additions", () => {
  const d = miniDiff("a\nb", "a\nx\nb");
  assert.deepEqual(d.lines, [{ sign: "+", text: "x" }]);
});

test("miniDiff: caps emitted lines and reports the overflow", () => {
  const oldS = Array.from({ length: 30 }, (_, i) => `o${i}`).join("\n");
  const newS = Array.from({ length: 30 }, (_, i) => `n${i}`).join("\n");
  const d = miniDiff(oldS, newS, 10);
  assert.equal(d.lines.length, 10);
  assert.equal(d.truncated, 50);
});

// ---------------------------------------------------------------------------
// toolsSummary
// ---------------------------------------------------------------------------

test("toolsSummary: unique verbs in first-seen order", () => {
  const tools = [
    tool({ kind: "Skill" }),
    tool({ kind: "Bash" }),
    tool({ kind: "Bash" }),
    tool({ kind: "Edit" }),
  ];
  assert.equal(toolsSummary(tools), "Skill · Bash · Edit");
});

test("toolsSummary: empty list yields empty string", () => {
  assert.equal(toolsSummary([]), "");
});
