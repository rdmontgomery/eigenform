// Tests for links.ts — pure URL extraction from chat exchanges.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { extractUrls } from "./links.ts";
import type { Exchange } from "./turns.ts";

function ex(overrides: Partial<Exchange> & { n: number }): Exchange {
  return { user: "", ...overrides };
}

test("extractUrls: finds a URL in user text", () => {
  const got = extractUrls([ex({ n: 1, user: "check https://example.com/foo please" })]);
  assert.deepEqual(got, [{ url: "https://example.com/foo", turnNumber: 1, role: "user" }]);
});

test("extractUrls: finds a URL in assistant text", () => {
  const got = extractUrls([ex({ n: 1, assistant: "see https://docs.rs/tokio for details" })]);
  assert.deepEqual(got, [{ url: "https://docs.rs/tokio", turnNumber: 1, role: "assistant" }]);
});

test("extractUrls: strips trailing sentence punctuation", () => {
  const got = extractUrls([ex({ n: 1, user: "Link: https://example.com/x." })]);
  assert.equal(got[0]!.url, "https://example.com/x");
});

test("extractUrls: strips trailing comma and parenthesis (unbalanced)", () => {
  const got = extractUrls([
    ex({ n: 1, user: "see https://example.com/a, and (https://example.com/b)" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), ["https://example.com/a", "https://example.com/b"]);
});

test("extractUrls: keeps a balanced closing paren that's part of the URL", () => {
  const got = extractUrls([
    ex({ n: 1, user: "wiki: https://en.wikipedia.org/wiki/Foo_(bar)" }),
  ]);
  assert.equal(got[0]!.url, "https://en.wikipedia.org/wiki/Foo_(bar)");
});

test("extractUrls: dedupes by URL, keeping the earliest turn", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/dup" }),
    ex({ n: 2, assistant: "https://example.com/dup again" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.turnNumber, 1);
  assert.equal(got[0]!.role, "user");
});

test("extractUrls: preserves order of first appearance across exchanges", () => {
  const got = extractUrls([
    ex({ n: 1, user: "first https://a.example.com" }),
    ex({ n: 2, assistant: "then https://b.example.com" }),
    ex({ n: 3, user: "then https://c.example.com" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), [
    "https://a.example.com",
    "https://b.example.com",
    "https://c.example.com",
  ]);
});

test("extractUrls: multiple URLs in one message", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://one.example.com and https://two.example.com" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), ["https://one.example.com", "https://two.example.com"]);
});

test("extractUrls: ignores tool input/output entirely", () => {
  const got = extractUrls([
    ex({
      n: 1,
      user: "no link here",
      tool: { kind: "Bash", arg: "curl", delta: "", output: "https://leaked.example.com/from-output" },
    }),
  ]);
  assert.deepEqual(got, []);
});

test("extractUrls: no exchanges → empty list", () => {
  assert.deepEqual(extractUrls([]), []);
});

test("extractUrls: no URLs present → empty list", () => {
  const got = extractUrls([ex({ n: 1, user: "no links at all" })]);
  assert.deepEqual(got, []);
});
