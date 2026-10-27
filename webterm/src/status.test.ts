// Tests for status.ts — the status-dot / tag / ink presentation helpers.
import { test } from "node:test";
import assert from "node:assert/strict";
import { cacheClock, dotClasses, dotTitle, inkVar, livenessTag } from "./status.ts";

test("dotClasses: activity sets color, liveness sets fill", () => {
  assert.equal(dotClasses("working", "eigenform"), "dot dot--working dot--eigenform");
  assert.equal(dotClasses("waiting", "external"), "dot dot--waiting dot--external");
  assert.equal(dotClasses("idle", "none"), "dot dot--idle dot--dead");
});

test("dotClasses: an unknown activity falls back to idle", () => {
  assert.equal(dotClasses("compacting", "eigenform"), "dot dot--idle dot--eigenform");
});

test("livenessTag: dead rows get no tag; external rows are prefixed", () => {
  assert.equal(livenessTag("working", "none"), null);
  assert.equal(livenessTag("working", "eigenform"), "· running");
  assert.equal(livenessTag("waiting", "eigenform"), "· your turn");
  assert.equal(livenessTag("idle", "external"), "· ext · live");
});

test("dotTitle: explains both channels, or just provenance when dead", () => {
  assert.equal(dotTitle("working", "eigenform"), "assistant running — eigenform session");
  assert.equal(
    dotTitle("waiting", "external"),
    "waiting for your input — running outside eigenform — can't attach",
  );
  assert.equal(dotTitle("working", "none"), "no live process");
});

test("inkVar: same cwd → same hue; falls back to the label without a cwd", () => {
  assert.equal(inkVar("/home/me/proj", "x"), inkVar("/home/me/proj", "y"));
  assert.equal(inkVar(undefined, "proj"), inkVar("proj", "ignored"));
  assert.match(inkVar("/a", "a"), /^var\(--ink-\w+\)$/);
});

test("cacheClock: runs between turns, graded against the TTL", () => {
  const at = "2026-06-06T09:00:00+00:00";
  const t0 = Date.parse(at);
  const min = 60_000;
  assert.equal(cacheClock(at, 3600, true, t0 + 10 * min), null, "paused while working");
  assert.equal(cacheClock(null, 3600, false, t0), null, "no response yet");
  assert.equal(cacheClock(at, 3600, false, t0 + 20_000)?.text, "<1m");
  assert.deepEqual(
    [10, 45, 54, 60].map((m) => cacheClock(at, 3600, false, t0 + m * min)?.level),
    ["warm", "warn", "urgent", "cold"],
  );
  assert.equal(cacheClock(at, 3600, false, t0 + 47 * min)?.text, "47m");
  assert.equal(cacheClock(at, 3600, false, t0 + 61 * min)?.text, "cold");
});

test("cacheClock: a 5m TTL goes cold in minutes; an unknown TTL assumes 1h", () => {
  const at = "2026-06-06T09:00:00+00:00";
  const t0 = Date.parse(at);
  assert.equal(cacheClock(at, 300, false, t0 + 6 * 60_000)?.level, "cold");
  const c = cacheClock(at, null, false, t0 + 10 * 60_000);
  assert.equal(c?.level, "warm");
  assert.match(c!.title, /1h \(assumed\) TTL · expires in 50m/);
});
