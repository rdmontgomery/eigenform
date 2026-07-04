/**
 * events.ts — the Events pane: a live tail of the daemon's structured event
 * stream, docked below the transcript.
 *
 * The daemon has no logs; this pane is the browser surface for its event bus.
 * It loads history from GET /api/events on mount, then live-appends from the
 * /api/events/stream SSE. Each row is `time · kind chip · one-line summary`,
 * newest at the bottom with stick-to-bottom scrolling (like the transcript).
 *
 * Unlike the reach map + transcript, events are GLOBAL (not session-scoped), so
 * this pane is mounted once while the dock is open, independent of the active
 * tab's uuid.
 *
 * The summary/time formatting are pure functions (eventSummary, fmtEventTime),
 * unit-tested in events.test.ts. eventSummary is deliberately total: a kind it
 * doesn't special-case still renders a compact key:value summary, so a future
 * branch's new event kind shows up legibly with no change here.
 *
 * XSS safety: every field is set via textContent, never innerHTML.
 */

import type { EventRecord } from "./types.ts";

// ---------------------------------------------------------------------------
// Pure formatting helpers (unit-tested)
// ---------------------------------------------------------------------------

/** First 8 chars of a string value (a uuid/turn stub), else "". */
export function shortId(v: unknown): string {
  return typeof v === "string" ? v.slice(0, 8) : "";
}

