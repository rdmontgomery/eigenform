/**
 * watch.ts — one shared SSE stream per session, fanned out to many subscribers.
 *
 * The reach map and the transcript drawer both follow the same session file via
 * `GET /api/watch/:uuid`. Opening a separate EventSource from each doubles the
 * daemon connections for one file and, over HTTP/1.1, eats into the browser's
 * ~6-per-origin connection budget. This hub keeps a single EventSource per uuid,
 * reference-counted across subscribers: the first subscriber opens it, the last
 * to leave closes it.
 *
 * Reconnect: the daemon returns 404 for `/api/watch/:uuid` until the session's
 * JSONL exists on disk (a brand-new session hasn't been flushed yet). A non-2xx
 * response is a *hard* EventSource failure — the browser fires `onerror`, sets
 * readyState=CLOSED, and never retries. So the hub reconnects on error with a
 * fixed backoff until the stream establishes.
 */

/** The slice of EventSource the hub actually uses — lets tests inject a fake. */
export interface WatchSource {
  onmessage: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  addEventListener(type: string, cb: () => void): void;
  close(): void;
}

/** Injectable side-effects, so the hub is unit-testable without a browser. */
export interface WatchDeps {
  makeSource: (url: string) => WatchSource;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (h: unknown) => void;
}

type Listener = () => void;

interface Entry {
  source: WatchSource | null;
  listeners: Set<Listener>;
  reconnectTimer: unknown | null;
}

/** Reconnect backoff after a hard SSE failure (e.g. the pre-flush 404). */
export const RECONNECT_MS = 1500;

export interface WatchHub {
  /** Follow a session; `onChange` fires on every write. Returns an unsubscribe fn. */
  subscribe(uuid: string, onChange: Listener): () => void;
  /** Number of currently-open EventSources (for tests/introspection). */
  openCount(): number;
}

export function createWatchHub(deps: WatchDeps): WatchHub {
  const registry = new Map<string, Entry>();

  function connect(uuid: string, entry: Entry): void {
    const src = deps.makeSource(`/api/watch/${encodeURIComponent(uuid)}`);
    entry.source = src;
    const fire = () => {
      // Copy guard: a listener may unsubscribe during iteration.
      for (const l of [...entry.listeners]) l();
    };
    src.onmessage = fire;
    src.addEventListener("change", fire);
    src.onerror = () => {
      // Torn down (last unsubscribe) while an error was in flight — do nothing.
      if (registry.get(uuid) !== entry) return;
      src.close();
      if (entry.source === src) entry.source = null;
      if (entry.reconnectTimer === null) {
        entry.reconnectTimer = deps.setTimer(() => {
          entry.reconnectTimer = null;
          // Only reconnect if still registered and someone still cares.
          if (registry.get(uuid) === entry && entry.listeners.size > 0) {
            connect(uuid, entry);
          }
        }, RECONNECT_MS);
      }
    };
  }

  function teardown(uuid: string, entry: Entry): void {
    if (entry.reconnectTimer !== null) {
      deps.clearTimer(entry.reconnectTimer);
      entry.reconnectTimer = null;
    }
    entry.source?.close();
    entry.source = null;
    registry.delete(uuid);
  }

  return {
    subscribe(uuid, onChange) {
      let entry = registry.get(uuid);
      if (!entry) {
        entry = { source: null, listeners: new Set(), reconnectTimer: null };
        registry.set(uuid, entry);
        connect(uuid, entry);
      }
      entry.listeners.add(onChange);
      let live = true;
      return () => {
        if (!live) return; // idempotent unsubscribe
        live = false;
        const e = registry.get(uuid);
        if (!e) return;
        e.listeners.delete(onChange);
        if (e.listeners.size === 0) teardown(uuid, e);
      };
    },
    openCount() {
      let n = 0;
      for (const e of registry.values()) if (e.source) n++;
      return n;
    },
  };
}

/** App-wide singleton, backed by the real EventSource and timers. */
export const watchHub: WatchHub = createWatchHub({
  makeSource: (url) => new EventSource(url) as unknown as WatchSource,
  setTimer: (fn, ms) => setTimeout(fn, ms),
  clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
});

/** Convenience: follow a session through the shared hub. Returns unsubscribe. */
export function subscribeWatch(uuid: string, onChange: Listener): () => void {
  return watchHub.subscribe(uuid, onChange);
}
