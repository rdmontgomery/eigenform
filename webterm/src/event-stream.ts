/**
 * event-stream.ts — the window's one connection to the daemon's event bus
 * (`GET /api/events/stream`), fanned out to many subscribers.
 *
 * Two things listen: the Events pane (while the dock shows it) and deep-link
 * focus (for the window's whole life, hidden or not — a notification click is
 * exactly when the window is in the background). Sharing one EventSource keeps
 * it to one connection against the ~6-per-origin HTTP/1.1 budget, and makes the
 * daemon's count of connected windows (what `POST /api/focus` answers 200 vs 202
 * on) one per window.
 *
 * Reconnect: the stream is always a 200, so an error is a transport drop (daemon
 * restart, idle reaping). The hub closes, waits RECONNECT_MS, reopens, and tells
 * subscribers via `onReopen` so they can backfill the gap from `/api/events`.
 */

import type { EventRecord } from "./types.ts";

/** The slice of EventSource the hub uses — lets tests inject a fake. */
export interface StreamSource {
  onmessage: ((ev: { data: string }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  onopen: ((ev: unknown) => void) | null;
  close(): void;
}

export interface StreamDeps {
  makeSource: (url: string) => StreamSource;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (h: unknown) => void;
}

export interface StreamListener {
  onEvent: (ev: EventRecord) => void;
  /** The stream came back after a drop: events in the gap were missed. */
  onReopen?: () => void;
}

export const RECONNECT_MS = 1500;

export interface EventStream {
  /** Listen to every event. The first subscriber opens the stream, the last
   *  closes it. Returns an idempotent unsubscribe. */
  subscribe(listener: StreamListener): () => void;
  /** True while an EventSource is open (for tests/introspection). */
  isOpen(): boolean;
}

export function createEventStream(deps: StreamDeps): EventStream {
  const listeners = new Set<StreamListener>();
  let source: StreamSource | null = null;
  let timer: unknown | null = null;

  function connect(reopen: boolean) {
    const src = deps.makeSource("/api/events/stream");
    source = src;
    src.onopen = () => {
      if (!reopen || source !== src) return;
      reopen = false;
      for (const l of [...listeners]) l.onReopen?.();
    };
    src.onmessage = (e) => {
      let ev: EventRecord;
      try {
        ev = JSON.parse(e.data) as EventRecord;
      } catch {
        return; // a malformed frame
      }
      for (const l of [...listeners]) l.onEvent(ev);
    };
    src.onerror = () => {
      if (source !== src) return;
      src.close();
      source = null;
      if (timer === null) {
        timer = deps.setTimer(() => {
          timer = null;
          if (listeners.size > 0) connect(true);
        }, RECONNECT_MS);
      }
    };
  }

  function teardown() {
    if (timer !== null) {
      deps.clearTimer(timer);
      timer = null;
    }
    source?.close();
    source = null;
  }

  return {
    subscribe(listener) {
      listeners.add(listener);
      if (!source && timer === null) connect(false);
      let live = true;
      return () => {
        if (!live) return;
        live = false;
        listeners.delete(listener);
        if (listeners.size === 0) teardown();
      };
    },
    isOpen: () => source !== null,
  };
}

/** App-wide singleton, backed by the real EventSource and timers. */
export const eventStream: EventStream = createEventStream({
  makeSource: (url) => new EventSource(url) as unknown as StreamSource,
  setTimer: (fn, ms) => setTimeout(fn, ms),
  clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
});
