import { test } from "node:test";
import assert from "node:assert/strict";
import { groupByDirectory, orderByLastPrompt, tabGroupKey } from "./shell-helpers.ts";

test("tabGroupKey: a repo under src and its worktrees share a key", () => {
  assert.equal(tabGroupKey("/Users/me/src/app"), "/Users/me/src/app");
  assert.equal(tabGroupKey("/Users/me/src/app/feat-x"), "/Users/me/src/app");
  assert.equal(tabGroupKey("/Users/me/src/app/main/src/lib"), "/Users/me/src/app");
  assert.equal(tabGroupKey("/Users/me/src"), "/Users/me/src");
  assert.equal(tabGroupKey("/Users/me/projects/tool/"), "/Users/me/projects/tool");
  assert.equal(tabGroupKey(undefined), "");
});

test("orderByLastPrompt: recent or stale first, unknown times last in place", () => {
  const items = [
    { id: "a", lastPromptAt: "2026-11-13T10:00:00Z" },
    { id: "none1" },
    { id: "b", lastPromptAt: "2026-11-13T12:00:00Z" },
    { id: "none2", lastPromptAt: null },
    { id: "c", lastPromptAt: "2026-11-13T11:00:00+00:00" },
  ];
  assert.deepEqual(orderByLastPrompt(items, "recent"), ["b", "c", "a", "none1", "none2"]);
  assert.deepEqual(orderByLastPrompt(items, "stale"), ["a", "c", "b", "none1", "none2"]);
});

test("groupByDirectory: groups by first appearance, tabs keep their order", () => {
  const items = [
    { id: "1", cwd: "/u/src/app/feat-x" },
    { id: "2", cwd: "/u/notes" },
    { id: "3", cwd: "/u/src/app" },
    { id: "4" },
    { id: "5", cwd: "/u/notes" },
    { id: "6", cwd: "/u/src/app/main" },
  ];
  assert.deepEqual(groupByDirectory(items), ["1", "3", "6", "2", "5", "4"]);
});
