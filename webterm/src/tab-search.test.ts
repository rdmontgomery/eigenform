// Tests for tab-search.ts's pure matcher.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { matchTabs, type TabSearchItem } from "./tab-search.ts";

const TABS: TabSearchItem[] = [
  { id: "a", label: "status line fit", cwd: "/home/rick/eigenform/webterm", uuid: "1111-aaaa" },
  { id: "b", label: "forest recency", cwd: "/home/rick/eigenform", ptyId: "pty-7" },
  { id: "c", label: "zsh", cwd: "/home/rick/notes/status" },
];

test("blank query matches nothing", () => {
  assert.deepEqual(matchTabs("", TABS), []);
  assert.deepEqual(matchTabs("   ", TABS), []);
});

test("case-insensitive substring over label, cwd, uuid and ptyId", () => {
  assert.deepEqual(matchTabs("FOREST", TABS), ["b"]);
  assert.deepEqual(matchTabs("webterm", TABS), ["a"]);
  assert.deepEqual(matchTabs("1111", TABS), ["a"]);
  assert.deepEqual(matchTabs("pty-7", TABS), ["b"]);
});

test("every term must match somewhere", () => {
  assert.deepEqual(matchTabs("eigenform recency", TABS), ["b"]);
  assert.deepEqual(matchTabs("eigenform nope", TABS), []);
});

test("label hits rank before cwd-only hits; ties keep overstory order", () => {
  // "status" is in a's label but only in c's cwd.
  assert.deepEqual(matchTabs("status", [TABS[2]!, TABS[0]!]), ["a", "c"]);
  assert.deepEqual(matchTabs("eigenform", TABS), ["a", "b"]);
});

test("facet terms filter by the tab's session facets", () => {
  const tabs: TabSearchItem[] = [
    { id: "a", label: "login fix", facets: { work: ["repo:me/app", "pr:me/app#57"], mention: [] } },
    { id: "b", label: "login notes", facets: { work: ["repo:me/notes"], mention: ["pr:me/app#57"] } },
    { id: "c", label: "zsh" },
  ];
  assert.deepEqual(matchTabs("pr:57", tabs), ["a"]);
  assert.deepEqual(matchTabs("~pr:57", tabs), ["a", "b"]);
  assert.deepEqual(matchTabs("notes ~#57", tabs), ["b"], "words and facets AND");
});
