// Status presentation shared by the rail and the tab strip: the two-channel
// status dot (activity × liveness), its hover text, the live-row tag, and the
// per-project ink hue. Pure string builders — no DOM.

import type { Activity, Liveness } from "./roster.ts";
import { inkFor } from "./shell-helpers.ts";

const KNOWN_ACTIVITY = new Set(["working", "waiting", "idle"]);

/**
 * CSS classes for a status dot across the two orthogonal channels:
 *   activity → color + glow (`dot--working|waiting|idle`)
 *   liveness → fill        (`dot--eigenform|external|dead`)
 * Only live provenances (eigenform/external) animate; dead never glows.
 */
export function dotClasses(activity: string, liveness: Liveness): string {
  const act = KNOWN_ACTIVITY.has(activity) ? activity : "idle";
  const prov = liveness === "eigenform" ? "eigenform" : liveness === "external" ? "external" : "dead";
  return `dot dot--${act} dot--${prov}`;
}

/** Short turn-state tag for a live row's meta line; null for dead rows.
 *  External (live outside eigenform) rows are prefixed so provenance reads at a
 *  glance without hovering, complementing the hollow-ring dot. */
export function livenessTag(activity: Activity, liveness: Liveness): string | null {
  if (liveness === "none") return null;
  const turn = activity === "working" ? "running" : activity === "waiting" ? "your turn" : "live";
  return liveness === "external" ? `· ext · ${turn}` : `· ${turn}`;
}

/** Full hover explanation of a dot's combined state. */
export function dotTitle(activity: Activity, liveness: Liveness): string {
  const where =
    liveness === "eigenform"
      ? "eigenform session"
      : liveness === "external"
        ? "running outside eigenform — can't attach"
        : "no live process";
  if (liveness === "none") return where;
  const turn =
    activity === "working"
      ? "assistant running"
      : activity === "waiting"
        ? "waiting for your input"
        : "idle at prompt";
  return `${turn} — ${where}`;
}

/** The session's ink hue CSS value, from its most durable key. */
// Color = project: hash on the full cwd path so every session in the same
// directory shares one hue (in both the rail and the tab strip). Falls back to
// a label/chip when the cwd is unknown. Hashing the full path (not the basename)
// keeps unrelated `…/src` dirs from colliding.
export function inkVar(cwd: string | undefined, fallback: string): string {
  return `var(--ink-${inkFor(cwd ?? fallback)})`;
}

/** A tab's prompt-cache clock, ready to render. */
export interface CacheClock {
  /** Minutes since the last API response ("<1m", "47m"), or "cold" past the TTL. */
  text: string;
  /** warm < 75% of the TTL ≤ warn < 90% ≤ urgent < 100% ≤ cold. */
  level: "warm" | "warn" | "urgent" | "cold";
  title: string;
}

const DEFAULT_TTL_SECS = 3600;

function ttlLabel(secs: number): string {
  return secs % 3600 === 0 ? `${secs / 3600}h` : `${Math.round(secs / 60)}m`;
}

/**
 * The prompt-cache clock for a live session: time since its last API response
 * (`cacheAt`, the last real assistant row), graded against the TTL Claude Code
 * cached at (`ttlSecs`; 1h assumed when the transcript didn't say). Every API
 * call refreshes the cache, so the clock only runs between turns: null while a
 * turn is in flight, or when there is no response yet.
 */
export function cacheClock(
  cacheAt: string | null | undefined,
  ttlSecs: number | null | undefined,
  working: boolean,
  now: number,
): CacheClock | null {
  if (working || !cacheAt) return null;
  const at = Date.parse(cacheAt);
  if (Number.isNaN(at)) return null;
  const ttl = ttlSecs ?? DEFAULT_TTL_SECS;
  const ttlText = ttlLabel(ttl) + (ttlSecs ? "" : " (assumed)");
  const elapsed = Math.max(0, (now - at) / 1000);
  const frac = elapsed / ttl;
  const mins = Math.floor(elapsed / 60);
  if (frac >= 1) {
    return {
      text: "cold",
      level: "cold",
      title: `Prompt cache expired (${ttlText} TTL): the next prompt re-writes the whole context`,
    };
  }
  const left = Math.ceil((ttl - elapsed) / 60);
  return {
    text: mins < 1 ? "<1m" : `${mins}m`,
    level: frac >= 0.9 ? "urgent" : frac >= 0.75 ? "warn" : "warm",
    title: `Prompt cache: last API call ${mins < 1 ? "under a minute" : `${mins}m`} ago · ${ttlText} TTL · expires in ${left}m`,
  };
}
