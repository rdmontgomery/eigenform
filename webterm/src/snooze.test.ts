/**
 * snooze.test.ts — duration parsing, preset wake times, wake formatting, due picking.
 * Run: `node --test`.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  parseDuration,
  formatWake,
  dueSnoozes,
  tomorrowAt,
  snoozableTab,
  SNOOZE_PRESETS,
  type Snooze,
} from "./snooze.ts";

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

test("parseDuration reads units, compounds, and bare minutes", () => {
  assert.equal(parseDuration("30m"), 30 * MIN);
  assert.equal(parseDuration("45 min"), 45 * MIN);
  assert.equal(parseDuration("2h"), 2 * HOUR);
  assert.equal(parseDuration("1h30m"), 90 * MIN);
  assert.equal(parseDuration("1h 30m"), 90 * MIN);
  assert.equal(parseDuration("3 days"), 3 * DAY);
  assert.equal(parseDuration("1w"), 7 * DAY);
  assert.equal(parseDuration("1.5h"), 90 * MIN);
  assert.equal(parseDuration("20"), 20 * MIN);
  assert.equal(parseDuration(" 2D "), 2 * DAY);
});

test("parseDuration rejects junk and zero", () => {
  assert.equal(parseDuration(""), null);
  assert.equal(parseDuration("soon"), null);
  assert.equal(parseDuration("3 fortnights"), null);
  assert.equal(parseDuration("0m"), null);
  assert.equal(parseDuration("0"), null);
  assert.equal(parseDuration("5m and then"), null);
});

test("tomorrowAt lands on tomorrow's local hour", () => {
  const now = new Date(2026, 9, 2, 23, 30).getTime();
  const t = new Date(tomorrowAt(now, 9));
  assert.equal(t.getDate(), 3);
  assert.equal(t.getHours(), 9);
  assert.equal(t.getMinutes(), 0);
});

test("every preset wakes in the future", () => {
  const now = Date.now();
  for (const p of SNOOZE_PRESETS) assert.ok(p.until(now) > now, p.label);
});

test("formatWake rounds up and compacts", () => {
  const now = 1_000_000;
  assert.equal(formatWake(now - 1, now), "now");
  assert.equal(formatWake(now + 1, now), "in 1m");
  assert.equal(formatWake(now + 25 * MIN, now), "in 25m");
  assert.equal(formatWake(now + 2 * HOUR, now), "in 2h");
  assert.equal(formatWake(now + 2 * HOUR + 10 * MIN, now), "in 2h 10m");
  assert.equal(formatWake(now + 3 * DAY, now), "in 3d");
  assert.equal(formatWake(now + 3 * DAY + 4 * HOUR, now), "in 3d 4h");
});

test("dueSnoozes picks only arrived wakes, soonest first", () => {
  const s = (id: string, until: number): Snooze => ({
    id, until, snoozedAt: 0, tab: { label: id },
  });
  const due = dueSnoozes([s("b", 200), s("future", 900), s("a", 100)], 500);
  assert.deepEqual(due.map((x) => x.id), ["a", "b"]);
});

test("snoozableTab drops the transient seed", () => {
  assert.deepEqual(
    snoozableTab({ label: "x", uuid: "u", seedInput: "typed" }),
    { label: "x", uuid: "u" },
  );
});
