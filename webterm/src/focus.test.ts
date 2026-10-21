// Tests for focus.ts pure helpers — deep-link parsing and which window acts. Run: `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  STALE_MS,
  focusTarget,
  isMostRecent,
  isSessionUuid,
  pruneRegistry,
  sessionFromSearch,
} from "./focus.ts";
import type { EventRecord } from "./types.ts";

const U = "aaaa1111-0000-4000-8000-000000000001";

function ev(kind: string, data: Record<string, unknown>): EventRecord {
  return { seq: 1, at: "2026-10-20T12:00:00.000Z", kind, data };
}

test("isSessionUuid accepts canonical uuids only", () => {
  assert.ok(isSessionUuid(U));
  assert.ok(isSessionUuid(U.toUpperCase()));
  assert.ok(!isSessionUuid(""));
  assert.ok(!isSessionUuid(U.slice(1)));
  assert.ok(!isSessionUuid(`${U}0`));
  assert.ok(!isSessionUuid(U.replaceAll("-", "")));
  assert.ok(!isSessionUuid(42));
  assert.ok(!isSessionUuid(undefined));
});

test("focusTarget reads a focus event's session, lowercased", () => {
  assert.deepEqual(focusTarget(ev("focus", { session: U })), { session: U });
  assert.deepEqual(focusTarget(ev("focus", { session: U.toUpperCase() })), { session: U });
  assert.deepEqual(focusTarget(ev("focus", { session: U, replayed: true })), { session: U });
  assert.equal(focusTarget(ev("focus", { session: "nope" })), null);
  assert.equal(focusTarget(ev("focus", {})), null);
  assert.equal(focusTarget(ev("pty-spawned", { session: U })), null);
});

test("focusTarget reads a focus event's pty id", () => {
  assert.deepEqual(focusTarget(ev("focus", { pty: "12" })), { pty: "12" });
  assert.deepEqual(focusTarget(ev("focus", { pty: "12", replayed: true })), { pty: "12" });
  assert.equal(focusTarget(ev("focus", { pty: 12 })), null, "the daemon sends ids as strings");
  assert.equal(focusTarget(ev("focus", { pty: "12; rm" })), null);
  assert.equal(focusTarget(ev("focus", { pty: "" })), null);
});

test("sessionFromSearch splits ?session= off and keeps the rest", () => {
  assert.equal(sessionFromSearch(""), null);
  assert.equal(sessionFromSearch("?theme=dark"), null);
  assert.deepEqual(sessionFromSearch(`?session=${U}`), { uuid: U, rest: "" });
  assert.deepEqual(sessionFromSearch(`?a=1&session=${U.toUpperCase()}&b=2`), {
    uuid: U,
    rest: "?a=1&b=2",
  });
  // Malformed: still stripped, nothing to focus.
  assert.deepEqual(sessionFromSearch("?session=junk"), { uuid: null, rest: "" });
});

test("isMostRecent: the live window focused last wins", () => {
  const now = 1_000_000;
  const reg = {
    a: { focusedAt: now - 500, aliveAt: now },
    b: { focusedAt: now - 100, aliveAt: now },
  };
  assert.ok(isMostRecent(reg, "b", now));
  assert.ok(!isMostRecent(reg, "a", now));
});

test("isMostRecent: a dead window's newer stamp doesn't block the living", () => {
  const now = 10_000_000;
  const reg = {
    alive: { focusedAt: now - 60_000, aliveAt: now },
    crashed: { focusedAt: now - 1_000, aliveAt: now - STALE_MS - 1 },
  };
  assert.ok(isMostRecent(reg, "alive", now));
});

test("isMostRecent: exactly one window wins a tie", () => {
  const now = 1_000;
  const reg = {
    x: { focusedAt: 5, aliveAt: now },
    y: { focusedAt: 5, aliveAt: now },
  };
  assert.notEqual(isMostRecent(reg, "x", now), isMostRecent(reg, "y", now));
});

test("isMostRecent: with no registry every window acts", () => {
  assert.ok(isMostRecent({}, "me", 1));
});

test("pruneRegistry drops entries past STALE_MS", () => {
  const now = 10_000_000;
  const pruned = pruneRegistry(
    {
      fresh: { focusedAt: 1, aliveAt: now - 1000 },
      stale: { focusedAt: 2, aliveAt: now - STALE_MS - 1 },
    },
    now,
  );
  assert.deepEqual(Object.keys(pruned), ["fresh"]);
});
