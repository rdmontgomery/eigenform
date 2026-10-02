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
