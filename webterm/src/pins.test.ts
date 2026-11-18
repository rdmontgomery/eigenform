// Tests for pins.ts's pure split.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { splitPinned } from "./pins.ts";

const A = "0000aaaa-0000-4000-8000-000000000001";
const B = "0000bbbb-0000-4000-8000-000000000002";
const C = "0000cccc-0000-4000-8000-000000000003";

const rows = [
  { name: "a", uuid: A },
  { name: "fresh" },
  { name: "b", uuid: B },
  { name: "c", uuid: C },
];
const names = (rs: { name: string }[]) => rs.map((r) => r.name);

test("pinned rows come out in pin order, the rest keep theirs", () => {
  const { pinned, rest } = splitPinned(rows, [C, A]);
  assert.deepEqual(names(pinned), ["c", "a"]);
  assert.deepEqual(names(rest), ["fresh", "b"]);
});

test("no pins leaves every row where it was", () => {
  const { pinned, rest } = splitPinned(rows, []);
  assert.deepEqual(pinned, []);
  assert.deepEqual(names(rest), ["a", "fresh", "b", "c"]);
});

test("a pin whose session isn't in the roster shows nothing", () => {
  const { pinned } = splitPinned(rows, ["0000dddd-0000-4000-8000-000000000004", B]);
  assert.deepEqual(names(pinned), ["b"]);
});
