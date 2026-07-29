// Tests for links.ts — pure URL extraction from chat exchanges.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { extractUrls, linkLabel } from "./links.ts";
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

test("extractUrls: dedupes http vs https as the same link", () => {
  const got = extractUrls([
    ex({ n: 1, user: "http://example.com/a" }),
    ex({ n: 2, assistant: "https://example.com/a" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "http://example.com/a", "keeps the first-seen variant verbatim");
  assert.equal(got[0]!.turnNumber, 1);
});

test("extractUrls: dedupes case-insensitive host", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://GitHub.com/foo/bar" }),
    ex({ n: 2, assistant: "https://github.com/foo/bar" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "https://GitHub.com/foo/bar");
});

test("extractUrls: dedupes a trailing slash on the path", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/foo" }),
    ex({ n: 2, assistant: "https://example.com/foo/" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "https://example.com/foo");
});

test("extractUrls: dedupes an explicit default port", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com:443/foo" }),
    ex({ n: 2, assistant: "https://example.com/foo" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "https://example.com:443/foo");
});

test("extractUrls: does NOT dedupe a root path's slash (nothing to strip)", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/" }),
    ex({ n: 2, assistant: "https://example.com" }),
  ]);
  assert.equal(got.length, 1, "both normalize to the same empty/root path");
});

test("extractUrls: does NOT dedupe differing query strings", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/x?tab=a" }),
    ex({ n: 2, assistant: "https://example.com/x?tab=b" }),
  ]);
  assert.equal(got.length, 2);
});

test("extractUrls: does NOT dedupe differing fragments", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/pull/22#issue-1" }),
    ex({ n: 2, assistant: "https://example.com/pull/22#issuecomment-2" }),
  ]);
  assert.equal(got.length, 2);
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

// ---------------------------------------------------------------------------
// linkLabel — rail display label, truncated to keep the differentiating tail
// ---------------------------------------------------------------------------

test("linkLabel: short host+path fits untouched", () => {
  assert.equal(linkLabel("https://example.com/foo"), "example.com/foo");
});

test("linkLabel: bare host (root path) has no trailing slash", () => {
  assert.equal(linkLabel("https://example.com/"), "example.com");
  assert.equal(linkLabel("https://example.com"), "example.com");
});

test("linkLabel: sibling PR links stay distinguishable (the bug this fixes)", () => {
  const labels = [22, 23, 24, 25].map((n) =>
    linkLabel(`https://github.com/rdmontgomery/eigenform/pull/${n}`),
  );
  // All four must render to DIFFERENT strings — a shared long prefix
  // ("github.com/rdmontgomery/eigenform/pull/") must not swallow the part
  // that actually differs (the PR number).
  assert.equal(new Set(labels).size, 4, `expected 4 distinct labels, got ${JSON.stringify(labels)}`);
  for (const label of labels) {
    assert.ok(label.length <= 36, `label too long: ${label}`);
  }
});

test("linkLabel: truncates from the front, keeping whole trailing path segments", () => {
  const label = linkLabel("https://github.com/rdmontgomery/eigenform/pull/22");
  assert.equal(label, "github.com…/eigenform/pull/22");
});

test("linkLabel: preserves the host even when the path must truncate", () => {
  const label = linkLabel("https://github.com/rdmontgomery/eigenform/pull/22");
  assert.ok(label.startsWith("github.com"), `expected host kept, got ${label}`);
});

test("linkLabel: a single path segment longer than the whole budget falls back to raw tail", () => {
  const url = `https://example.com/${"x".repeat(60)}`;
  const label = linkLabel(url);
  assert.ok(label.startsWith("example.com…"));
  assert.ok(label.length <= 36);
});

test("linkLabel: query string and fragment count toward the path length", () => {
  const label = linkLabel("https://example.com/pull/22?tab=files#issuecomment-99999999");
  assert.equal(label, "example.com…es#issuecomment-99999999");
  assert.ok(label.length <= 36);
});
