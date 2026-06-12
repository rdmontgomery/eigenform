/**
 * fuzzy.test.ts — Tests for rankCandidates pure fuzzy scorer.
 * Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
 *
 * Ranking priorities (ordinal, not weights):
 *   1. basenameHit: query letters appear as subsequence in basename(path)
 *   2. longestRun: length of longest contiguous matching run in full path
 *   3. wordBoundaryStarts: matches that start at a word boundary (/, -, _, .)
 *   4. recent: candidate.recent === true
 *   5. input order (stable sort within equal score)
 *
 * Case: queries are lowercased before matching; paths are matched case-insensitively.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { rankCandidates } from "./fuzzy.ts";
import type { Candidate } from "./types.ts";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function c(path: string, recent = false): Candidate {
  return { path, recent };
}

function paths(candidates: Candidate[]): string[] {
  return candidates.map((c) => c.path);
}

// ---------------------------------------------------------------------------
// Non-subsequence exclusion
// ---------------------------------------------------------------------------

test("non-subsequence is excluded", () => {
  const candidates = [c("/home/r/projects/eigen"), c("/home/r/other")];
  const result = rankCandidates("xyz", candidates);
  assert.equal(result.length, 0);
});

test("partial non-subsequence is excluded (letters present but wrong order)", () => {
  // "negie" — letters n,e,g,i,e all in "eigen" but not in subsequence order "negie"
  // Actually "n" comes after "e" in "eigen", "e" before "n", "g" after "n" — negie fails
  const result = rankCandidates("negie", [c("/home/r/projects/eigen")]);
  assert.equal(result.length, 0);
});

// ---------------------------------------------------------------------------
// Empty query — input order preserved, all candidates returned
// ---------------------------------------------------------------------------

test("empty query returns all candidates in input order", () => {
  const candidates = [
    c("/home/r/projects/eigen"),
    c("/home/r/other/foo"),
    c("/home/r/bar"),
  ];
  const result = rankCandidates("", candidates);
  assert.deepEqual(paths(result), paths(candidates));
});

test("empty query with empty candidates returns empty", () => {
  const result = rankCandidates("", []);
  assert.equal(result.length, 0);
});

// ---------------------------------------------------------------------------
// Basename hit beats non-basename hit
// ---------------------------------------------------------------------------

test("basename hit ranks above non-basename hit", () => {
  // "eig" matches basename "eigen" in /home/r/projects/eigen
  // "eig" matches path component "eigtmp" in /home/r/eigtmp/old (also basename hit — use different example)
  // Use path where "eig" is only in directory prefix, not basename
  const basenameHit = c("/home/r/projects/eigen");      // basename=eigen, "eig" in basename
  const nonBasename = c("/home/r/eigtmp/sessions");     // basename=sessions, "eig" only in dir prefix
  const result = rankCandidates("eig", [nonBasename, basenameHit]);
  // basenameHit should rank first despite being second in input
  assert.equal(result[0]!.path, basenameHit.path);
  assert.equal(result[1]!.path, nonBasename.path);
});

test("basename hit: eig matches eigen above eigtmp/old", () => {
  // From spec: "eig" matches /home/r/projects/eigen (basename hit) above /home/r/eigtmp/old (non-basename)
  const eigenPath = c("/home/r/projects/eigen");
  const eigtmpOld = c("/home/r/eigtmp/old");
  const result = rankCandidates("eig", [eigtmpOld, eigenPath]);
  assert.equal(result[0]!.path, eigenPath.path);
  assert.equal(result[1]!.path, eigtmpOld.path);
});

// ---------------------------------------------------------------------------
// Contiguous run beats scattered match
// ---------------------------------------------------------------------------

test("contiguous run beats scattered match", () => {
  // "eign" against "eigen" (contiguous "eign" — run=4)
  // vs a path where e, i, g, n are spread out with gaps
  const contiguous = c("/home/r/projects/eigen"); // "eign" contiguous in basename
  const scattered = c("/home/r/e/ignition/xn");   // e,i,g,n but spread: /e/ + ign + xn — run of 3 at most
  const result = rankCandidates("eign", [scattered, contiguous]);
  assert.equal(result[0]!.path, contiguous.path);
});

// ---------------------------------------------------------------------------
// Recent breaks ties between equal scores
// ---------------------------------------------------------------------------

test("recent breaks ties between equal-scored candidates", () => {
  // Use paths with identical scoring for query "zt":
  //   both have basename starting after "/" boundary (wordBoundary hit)
  //   "zt" is NOT a subsequence of either basename ("ztr" vs "ztx") — same basenameHit
  // Actually simplest: paths with identical basenames, only differing in recent flag.
  // query "q" — neither "zoo" nor "yak" contain "q" in their basename; "q" is only in the
  // shared "/qbase/" directory segment so basenameHit=0 for both, run=1 for both,
  // wordBoundaryStarts=1 for both (q follows /). Only tie-breaker is recent.
  const nonRecent = c("/qbase/zoo", false);
  const recent = c("/qbase/yak", true);
  const result = rankCandidates("q", [nonRecent, recent]);
  assert.equal(result[0]!.path, recent.path);
});

test("recent breaks ties: recent comes before non-recent with identical score", () => {
  const a = c("/work/foo", false);
  const b = c("/work/foo2", true);
  // query "foo" — both have basename hit + same run; b.recent wins
  const result = rankCandidates("foo", [a, b]);
  assert.equal(result[0]!.path, b.path);
});

// ---------------------------------------------------------------------------
// Case-insensitive matching
// ---------------------------------------------------------------------------

test("query is matched case-insensitively (uppercase path)", () => {
  const result = rankCandidates("eigen", [c("/home/r/projects/Eigen")]);
  assert.equal(result.length, 1);
  assert.equal(result[0]!.path, "/home/r/projects/Eigen");
});

test("uppercase query matches lowercase path", () => {
  const result = rankCandidates("EIGEN", [c("/home/r/projects/eigen")]);
  assert.equal(result.length, 1);
});

test("mixed-case query matches mixed-case path", () => {
  const result = rankCandidates("MyProj", [c("/home/r/MyProject")]);
  assert.equal(result.length, 1);
});

// ---------------------------------------------------------------------------
// Query longer than path — excluded (can't form subsequence)
// ---------------------------------------------------------------------------

test("query longer than path is excluded", () => {
  const result = rankCandidates("averylongquerythatexceedspath", [c("/a/b")]);
  assert.equal(result.length, 0);
});

// ---------------------------------------------------------------------------
// Unicode in paths
// ---------------------------------------------------------------------------

test("unicode in path — exact subsequence match works", () => {
  const result = rankCandidates("café", [c("/home/user/café-proj")]);
  assert.equal(result.length, 1);
});

test("unicode in path — non-match is excluded", () => {
  const result = rankCandidates("xyz", [c("/home/user/café-proj")]);
  assert.equal(result.length, 0);
});

// ---------------------------------------------------------------------------
// Stable sort — same-score candidates preserve input order
// ---------------------------------------------------------------------------

test("stable sort: same-score candidates preserve input order", () => {
  // These three paths all have basename starting with "p", same run length for "p"
  // and all recent=false. Input order must be preserved.
  const candidates = [
    c("/home/r/projects/proj-a"),
    c("/home/r/projects/proj-b"),
    c("/home/r/projects/proj-c"),
  ];
  const result = rankCandidates("p", candidates);
  // All should match; order should be stable (input order preserved within same score)
  const resultPaths = paths(result);
  const matchingPaths = paths(candidates);
  // All three match "p"
  assert.equal(result.length, 3);
  // Verify relative order of equal-scored items is preserved
  const aIdx = resultPaths.indexOf("/home/r/projects/proj-a");
  const bIdx = resultPaths.indexOf("/home/r/projects/proj-b");
  const cIdx = resultPaths.indexOf("/home/r/projects/proj-c");
  assert.ok(aIdx < bIdx, "proj-a should appear before proj-b");
  assert.ok(bIdx < cIdx, "proj-b should appear before proj-c");
  void matchingPaths; // used above
});

test("stable sort: recents-first order from API survives equal scores", () => {
  // Simulate: API returns recents first (all recent=true), then non-recents.
  // Query "q" — none of basenames "zeta", "yak", "kiwi" contain "q";
  // "q" only matches in the shared "/qroot/" directory segment.
  // So all three have identical scores (basenameHit=0, longestRun=1, wordBoundaryStarts=1).
  // recents sort before non-recents; within each tier, input order is preserved.
  const candidates = [
    c("/qroot/zeta", true),   // recent, index 0
    c("/qroot/yak", true),    // recent, index 1
    c("/qroot/kiwi", false),  // non-recent, index 2
  ];
  const result = rankCandidates("q", candidates);
  assert.equal(result.length, 3);
  // recents come first (higher score), then non-recents; within each group, input order holds
  assert.equal(result[0]!.path, "/qroot/zeta");
  assert.equal(result[1]!.path, "/qroot/yak");
  assert.equal(result[2]!.path, "/qroot/kiwi");
});

// ---------------------------------------------------------------------------
// Additional ranking sanity
// ---------------------------------------------------------------------------

test("full subsequence match on basename scores higher than partial elsewhere", () => {
  const fullBasename = c("/work/eigen");          // basename=eigen, full match for "eigen"
  const inDir = c("/work/eigen-things/readme");   // basename=readme, "eigen" in dir component
  const result = rankCandidates("eigen", [inDir, fullBasename]);
  assert.equal(result[0]!.path, fullBasename.path);
});

test("word boundary start boosts score", () => {
  // "foo" at start of basename is a word-boundary start
  // vs "foo" appearing mid-word in a deep path
  const boundaryStart = c("/home/user/foo-project");  // basename starts with "foo" (word boundary at start)
  const midWord = c("/home/user/xfooxyz");             // "foo" in middle of basename, no boundary
  const result = rankCandidates("foo", [midWord, boundaryStart]);
  assert.equal(result[0]!.path, boundaryStart.path);
});

test("single character query matches correctly", () => {
  // "/work/klm" has no "e" anywhere in the path, so only the eigen path matches
  const result = rankCandidates("e", [
    c("/home/r/projects/eigen"),
    c("/work/klm"),
  ]);
  assert.equal(result.length, 1);
  assert.equal(result[0]!.path, "/home/r/projects/eigen");
});

// ---------------------------------------------------------------------------
// Greedy-limitation pin: "ac" in "/xaac" vs "/xac"
//
// This test pins the ACTUAL (greedy) ranking outcome, not the theoretically
// optimal one. Greedy left-to-right assigns "ac" in "/xaac" positions [2,4]
// (run=1 — skips the better [3,4] pair that would give run=2). "/xac" gets
// positions [2,3] (run=2). So "/xac" scores HIGHER than "/xaac" under greedy.
//
// A max-run algorithm would find [3,4] in "/xaac" (run=2), tying "/xac" and
// making input order the tiebreaker — changing the ranking outcome. This test
// pins the current greedy behavior so a future algorithm swap is visible.
// ---------------------------------------------------------------------------

test("greedy limitation pin: 'ac' in '/xaac' vs '/xac' — greedy gives /xaac run=1 vs /xac run=2", () => {
  // greedy on "/xaac": picks 'a' at index 2, 'c' at index 4 → positions [2,4] → run=1
  // greedy on "/xac":  picks 'a' at index 2, 'c' at index 3 → positions [2,3] → run=2
  // both have basenameHit=1 (basename contains "ac" as subseq) and same wordBoundaryStarts
  // "/xac" wins on longestRun (2 > 1) and comes first despite being input-order second.
  // A max-run algorithm would give "/xaac" run=2 also → tie → input order → "/xaac" first.
  // This test fails if the algorithm is changed to max-run, surfacing the behavior change.
  const first = c("/xaac");
  const second = c("/xac");
  const result = rankCandidates("ac", [first, second]);
  assert.equal(result.length, 2, "both paths match 'ac'");
  // "/xac" ranks FIRST because greedy gives it run=2 vs "/xaac" run=1.
  assert.equal(result[0]!.path, "/xac", "greedy: /xac ranks first (run=2)");
  assert.equal(result[1]!.path, "/xaac", "greedy: /xaac ranks second (run=1, not the optimal run=2)");
});
