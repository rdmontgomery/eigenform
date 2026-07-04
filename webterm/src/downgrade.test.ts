// Tests for the auto-recover decision gate — pure, no DOM/fetch.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { shouldAutoRecover } from "./downgrade.ts";
import type { DowngradeCandidate } from "./downgrade.ts";

// A far-past keystroke so the recent-input guard never trips unless a test wants it to.
const NOW = 1_000_000;
const QUIET = 0; // no keystroke ever → now - 0 >> recentInputMs

function base(overrides: Partial<Parameters<typeof shouldAutoRecover>[0]> = {}) {
  return {
    activeUuid: "aaa",
    rows: [{ uuid: "aaa", downgrade: { offendingTurn: "u2" } }] as DowngradeCandidate[],
    handled: new Set<string>(),
    lastInputAt: QUIET,
    now: NOW,
    recentInputMs: 1500,
    ...overrides,
  };
}

test("fires for the active downgraded session", () => {
  assert.deepEqual(shouldAutoRecover(base()), { uuid: "aaa", offendingTurn: "u2" });
});

test("returns null when the session was already handled", () => {
  assert.equal(shouldAutoRecover(base({ handled: new Set(["aaa"]) })), null);
});

test("returns null when the user typed within recentInputMs", () => {
  // now - lastInputAt = 1000 < 1500
  assert.equal(shouldAutoRecover(base({ lastInputAt: NOW - 1000 })), null);
});

test("returns null when the active session has no downgrade", () => {
  assert.equal(
    shouldAutoRecover(base({ rows: [{ uuid: "aaa", downgrade: null }] })),
    null,
  );
});

test("returns null when the downgraded session is NOT the active one", () => {
  assert.equal(
    shouldAutoRecover(
      base({
        activeUuid: "aaa",
        rows: [{ uuid: "bbb", downgrade: { offendingTurn: "u2" } }],
      }),
    ),
    null,
  );
});

test("returns null when activeUuid is null", () => {
  assert.equal(shouldAutoRecover(base({ activeUuid: null })), null);
});
