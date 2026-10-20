// Tests for event-stream.ts — one shared /api/events/stream per window. Run: `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { RECONNECT_MS, createEventStream, type StreamSource } from "./event-stream.ts";
import type { EventRecord } from "./types.ts";

class FakeSource implements StreamSource {
  onmessage: ((ev: { data: string }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onopen: ((ev: unknown) => void) | null = null;
  closed = false;
  url: string;
  constructor(url: string) {
    this.url = url;
  }
  close() {
    this.closed = true;
  }
  emit(ev: Partial<EventRecord>) {
    this.onmessage?.({ data: JSON.stringify(ev) });
  }
}

function harness() {
  const sources: FakeSource[] = [];
  const timers: { fn: () => void; ms: number; cleared: boolean }[] = [];
  const stream = createEventStream({
    makeSource: (url) => {
      const s = new FakeSource(url);
      sources.push(s);
      return s;
    },
    setTimer: (fn, ms) => {
      const t = { fn, ms, cleared: false };
      timers.push(t);
      return t;
    },
    clearTimer: (h) => {
      (h as { cleared: boolean }).cleared = true;
    },
  });
  return { stream, sources, timers };
}

test("subscribers share one source and each gets every event", () => {
  const { stream, sources } = harness();
  const a: string[] = [];
  const b: string[] = [];
  stream.subscribe({ onEvent: (e) => a.push(e.kind) });
  stream.subscribe({ onEvent: (e) => b.push(e.kind) });
  assert.equal(sources.length, 1);
  assert.equal(sources[0]!.url, "/api/events/stream");
  sources[0]!.emit({ seq: 1, kind: "focus", data: {} });
  sources[0]!.onmessage?.({ data: "not json" }); // ignored
  assert.deepEqual(a, ["focus"]);
  assert.deepEqual(b, ["focus"]);
});

test("the last unsubscribe closes the source", () => {
  const { stream, sources } = harness();
  const off1 = stream.subscribe({ onEvent: () => {} });
  const off2 = stream.subscribe({ onEvent: () => {} });
  off1();
  off1(); // idempotent
  assert.ok(!sources[0]!.closed);
  off2();
  assert.ok(sources[0]!.closed);
  assert.ok(!stream.isOpen());
});

test("a drop reconnects after a backoff and reports the reopen", () => {
  const { stream, sources, timers } = harness();
  let reopens = 0;
  stream.subscribe({ onEvent: () => {}, onReopen: () => reopens++ });
  sources[0]!.onopen?.({}); // the first open is not a reopen
  assert.equal(reopens, 0);

  sources[0]!.onerror?.({});
  assert.ok(sources[0]!.closed);
  assert.equal(timers.length, 1);
  assert.equal(timers[0]!.ms, RECONNECT_MS);
  timers[0]!.fn();
  assert.equal(sources.length, 2);
  sources[1]!.onopen?.({});
  assert.equal(reopens, 1);
});

test("unsubscribing during a backoff cancels the reconnect", () => {
  const { stream, sources, timers } = harness();
  const off = stream.subscribe({ onEvent: () => {} });
  sources[0]!.onerror?.({});
  off();
  assert.ok(timers[0]!.cleared);
});
