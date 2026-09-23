// Tests for the claims modal's pure shaping.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  claimActivity,
  claimAge,
  claimLabel,
  filterClaims,
  partitionClaims,
  summarizeClaims,
} from "./claims.ts";
import type { Claim } from "./types.ts";

function claim(o: Partial<Claim> & { pid: number }): Claim {
  return {
    sessionId: "0123456789abcdef",
    cwd: "/home/me/p",
    startedAt: 1000,
    kind: "interactive",
    entrypoint: "cli",
    status: null,
    name: null,
    title: null,
    health: "alive",
    headless: false,
    ptyId: null,
    ...o,
  };
}

const fixture = [
  claim({ pid: 1, startedAt: 10 }),
  claim({ pid: 2, startedAt: 30, headless: true }),
  claim({ pid: 3, startedAt: 20, health: "dead" }),
  claim({ pid: 4, startedAt: 40, health: "reused", headless: true }),
];

test("summary counts running, headless-among-running, and stale", () => {
  assert.deepEqual(summarizeClaims(fixture), { alive: 2, headless: 1, stale: 2 });
});

test("filter splits interactive from headless", () => {
  assert.deepEqual(filterClaims(fixture, "headless").map((c) => c.pid), [2, 4]);
  assert.deepEqual(filterClaims(fixture, "interactive").map((c) => c.pid), [1, 3]);
  assert.equal(filterClaims(fixture, "all").length, 4);
});

test("partition: running vs stale, newest first", () => {
  const { alive, stale } = partitionClaims(fixture);
  assert.deepEqual(alive.map((c) => c.pid), [2, 1]);
  assert.deepEqual(stale.map((c) => c.pid), [4, 3]);
});

test("label falls back title → name → short id", () => {
  assert.equal(claimLabel(claim({ pid: 1, title: "T", name: "n" })), "T");
  assert.equal(claimLabel(claim({ pid: 1, name: "n" })), "n");
  assert.equal(claimLabel(claim({ pid: 1 })), "01234567");
});

test("status maps busy → working, idle → waiting", () => {
  assert.equal(claimActivity(claim({ pid: 1, status: "busy" })), "working");
  assert.equal(claimActivity(claim({ pid: 1, status: "idle" })), "waiting");
  assert.equal(claimActivity(claim({ pid: 1 })), "idle");
});

test("age is compact", () => {
  assert.equal(claimAge(null, 0), "");
  assert.equal(claimAge(0, 42_000), "42s");
  assert.equal(claimAge(0, 17 * 60_000), "17m");
  assert.equal(claimAge(0, 3 * 3_600_000), "3h");
  assert.equal(claimAge(0, 5 * 86_400_000), "5d");
});
