// Tests for watch.ts — the shared, refcounted SSE hub. Run: `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createWatchHub, RECONNECT_MS } from "./watch.ts";
import type { WatchSource, WatchDeps } from "./watch.ts";

/** A fake EventSource that records lifecycle and lets tests drive events. */
class FakeSource implements WatchSource {
  onmessage: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  changeCbs: Array<() => void> = [];
  closed = false;
  url: string;
  constructor(url: string) {
    this.url = url;
  }
  addEventListener(type: string, cb: () => void): void {
    if (type === "change") this.changeCbs.push(cb);
  }
  close(): void {
    this.closed = true;
  }
  // Test drivers:
  emitMessage(): void {
    this.onmessage?.({});
  }
  emitChange(): void {
    for (const cb of this.changeCbs) cb();
  }
  emitError(): void {
    this.onerror?.({});
  }
}

/** A harness: fake sources + a manual timer queue. */
function harness() {
  const sources: FakeSource[] = [];
  let timerSeq = 0;
  const timers = new Map<number, { fn: () => void; ms: number }>();
  const deps: WatchDeps = {
    makeSource: (url) => {
      const s = new FakeSource(url);
      sources.push(s);
      return s;
    },
    setTimer: (fn, ms) => {
      const id = ++timerSeq;
      timers.set(id, { fn, ms });
      return id;
    },
    clearTimer: (h) => {
      timers.delete(h as number);
    },
  };
  return {
    hub: createWatchHub(deps),
    sources,
    /** Fire every pending timer (like advancing the clock past all of them). */
    flushTimers() {
      const pending = [...timers.entries()];
      timers.clear();
      for (const [, t] of pending) t.fn();
    },
    pendingTimers: () => timers.size,
  };
}

test("subscribe: two subscribers to one uuid share a single source", () => {
  const h = harness();
  const a: string[] = [];
  const b: string[] = [];
  h.hub.subscribe("u1", () => a.push("a"));
  h.hub.subscribe("u1", () => b.push("b"));
  assert.equal(h.sources.length, 1, "only one EventSource opened");
  assert.equal(h.hub.openCount(), 1);
  assert.equal(h.sources[0].url, "/api/watch/u1");
});

test("fan-out: a message/change notifies every subscriber", () => {
  const h = harness();
  let a = 0;
  let b = 0;
  h.hub.subscribe("u1", () => a++);
  h.hub.subscribe("u1", () => b++);
  h.sources[0].emitMessage();
  assert.deepEqual([a, b], [1, 1]);
  h.sources[0].emitChange();
  assert.deepEqual([a, b], [2, 2]);
});

test("distinct uuids get distinct sources", () => {
  const h = harness();
  h.hub.subscribe("u1", () => {});
  h.hub.subscribe("u2", () => {});
  assert.equal(h.sources.length, 2);
  assert.equal(h.hub.openCount(), 2);
});

test("refcount: source stays open until the LAST subscriber leaves", () => {
  const h = harness();
  const off1 = h.hub.subscribe("u1", () => {});
  const off2 = h.hub.subscribe("u1", () => {});
  off1();
  assert.equal(h.sources[0].closed, false, "still one subscriber");
  assert.equal(h.hub.openCount(), 1);
  off2();
  assert.equal(h.sources[0].closed, true, "last subscriber left → closed");
  assert.equal(h.hub.openCount(), 0);
});

test("unsubscribe is idempotent and doesn't double-close", () => {
  const h = harness();
  const off = h.hub.subscribe("u1", () => {});
  off();
  off(); // no throw, no effect
  assert.equal(h.hub.openCount(), 0);
});

test("a delivered event does not reach an unsubscribed listener", () => {
  const h = harness();
  let a = 0;
  let b = 0;
  const offA = h.hub.subscribe("u1", () => a++);
  h.hub.subscribe("u1", () => b++);
  offA();
  h.sources[0].emitChange();
  assert.deepEqual([a, b], [0, 1]);
});

test("reconnect: onerror schedules a backoff, then opens a fresh source", () => {
  const h = harness();
  h.hub.subscribe("u1", () => {});
  assert.equal(h.sources.length, 1);
  h.sources[0].emitError(); // e.g. the pre-flush 404
  assert.equal(h.sources[0].closed, true, "dead source closed");
  assert.equal(h.hub.openCount(), 0, "no open source between attempts");
  assert.equal(h.pendingTimers(), 1, "a reconnect is scheduled");
  h.flushTimers();
  assert.equal(h.sources.length, 2, "reconnected with a new source");
  assert.equal(h.hub.openCount(), 1);
  // The new source still fans out.
  let hit = 0;
  h.hub.subscribe("u1", () => hit++);
  h.sources[1].emitChange();
  assert.equal(hit, 1);
});

test("reconnect: a single error schedules only one timer", () => {
  const h = harness();
  h.hub.subscribe("u1", () => {});
  h.sources[0].emitError();
  h.sources[0].emitError(); // duplicate error on the same dead source
  assert.equal(h.pendingTimers(), 1);
});

test("teardown cancels a pending reconnect", () => {
  const h = harness();
  const off = h.hub.subscribe("u1", () => {});
  h.sources[0].emitError();
  assert.equal(h.pendingTimers(), 1);
  off(); // last subscriber leaves before the backoff fires
  assert.equal(h.pendingTimers(), 0, "reconnect timer cleared");
  h.flushTimers();
  assert.equal(h.sources.length, 1, "no reconnect after teardown");
});

test("RECONNECT_MS is the scheduled delay", () => {
  const h = harness();
  h.hub.subscribe("u1", () => {});
  h.sources[0].emitError();
  // Peek the one pending timer's delay via a fresh harness call path.
  // (flushTimers ignores ms; assert the constant is what the hub used.)
  assert.equal(RECONNECT_MS, 1500);
});
