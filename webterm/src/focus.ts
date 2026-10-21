/**
 * focus.ts — deep-linking a session to the front.
 *
 * Two ways in, one action (the shell's: open or activate the session's tab and
 * scroll its rail row into view):
 *
 * - a `focus` event on the daemon's bus, sent by `POST /api/focus` (Stead's
 *   notification click). It names a session uuid, or a live pty id for a session
 *   launched headless (`POST /api/pty`) that has no uuid yet. Every open window
 *   gets it; only the most recently
 *   focused one acts, so two windows don't both jump. A `replayed` event (held
 *   by the daemon while no window was connected, handed to the first one to
 *   connect) is acted on by whoever gets it, since nobody else will.
 * - `?session=<uuid>` on the app URL, once at startup, then stripped from the
 *   address bar so a reload doesn't re-trigger it.
 *
 * "Most recently focused" is shared across windows through localStorage: each
 * window stamps its entry on focus and heartbeats while open, and drops it on
 * pagehide. document.hasFocus() alone isn't enough — when the click arrives the
 * notification centre has focus, not any window. Pure pieces are unit-tested in
 * focus.test.ts.
 */

import type { EventRecord } from "./types.ts";

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** A canonical 8-4-4-4-12 hex uuid (Claude session ids, Codex thread ids). */
export function isSessionUuid(s: unknown): s is string {
  return typeof s === "string" && UUID_RE.test(s);
}

/** What a deep link asks to bring forward: a session by uuid, or a live pty by id. */
export type FocusTarget = { session: string } | { pty: string };

const PTY_ID_RE = /^\d+$/;

/** The target a `focus` event asks for (a session uuid lowercased), else null. */
export function focusTarget(ev: EventRecord): FocusTarget | null {
  if (ev.kind !== "focus") return null;
  const s = ev.data?.session;
  if (isSessionUuid(s)) return { session: s.toLowerCase() };
  const p = ev.data?.pty;
  return typeof p === "string" && PTY_ID_RE.test(p) ? { pty: p } : null;
}

/**
 * Split `?session=<uuid>` off a location.search string. `rest` is the search to
 * put back (other params kept; "" when none remain). A malformed value is still
 * stripped, but yields no uuid.
 */
export function sessionFromSearch(search: string): { uuid: string | null; rest: string } | null {
  const params = new URLSearchParams(search);
  if (!params.has("session")) return null;
  const v = params.get("session");
  params.delete("session");
  const q = params.toString();
  return { uuid: isSessionUuid(v) ? v.toLowerCase() : null, rest: q ? `?${q}` : "" };
}

// ---------------------------------------------------------------------------
// Which window acts: the shared "most recently focused" registry
// ---------------------------------------------------------------------------

/** One window's registry entry: when it last had focus, when it last checked in. */
export interface WindowStamp {
  focusedAt: number;
  aliveAt: number;
}

export type WindowRegistry = Record<string, WindowStamp>;

/** An entry not heartbeated for this long is a window that died without
 *  pagehide (a crash). Generous because hidden windows' timers are throttled to
 *  as little as once a minute. */
export const STALE_MS = 5 * 60_000;
export const HEARTBEAT_MS = 30_000;

/** True when `me` is the live window that was focused most recently (ties go to
 *  the lower id, so exactly one window wins). With no live entries — storage
 *  unavailable — every window acts. */
export function isMostRecent(reg: WindowRegistry, me: string, now: number): boolean {
  let best: string | null = null;
  for (const [id, s] of Object.entries(reg)) {
    if (now - s.aliveAt > STALE_MS && id !== me) continue;
    const b = best === null ? null : reg[best]!;
    if (b === null || s.focusedAt > b.focusedAt || (s.focusedAt === b.focusedAt && id < best!)) {
      best = id;
    }
  }
  return best === null || best === me;
}

/** Drop dead windows' entries. */
export function pruneRegistry(reg: WindowRegistry, now: number): WindowRegistry {
  const out: WindowRegistry = {};
  for (const [id, s] of Object.entries(reg)) if (now - s.aliveAt <= STALE_MS) out[id] = s;
  return out;
}

const LS_WINDOWS = "eigenform:term:windows:v1";

function readRegistry(): WindowRegistry {
  try {
    const v = JSON.parse(localStorage.getItem(LS_WINDOWS) ?? "{}") as unknown;
    return v && typeof v === "object" ? (v as WindowRegistry) : {};
  } catch {
    return {};
  }
}

function writeRegistry(reg: WindowRegistry) {
  try {
    localStorage.setItem(LS_WINDOWS, JSON.stringify(reg));
  } catch {
    // Storage full or blocked: hasFocus() still decides for a focused window.
  }
}

export interface WindowPresence {
  /** Should this window act on a live (non-replayed) focus event? */
  shouldAct(): boolean;
}

/**
 * Register this window in the shared registry and keep its entry current. A
 * freshly loaded window counts as just focused (it's the one the user opened).
 */
export function trackWindowPresence(): WindowPresence {
  const me = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  let focusedAt = Date.now();

  const stamp = () => {
    const now = Date.now();
    const reg = pruneRegistry(readRegistry(), now);
    reg[me] = { focusedAt, aliveAt: now };
    writeRegistry(reg);
  };

  stamp();
  window.addEventListener("focus", () => {
    focusedAt = Date.now();
    stamp();
  });
  setInterval(stamp, HEARTBEAT_MS);
  window.addEventListener("pagehide", () => {
    const reg = readRegistry();
    delete reg[me];
    writeRegistry(reg);
  });
  // bfcache restore: pagehide removed us, so check back in.
  window.addEventListener("pageshow", (e) => {
    if (e.persisted) stamp();
  });

  return {
    shouldAct() {
      if (document.hasFocus()) return true;
      stamp(); // our own entry is live by definition
      return isMostRecent(readRegistry(), me, Date.now());
    },
  };
}
