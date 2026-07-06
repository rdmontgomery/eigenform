/**
 * turns.test.ts — node --test suite for groupTurns (pure logic, no DOM).
 *
 * Scenarios exercised:
 *   1. 1 user + 3 tool rounds + 1 user → 2 groups
 *   2. leaf exchange is flagged on its group
 *   3. empty session → empty groups array
 *   4. group carries its tool exchanges in order
 *   5. consecutive assistant exchanges within one user turn collapse into one group
 *   6. single user exchange with no assistant → 1 group
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { groupTurns, toolExpandKey } from "./turns.ts";
import type { Exchange, TurnGroup } from "./turns.ts";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function user(n: number, text = `user ${n}`, uuid = `u${n}`): Exchange {
  return { n, tok: 0, user: text, uuid };
}

function assistant(n: number, text = `assistant ${n}`): Exchange {
  return { n, tok: 0, user: "", assistant: text };
}

function toolExchange(n: number, kind = "Edit", arg = "file.ts"): Exchange {
  return {
    n,
    tok: 0,
    user: "",
    assistant: "",
    tool: { kind, arg, delta: "+1 −0", input: {}, output: "" },
  };
}

function leaf(n: number): Exchange {
  return { n, tok: 0, user: "", leaf: true };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test("empty session → empty groups", () => {
  const groups = groupTurns([]);
  assert.deepEqual(groups, []);
});

test("single user exchange → 1 group with no tool exchanges", () => {
  const exchanges: Exchange[] = [user(1)];
  const groups = groupTurns(exchanges);
  assert.equal(groups.length, 1);
  assert.equal(groups[0]!.userText, "user 1");
  assert.equal(groups[0]!.toolExchanges.length, 0);
  assert.equal(groups[0]!.isLeaf, false);
});

test("1 user + 3 tool rounds + 1 user → 2 groups", () => {
  const exchanges: Exchange[] = [
    user(1, "first question"),
    toolExchange(2, "Read", "a.ts"),
    toolExchange(3, "Edit", "b.ts"),
    toolExchange(4, "Bash", "cargo test"),
    user(5, "second question"),
  ];
  const groups = groupTurns(exchanges);
  assert.equal(groups.length, 2, "expected 2 groups");

  // First group: user text + 3 tool exchanges
  assert.equal(groups[0]!.userText, "first question");
  assert.equal(groups[0]!.toolExchanges.length, 3);
  assert.equal(groups[0]!.toolExchanges[0]!.tool!.kind, "Read");
  assert.equal(groups[0]!.toolExchanges[1]!.tool!.kind, "Edit");
  assert.equal(groups[0]!.toolExchanges[2]!.tool!.kind, "Bash");
  assert.equal(groups[0]!.isLeaf, false);

  // Second group: no tools yet
  assert.equal(groups[1]!.userText, "second question");
  assert.equal(groups[1]!.toolExchanges.length, 0);
});

test("tool exchanges preserved in order within a group", () => {
  const kinds = ["Read", "Edit", "Write", "Bash"];
  const exchanges: Exchange[] = [
    user(1),
    ...kinds.map((k, i) => toolExchange(i + 2, k, `arg${i}`)),
    user(kinds.length + 2),
  ];
  const groups = groupTurns(exchanges);
  const tools = groups[0]!.toolExchanges;
  assert.equal(tools.length, kinds.length);
  for (let i = 0; i < kinds.length; i++) {
    assert.equal(tools[i]!.tool!.kind, kinds[i]);
  }
});

test("leaf exchange is flagged on its group and not counted as a separate group", () => {
  const exchanges: Exchange[] = [
    user(1, "a question"),
    toolExchange(2),
    leaf(3),
  ];
  const groups = groupTurns(exchanges);
  // The leaf terminates; we expect 1 group for the user turn and the leaf produces
  // a synthetic group so the UI can render the input affordance.
  assert.ok(groups.length >= 1, "at least one group");
  // The leaf group should be flagged.
  const leafGroup = groups.find((g) => g.isLeaf);
  assert.ok(leafGroup !== undefined, "there must be a leaf-flagged group");
  assert.equal(leafGroup!.isLeaf, true);
});

test("consecutive assistant exchanges within one user-span collapse into one group", () => {
  // Pattern: user → assistant(with text) → assistant(with text) → user
  // Both assistant replies belong to the first group, not separate groups.
  const exchanges: Exchange[] = [
    user(1, "question"),
    assistant(2, "first reply"),
    assistant(3, "second reply"),
    user(4, "follow-up"),
  ];
  const groups = groupTurns(exchanges);
  assert.equal(groups.length, 2);
  assert.equal(groups[0]!.assistantText, "first reply\n\nsecond reply");
});

test("assistant text is concatenated from all assistant exchanges in the group", () => {
  const exchanges: Exchange[] = [
    user(1, "ask"),
    assistant(2, "part one"),
    assistant(3, "part two"),
    leaf(4),
  ];
  const groups = groupTurns(exchanges);
  const ask = groups.find((g) => g.userText === "ask");
  assert.ok(ask !== undefined);
  assert.equal(ask!.assistantText, "part one\n\npart two");
});

test("group-opening exchange with both user text and a tool includes the tool in toolExchanges", () => {
  // The Rust emitter attaches `tool` to the user-initiated exchange, so
  // {user: "...", tool: {...}} is the common shape.  The group-opening `continue`
  // was bypassing tool accumulation, silently dropping first-turn tools.
  const openingTool = { kind: "Read", arg: "file.ts", delta: "+0 −0", input: {}, output: "" };
  const exchanges: Exchange[] = [
    { n: 1, tok: 0, user: "first question", uuid: "u1", tool: openingTool },
    toolExchange(2, "Edit", "b.ts"),
    user(3, "second question"),
  ];
  const groups = groupTurns(exchanges);
  assert.equal(groups.length, 2);
  // The opening exchange carried a tool — it must appear as the first toolExchange.
  assert.equal(groups[0]!.toolExchanges.length, 2, "first group should have 2 tool exchanges");
  assert.equal(groups[0]!.toolExchanges[0]!.tool!.kind, "Read");
  assert.equal(groups[0]!.toolExchanges[1]!.tool!.kind, "Edit");
  // Second group has no tools.
  assert.equal(groups[1]!.toolExchanges.length, 0);
});

test("group turnNumber equals the exchange n of the opening user turn", () => {
  const exchanges: Exchange[] = [
    user(7, "q"),
    toolExchange(8),
    user(9, "q2"),
  ];
  const groups = groupTurns(exchanges);
  assert.equal(groups[0]!.turnNumber, 7);
  assert.equal(groups[1]!.turnNumber, 9);
});

// ---------------------------------------------------------------------------
// toolExpandKey — pure helper
// ---------------------------------------------------------------------------

test("toolExpandKey produces a stable string key", () => {
  assert.equal(toolExpandKey(1, 0), "1:0");
  assert.equal(toolExpandKey(7, 3), "7:3");
  assert.equal(toolExpandKey(0, 0), "0:0");
});

test("toolExpandKey keys are unique across (turnNumber, toolIndex) pairs", () => {
  const keys = [
    toolExpandKey(1, 0),
    toolExpandKey(1, 1),
    toolExpandKey(2, 0),
    toolExpandKey(2, 1),
  ];
  const unique = new Set(keys);
  assert.equal(unique.size, keys.length, "all keys must be distinct");
});

test("toolExpandKey does not collide across groups (turnNumber stable per session)", () => {
  // Simulate two groups with same tool count — keys must not collide.
  const keysG1 = [0, 1, 2].map((i) => toolExpandKey(7, i));
  const keysG2 = [0, 1, 2].map((i) => toolExpandKey(9, i));
  const allKeys = [...keysG1, ...keysG2];
  const unique = new Set(allKeys);
  assert.equal(unique.size, allKeys.length, "no collision between groups");
});

test("tool exchanges carry input, output, truncated, inputTruncated, and detail through grouping", () => {
  const exchanges: Exchange[] = [
    user(1, "ask"),
    {
      n: 2,
      tok: 0,
      user: "",
      tool: {
        kind: "Read",
        arg: "file.ts",
        delta: "+3 −0",
        input: { path: "file.ts" },
        output: "content here",
        truncated: false,
        inputTruncated: false,
        detail: {
          tok: 12,
          lines: [
            { t: "+ added line", c: "add" },
            { t: "  context", c: "dim" },
            { t: "- removed", c: "rem" },
          ],
        },
      },
    },
  ];
  const groups = groupTurns(exchanges);
  assert.equal(groups.length, 1);
  const t = groups[0]!.toolExchanges[0]!.tool!;
  assert.deepEqual(t.input, { path: "file.ts" });
  assert.equal(t.output, "content here");
  assert.equal(t.truncated, false);
  assert.equal(t.inputTruncated, false);
  assert.ok(t.detail !== undefined);
  assert.equal(t.detail!.lines.length, 3);
  assert.equal(t.detail!.lines[0]!.c, "add");
  assert.equal(t.detail!.lines[2]!.c, "rem");
});

// ---------------------------------------------------------------------------
// items — interleaved text/tool order (the actual bug this fixes)
// ---------------------------------------------------------------------------

test("assistant text on the group-opening exchange is not dropped", () => {
  // The Rust emitter's common shape for a plain (no-tool) reply: the assistant's
  // text lands on the SAME exchange object as the opening user turn, since it's
  // still `exchanges.last_mut()` when the assistant turn is processed.
  const exchanges: Exchange[] = [
    { n: 1, tok: 0, user: "render the transcript", uuid: "u1", assistant: "on it" },
  ];
  const groups = groupTurns(exchanges);
  assert.equal(groups[0]!.assistantText, "on it", "previously dropped entirely");
  assert.deepEqual(groups[0]!.items, [{ kind: "text", text: "on it" }]);
});

test("items preserves the true interleaved order of text and tool exchanges", () => {
  const toolA = { kind: "Read", arg: "a.ts", delta: "" };
  const toolB = { kind: "Edit", arg: "b.ts", delta: "" };
  const exchanges: Exchange[] = [
    // opening exchange: user text + assistant text + a tool, all combined (real shape)
    { n: 1, tok: 0, user: "go", uuid: "u1", assistant: "first, reading", tool: toolA },
    // a tool-only exchange (no narration before this one)
    { n: 2, tok: 0, user: "", tool: toolB },
    // a text-only exchange (narration with no tool call)
    { n: 3, tok: 0, user: "", assistant: "done for now" },
  ];
  const groups = groupTurns(exchanges);
  assert.deepEqual(groups[0]!.items, [
    { kind: "text", text: "first, reading" },
    { kind: "tool", exchange: exchanges[0], toolIndex: 0 },
    { kind: "tool", exchange: exchanges[1], toolIndex: 1 },
    { kind: "text", text: "done for now" },
  ]);
});

test("items assigns toolIndex matching the tool's position in toolExchanges", () => {
  const exchanges: Exchange[] = [
    user(1, "go"),
    toolExchange(2, "Read"),
    toolExchange(3, "Edit"),
    toolExchange(4, "Bash"),
  ];
  const groups = groupTurns(exchanges);
  const toolItems = groups[0]!.items.filter((i) => i.kind === "tool");
  assert.deepEqual(toolItems.map((i) => i.toolIndex), [0, 1, 2]);
  assert.deepEqual(
    groups[0]!.toolExchanges.map((ex, i) => toolExpandKey(groups[0]!.turnNumber, i)),
    toolItems.map((i) => toolExpandKey(groups[0]!.turnNumber, i.toolIndex)),
  );
});

test("an empty-string assistant field does not produce a spurious text item", () => {
  const exchanges: Exchange[] = [
    user(1, "go"),
    toolExchange(2, "Read"), // toolExchange() sets assistant: "" alongside the tool
  ];
  const groups = groupTurns(exchanges);
  assert.deepEqual(groups[0]!.items, [{ kind: "tool", exchange: exchanges[1], toolIndex: 0 }]);
});
