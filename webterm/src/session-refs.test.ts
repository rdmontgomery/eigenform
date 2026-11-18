// Tests for session-refs.ts: which words in terminal text link to a session.
// Run: `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { buildRefIndex, findSessionRefs } from "./session-refs.ts";

const A = "0000aaaa-1111-4000-8000-000000000001";
const B = "0000aaaa-2222-4000-8000-000000000002";
const C = "1234cdef-0000-4000-8000-000000000003";
const D = "5678abcd-0000-4000-8000-000000000004";

const index = buildRefIndex([
  { uuid: A, peer: "app-dd" },
  { uuid: B },
  { uuid: C, peer: "web-ui" },
  { uuid: C, peer: "web-ui" }, // a live pty and its transcript: one session
  { uuid: D, peer: "twin" },
  { uuid: "9999eeee-0000-4000-8000-000000000005", peer: "Twin" },
]);

const refs = (text: string) => findSessionRefs(text, index).map((m) => [m.text, m.uuid]);

test("a peer name links, ignoring case", () => {
  assert.deepEqual(refs("ask app-dd about it"), [["app-dd", A]]);
  assert.deepEqual(refs("ask APP-DD about it"), [["APP-DD", A]]);
});

test("a peer name shared by one session's two rows still links", () => {
  assert.deepEqual(refs("web-ui is done"), [["web-ui", C]]);
});

test("an 8+ hex id prefix links; a shorter one does not", () => {
  assert.deepEqual(refs("see 1234cdef for details"), [["1234cdef", C]]);
  assert.deepEqual(refs("see 1234cdef-0000 for details"), [["1234cdef-0000", C]]);
  assert.deepEqual(refs("see 1234cde for details"), []);
});

test("a full uuid links", () => {
  assert.deepEqual(refs(`session ${A}.`), [[A, A]]);
});

test("ambiguous refs are not linked", () => {
  // two sessions start 0000aaaa
  assert.deepEqual(refs("0000aaaa is busy"), []);
  // two sessions are named twin (case-insensitively)
  assert.deepEqual(refs("ping twin"), []);
  // but a longer prefix that splits them does link
  assert.deepEqual(refs("0000aaaa-2222 is busy"), [["0000aaaa-2222", B]]);
});

test("unknown refs are not linked", () => {
  assert.deepEqual(refs("deadbeefcafe and some-other-name"), []);
  assert.deepEqual(refs("0000aaaa-1111-4000-8000-00000000ffff"), []);
});

test("trailing punctuation is not part of the link", () => {
  const [m] = findSessionRefs("handed off to app-dd.", index);
  assert.equal(m?.text, "app-dd");
  assert.equal(m?.start, 14);
  assert.equal(m?.end, 20);
  assert.deepEqual(refs("(app-dd), web-ui: done"), [
    ["app-dd", A],
    ["web-ui", C],
  ]);
});

test("names inside paths, URLs and addresses are not links", () => {
  assert.deepEqual(refs("~/work/app-dd/src"), []);
  assert.deepEqual(refs("https://example.com/app-dd"), []);
  assert.deepEqual(refs("app-dd:8080"), []);
  assert.deepEqual(refs("user@app-dd"), []);
  assert.deepEqual(refs("--name=app-dd"), []);
});

test("several refs on one line, in order", () => {
  const ms = findSessionRefs("app-dd → web-ui → 5678abcd", index);
  assert.deepEqual(
    ms.map((m) => m.uuid),
    [A, C, D],
  );
});

test("an empty index links nothing", () => {
  assert.deepEqual(findSessionRefs("app-dd 1234cdef", buildRefIndex([])), []);
});