/** Local wall-clock `HH:MM:SS` for an ISO-8601 timestamp; "" when unparseable. */
export function fmtEventTime(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const d = new Date(t);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** Compact scalar rendering for the generic fallback summary. */
function compactVal(v: unknown): string {
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (v === null || v === undefined) return "";
  return JSON.stringify(v);
}

/** Generic `key: value · key: value` fallback for kinds we don't special-case. */
function summarizeData(data: Record<string, unknown>): string {
  return Object.entries(data)
    .map(([k, v]) => `${k}: ${compactVal(v)}`)
    .filter((s) => !s.endsWith(": "))
    .join(" · ");
}

/**
 * A compact one-line summary of an event's payload, per kind. Total: an
 * unrecognized (e.g. future) kind falls through to a generic key:value dump so
 * it still reads well without a code change here.
 */
export function eventSummary(ev: EventRecord): string {
  const d = ev.data ?? {};
  const str = (k: string): string | undefined =>
    typeof d[k] === "string" ? (d[k] as string) : undefined;

  switch (ev.kind) {
    case "pty-spawned":
      return [`pty ${str("id") ?? "?"}`, str("program"), str("cwd")]
        .filter(Boolean)
        .join(" · ");
    case "pty-exited":
      return `pty ${str("id") ?? "?"}`;
    case "session-uuid-adopted":
      return `pty ${str("ptyId") ?? "?"} → ${shortId(d.uuid)} (${str("source") ?? "?"})`;
    case "fork-created":
      return `${shortId(d.srcUuid)} → ${shortId(d.branchUuid)} @ ${shortId(d.turn)}`;
    case "spawn-refused":
    case "resume-refused": {
      const where = str("cwd") ?? (str("session") ? shortId(d.session) : undefined);
      return [str("reason") ?? "refused", where].filter(Boolean).join(" · ");
    }
    default:
      return summarizeData(d);
  }
}

// ---------------------------------------------------------------------------
// Pane
// ---------------------------------------------------------------------------

export interface EventsHandle {
  /** Tear down the SSE + timers and remove the DOM. Safe to call twice. */
  close(): void;
}

const LS_EVENTS = "eigenform:term:events:v1"; // "open" | "closed" (default closed)
const STICK_THRESHOLD = 60; // px from bottom — re-stick after append if closer
const RECONNECT_MS = 1500;

/**
 * Mount the Events pane into `hostEl` (its region in the dock). Loads history,
 * then tails the SSE. Caller owns teardown via the returned handle.
 */
export function mountEvents(hostEl: HTMLElement): EventsHandle {
  let closed = false;
  let lastSeq = 0;
  let count = 0;

  // ── DOM ────────────────────────────────────────────────────────────────
  const panel = el("div", "events-pane");
  let collapsed = localStorage.getItem(LS_EVENTS) !== "open"; // default collapsed

  const head = el("div", "events-head");
  head.setAttribute("role", "button");
  head.tabIndex = 0;
  const caret = el("span", "events-caret");
  caret.textContent = "▸";
  const title = el("span", "events-title");
  title.textContent = "Events";
  const countEl = el("span", "events-count");
  head.append(caret, title, countEl);

  const body = el("div", "events-body scroll");

  panel.append(head, body);
  hostEl.append(panel);

  function applyCollapsed() {
    panel.classList.toggle("events-pane--collapsed", collapsed);
    caret.textContent = collapsed ? "▸" : "▾";
    head.title = collapsed ? "Show events" : "Hide events";
  }
  applyCollapsed();

  function setCollapsed(c: boolean) {
    collapsed = c;
    localStorage.setItem(LS_EVENTS, c ? "closed" : "open");
    applyCollapsed();
    if (!c) body.scrollTop = body.scrollHeight; // re-stick on expand
  }
  head.addEventListener("click", () => setCollapsed(!collapsed));
  head.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      setCollapsed(!collapsed);
    }
  });

  // ── Rows ───────────────────────────────────────────────────────────────
  function updateCount() {
    countEl.textContent = count === 1 ? "1" : String(count);
  }

  function isNearBottom(): boolean {
    return body.scrollHeight - body.scrollTop - body.clientHeight < STICK_THRESHOLD;
  }

  function rowEl(ev: EventRecord): HTMLElement {
    const row = el("div", "event-row");
    const time = el("span", "event-time");
    time.textContent = fmtEventTime(ev.at);
    const kind = el("span", "event-kind");
    kind.textContent = ev.kind;
    const summary = el("span", "event-summary");
    summary.textContent = eventSummary(ev);
    summary.title = summary.textContent;
    row.append(time, kind, summary);
    return row;
  }

  /** Append one event if it's newer than the high-water seq (dedup on reconnect). */
  function append(ev: EventRecord) {
    if (typeof ev.seq !== "number" || ev.seq <= lastSeq) return;
    lastSeq = ev.seq;
    count += 1;
    const stick = isNearBottom();
    body.append(rowEl(ev));
    updateCount();
    if (stick) body.scrollTop = body.scrollHeight;
  }

  /** Replace the list wholesale (initial history load). */
  function renderAll(list: EventRecord[]) {
    body.innerHTML = "";
    lastSeq = 0;
    count = 0;
    for (const ev of list) {
      if (typeof ev.seq !== "number") continue;
      lastSeq = Math.max(lastSeq, ev.seq);
      count += 1;
      body.append(rowEl(ev));
    }
    updateCount();
    body.scrollTop = body.scrollHeight;
  }

  // ── Data: history load + SSE tail ──────────────────────────────────────
  async function loadHistory() {
    try {
      const res = await fetch("/api/events");
      if (!res.ok || closed) return;
      renderAll((await res.json()) as EventRecord[]);
    } catch {
      // Daemon unreachable — the SSE reconnect will backfill once it's up.
    }
  }

  /** Pull anything recorded while the stream was down, then it's caught up. */
  async function backfill() {
    try {
      const res = await fetch(`/api/events?since=${lastSeq}`);
      if (!res.ok || closed) return;
      for (const ev of (await res.json()) as EventRecord[]) append(ev);
    } catch {
      // Still down — the next reconnect tick tries again.
    }
  }

  let es: EventSource | null = null;
  let reconnectTimer: number | null = null;

  function connect() {
    if (closed) return;
    es = new EventSource("/api/events/stream");
    es.onmessage = (e) => {
      try {
        append(JSON.parse(e.data) as EventRecord);
      } catch {
        // Ignore a malformed frame.
      }
    };
    es.onerror = () => {
      // /api/events/stream is always a 200 (the bus always exists), so an error
      // here is a transport drop (daemon restart, idle reaping). Close, then
      // reconnect after a backoff, backfilling the gap via ?since=lastSeq.
      es?.close();
      es = null;
      if (reconnectTimer === null) {
        reconnectTimer = window.setTimeout(() => {
          reconnectTimer = null;
          if (closed) return;
          void backfill().then(connect);
        }, RECONNECT_MS);
      }
    };
  }

  void loadHistory();
  connect();

  return {
    close() {
      if (closed) return;
      closed = true;
      if (reconnectTimer !== null) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      es?.close();
      es = null;
      panel.remove();
    },
  };
}

// ---------------------------------------------------------------------------
// DOM utility (local — same pattern as shell.ts / drawer.ts)
// ---------------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}
