// Tests for facets.ts: the search grammar shared by the rail and the tab strip.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { matchesFacets, parseQuery, type Facets } from "./facets.ts";

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
