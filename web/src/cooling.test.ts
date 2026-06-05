// Tests for the pure cache-economy model — the logic most likely to be swapped
// for real telemetry, so it earns real tests. Run: `node --test` (native TS).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  TTL_DEFAULT,
  TTL_EXTENDED,
  CTX_FULL,
  TOTAL_TURNS,
  prefixTokensTo,
  dropsAt,
  cacheReading,
  forkReading,
  fmtClock,
  fmtK,
  tempColor,
} from "./cooling.ts";

test("prefixTokensTo scales the full context by turn fraction", () => {
  assert.equal(prefixTokensTo(TOTAL_TURNS), CTX_FULL);
  assert.equal(prefixTokensTo(0), 0);
  assert.equal(prefixTokensTo(Math.round(TOTAL_TURNS / 2)), Math.round(CTX_FULL * (Math.round(TOTAL_TURNS / 2) / TOTAL_TURNS)));
});

test("dropsAt counts turns discarded downstream, never negative", () => {
  assert.equal(dropsAt(TOTAL_TURNS), 0); // the leaf drops nothing
  assert.equal(dropsAt(TOTAL_TURNS - 5), 5);
  assert.equal(dropsAt(TOTAL_TURNS + 3), 0); // clamps
  assert.equal(dropsAt(1), TOTAL_TURNS - 1);
});

test("cacheReading: hot right after a write", () => {
  const c = cacheReading(0);
  assert.equal(c.temp, 1);
  assert.equal(c.cold, false);
  assert.equal(c.label, "hot");
  assert.equal(c.remaining, TTL_DEFAULT);
});

test("cacheReading: label bands track temperature", () => {
  assert.equal(cacheReading(TTL_DEFAULT * 0.2).label, "hot"); // temp .8
  assert.equal(cacheReading(TTL_DEFAULT * 0.5).label, "warm"); // temp .5
  assert.equal(cacheReading(TTL_DEFAULT * 0.8).label, "cooling"); // temp .2
});

test("cacheReading: cold once idle reaches the TTL", () => {
  const c = cacheReading(TTL_DEFAULT);
  assert.equal(c.cold, true);
  assert.equal(c.label, "cold");
  assert.equal(c.temp, 0);
  assert.equal(c.remaining, 0);
});

test("cacheReading: temperature is clamped to [0,1]", () => {
  assert.equal(cacheReading(-50).temp, 1);
  assert.equal(cacheReading(TTL_DEFAULT * 4).temp, 0);
});

test("cacheReading: extended TTL cools more slowly", () => {
  const c = cacheReading(TTL_DEFAULT, TTL_EXTENDED);
  assert.equal(c.cold, false);
  assert.ok(c.temp > 0.9);
});

test("forkReading: the leaf is free", () => {
  const f = forkReading(TOTAL_TURNS, cacheReading(0));
  assert.equal(f.leaf, true);
  assert.equal(f.reWarm, 0);
  assert.equal(f.drops, 0);
});

test("forkReading: warm fork re-warms only a fraction of the prefix", () => {
  const f = forkReading(135, cacheReading(0)); // temp 1 → factor 0
  assert.equal(f.cold, false);
  assert.equal(f.reWarm, 0); // (1 - temp) == 0 while fully hot
});

test("forkReading: cold fork re-warms the whole surviving prefix", () => {
  const f = forkReading(135, cacheReading(TTL_DEFAULT)); // cold
  assert.equal(f.cold, true);
  assert.equal(f.reWarm, prefixTokensTo(135));
  assert.equal(f.drops, dropsAt(135));
});

test("forkReading: earlier cold forks keep a shorter (cheaper) prefix", () => {
  const early = forkReading(10, cacheReading(TTL_DEFAULT));
  const late = forkReading(130, cacheReading(TTL_DEFAULT));
  assert.ok(early.reWarm < late.reWarm);
});

test("fmtClock formats m:ss and floors negatives to 0:00", () => {
  assert.equal(fmtClock(0), "0:00");
  assert.equal(fmtClock(5), "0:05");
  assert.equal(fmtClock(65), "1:05");
  assert.equal(fmtClock(300), "5:00");
  assert.equal(fmtClock(-9), "0:00");
});

test("fmtK abbreviates thousands with Tufte-quiet precision", () => {
  assert.equal(fmtK(0), "0");
  assert.equal(fmtK(900), "900");
  assert.equal(fmtK(1800), "1.8k");
  assert.equal(fmtK(14200), "14k");
  assert.equal(fmtK(118000), "118k");
});

test("tempColor mixes cold→amber via theme vars, summing to 100%", () => {
  assert.equal(tempColor(1), "color-mix(in oklab, var(--cold) 0%, var(--amber) 100%)");
  assert.equal(tempColor(0), "color-mix(in oklab, var(--cold) 100%, var(--amber) 0%)");
  assert.equal(tempColor(0.5), "color-mix(in oklab, var(--cold) 50%, var(--amber) 50%)");
});
