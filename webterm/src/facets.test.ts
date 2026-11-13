// Tests for facets.ts: the search grammar shared by the rail and the tab strip.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { matchesFacets, parseQuery, wordMatches, type Facets } from "./facets.ts";

const APP: Facets = {
  work: ["repo:me/app", "branch:feat/Login", "pr:me/app#57", "issue:me/app#3"],
  mention: ["repo:other/lib", "pr:other/lib#9", "ref:me/app#12"],
};

const hit = (q: string, f: Facets | undefined = APP) => {
  const { words, facets } = parseQuery(q);
  assert.deepEqual(words, [], `"${q}" parses fully as facets`);
  return matchesFacets(facets, f);
};

test("pr: matches a PR the session worked on, by number, repo, or both", () => {
  for (const q of ["pr:57", "pr:#57", "pr:app#57", "pr:me/app#57", "pr:app", "PR:57"]) {
    assert.equal(hit(q), true, q);
  }
  for (const q of ["pr:58", "pr:lib#57", "pr:3"]) assert.equal(hit(q), false, q);
});

test("issue: and bare #N", () => {
  assert.equal(hit("issue:3"), true);
  assert.equal(hit("issue:57"), false, "57 is a PR");
  assert.equal(hit("#57"), true);
  assert.equal(hit("#3"), true, "# matches either kind");
  assert.equal(hit("app#3"), true);
  assert.equal(hit("lib#3"), false);
});

test("mentions need the ~ modifier", () => {
  assert.equal(hit("pr:9"), false, "other/lib#9 was only mentioned");
  assert.equal(hit("~pr:9"), true);
  assert.equal(hit("repo:lib"), false);
  assert.equal(hit("~repo:lib"), true);
  assert.equal(hit("~#57"), true, "~ widens, it doesn't exclude work");
});

test("a bare-ref mention satisfies pr: and issue: alike", () => {
  assert.equal(hit("~pr:12"), true);
  assert.equal(hit("~issue:12"), true);
  assert.equal(hit("pr:12"), false);
});

test("repo: and branch: are substrings; branch is case-insensitive", () => {
  assert.equal(hit("repo:me/"), true);
  assert.equal(hit("branch:login"), true);
  assert.equal(hit("branch:main"), false);
});

test("terms AND; an item without facets satisfies none", () => {
  assert.equal(hit("repo:app pr:57"), true);
  assert.equal(hit("repo:app pr:9"), false);
  assert.equal(matchesFacets(parseQuery("pr:57").facets, undefined), false);
  assert.equal(matchesFacets([], undefined), true, "no terms, no constraint");
});

test("anything that isn't a facet stays a plain word", () => {
  assert.deepEqual(parseQuery("fix https://x.y/a pr: foo:bar ~ app56 status"), {
    words: ["fix", "https://x.y/a", "pr:", "foo:bar", "~", "app56", "status"],
    facets: [],
  });
  assert.deepEqual(
    parseQuery("pr:app56").facets,
    [{ kind: "pr", repo: "app56", loose: false }],
    "pr:<repo> without # is any PR in that repo, not PR 56",
  );
});

test("Jira keys: ticket:, jira:, a whole project, and ~ for mentions", () => {
  const f: Facets = {
    work: ["branch:feat/ABC-12-tabs", "ticket:ABC-12", "ticket:ABC-34"],
    mention: ["ticket:OPS-9"],
  };
  for (const q of ["ticket:ABC-12", "jira:abc-34", "ticket:abc", "~OPS-9", "~ticket:ops"]) {
    assert.equal(hit(q, f), true, q);
  }
  for (const q of ["ticket:abc-1", "ticket:ops", "ticket:ab", "~XYZ-12"]) {
    assert.equal(hit(q, f), false, q);
  }
});

test("a bare key is a word: in the text, or a ticket the session worked on", () => {
  const f: Facets = { work: ["ticket:ABC-12"], mention: ["ticket:OPS-9"] };
  assert.deepEqual(parseQuery("ABC-12").words, ["abc-12"]);
  assert.equal(wordMatches("abc-12", "some title", f), true);
  assert.equal(wordMatches("ops-9", "some title", f), false, "a mention needs ~");
  assert.equal(wordMatches("pty-7", "title\npty-7", undefined), true, "text still matches");
  assert.equal(wordMatches("utf-8", "title", { work: ["ticket:UTF-8"], mention: [] }), false);
});

test("standards and encodings shaped like keys stay plain words", () => {
  assert.deepEqual(parseQuery("utf-8 sha-256 iso-8601 x-1").words, ["utf-8", "sha-256", "iso-8601", "x-1"]);
  assert.deepEqual(parseQuery("ticket:x-y").words, ["ticket:x-y"]);
});
