/**
 * shell-helpers.ts — Pure helpers for shell.ts, split out so they can be
 * tested with node --test without pulling in @xterm/xterm (which is a CJS
 * module that node --test cannot resolve as named ESM exports).
 *
 * No DOM, no fetch, no Date.now() — these are all pure functions.
 */

import type { PtyInfo } from "./types.ts";

// ---------------------------------------------------------------------------
// relativeRecency
// ---------------------------------------------------------------------------

/**
 * Compact relative time from an ISO-8601 string (rail/tab recency column).
 * Handles both "Z" and "+00:00" backend forms via Date.parse.
 *
 * Tiers:
 *   < 60s    → "just now"
 *   1–59m    → "Nm"
 *   1–23h    → "Nh"
 *   ≥ 24h    → "Nd"
 */
export function relativeRecency(iso: string, now: number): string {
  const ts = new Date(iso).getTime();
  if (isNaN(ts)) return "";
  const diff = Math.max(0, now - ts);
  const s = Math.floor(diff / 1000);
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.floor(h / 24);
  return `${d}d`;
}

// ---------------------------------------------------------------------------
// ageGroup
// ---------------------------------------------------------------------------

export type AgeGroup = "today" | "week" | "earlier";

/**
 * Bucket a recency timestamp for the rail's group headers.
 * Duration thresholds (not calendar boundaries — keeps this timezone-free):
 *   < 24h → "today", < 7d → "week", else (or unparseable) → "earlier".
 */
export function ageGroup(iso: string, now: number): AgeGroup {
  const ts = new Date(iso).getTime();
  if (isNaN(ts)) return "earlier";
  const diff = Math.max(0, now - ts);
  const h = diff / 3_600_000;
  if (h < 24) return "today";
  if (h < 24 * 7) return "week";
  return "earlier";
}

// ---------------------------------------------------------------------------
// railFromPointer
// ---------------------------------------------------------------------------

export const RAIL_MIN = 180;
export const RAIL_MAX = 480;
export const RAIL_DEFAULT = 244;
/** Dragging the splitter left of this hides the rail entirely. */
export const RAIL_COLLAPSE_AT = 110;

export interface RailDrag {
  collapsed: boolean;
  /** Rail width in px. When collapsed, the width is preserved so re-expanding
   *  (drag right / topbar button) restores the previous size. */
  w: number;
}

/**
 * Map a pointer x position (px from the shell's left edge) to rail state
 * during a splitter drag. Below RAIL_COLLAPSE_AT the rail collapses (keeping
 * `prevW` for restore); otherwise the width tracks the pointer, clamped to
 * [RAIL_MIN, RAIL_MAX].
 */
export function railFromPointer(x: number, prevW: number): RailDrag {
  if (x < RAIL_COLLAPSE_AT) return { collapsed: true, w: prevW };
  return { collapsed: false, w: Math.min(RAIL_MAX, Math.max(RAIL_MIN, Math.round(x))) };
}

// ---------------------------------------------------------------------------
// drawerWidthFromPointer — width of the right-docked drawer during a drag.
// ---------------------------------------------------------------------------

export const DRAWER_MIN_W = 320;
export const DRAWER_MAX_W = 900;
export const DRAWER_DEFAULT_W = 460;

/**
 * Map a pointer x (px from the viewport's left) to the docked drawer's width,
 * given the drawer container's right edge. The drawer grows to the LEFT, so its
 * width is the gap between the pointer and the right edge, clamped to
 * [DRAWER_MIN_W, DRAWER_MAX_W].
 */
export function drawerWidthFromPointer(x: number, containerRight: number): number {
  return Math.min(DRAWER_MAX_W, Math.max(DRAWER_MIN_W, Math.round(containerRight - x)));
}

// ---------------------------------------------------------------------------
// splitHeightFromPointer — reach-region height for the in-drawer vertical split.
// ---------------------------------------------------------------------------

export const REACH_MIN_H = 120;
export const TRANSCRIPT_MIN_H = 140;
export const REACH_DEFAULT_H = 260;

/**
 * Map a pointer y (px from the viewport's top) to the reach region's height in
 * the drawer's vertical split, given the split region's top and total height.
 * The reach map sits above the transcript, so its height is the pointer's
 * offset below `regionTop`, clamped to keep the reach map ≥ REACH_MIN_H and the
 * transcript ≥ TRANSCRIPT_MIN_H.
 */
export function splitHeightFromPointer(
  y: number,
  regionTop: number,
  regionHeight: number,
): number {
  const max = Math.max(REACH_MIN_H, regionHeight - TRANSCRIPT_MIN_H);
  return Math.min(max, Math.max(REACH_MIN_H, Math.round(y - regionTop)));
}

// ---------------------------------------------------------------------------
// inkFor
// ---------------------------------------------------------------------------

/** Per-project ink hues — each key has a matching CSS var --ink-<key>. */
export const INK_KEYS = ["clay", "ochre", "olive", "teal", "slate", "plum"] as const;

export type InkKey = (typeof INK_KEYS)[number];

/**
 * Deterministic ink hue: a stable key (the full cwd path, else a label/chip)
 * hashes to one of INK_KEYS, so every session in the same project shares a
 * color across the rail and tab strip. FNV-1a keeps nearby keys from clumping
 * the way a char-code sum would.
 */
export function inkFor(key: string): InkKey {
  let h = 0x811c9dc5;
  for (let i = 0; i < key.length; i++) {
    h ^= key.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return INK_KEYS[(h >>> 0) % INK_KEYS.length]!;
}

// ---------------------------------------------------------------------------
// reconcileTabs
// ---------------------------------------------------------------------------

/**
 * A tab descriptor saved to localStorage so tabs survive page reload.
 * ptyId: the live registry id at the time the tab was created.
 * uuid:  the session uuid, once resolved.
 * label: display label (may be derived or user-overridden).
 */
export interface TabDescriptor {
  ptyId?: string;
  uuid?: string;
  label: string;
  /** Full cwd path when known — drives the terminal-header breadcrumb. */
  cwd?: string;
}

/** The outcome of reconciling a saved tab against current live ptys. */
export type TabReconcileAction =
  | { action: "attach"; descriptor: TabDescriptor & { ptyId: string } }
  | { action: "resume"; descriptor: TabDescriptor & { uuid: string } }
  | { action: "drop"; descriptor: TabDescriptor };

/**
 * Reconcile saved tab descriptors against current live ptys.
 *
 * For each saved tab:
 *   - If saved.ptyId is live OR saved.uuid matches a live pty's uuid:
 *       action="attach" (live pty available); descriptor.ptyId is resolved.
 *   - Else if saved.uuid exists (pty died but session disk-resident):
 *       action="resume" (reconnect via ?session=uuid).
 *   - Else:
 *       action="drop" (no way to reopen).
 */
export function reconcileTabs(
  saved: TabDescriptor[],
  ptys: PtyInfo[],
): TabReconcileAction[] {
  const ptyById = new Map<string, PtyInfo>(ptys.map((p) => [p.id, p]));
  const ptyByUuid = new Map<string, PtyInfo>(
    ptys.filter((p) => p.uuid !== null).map((p) => [p.uuid!, p]),
  );

  return saved.map((desc): TabReconcileAction => {
    // Live pty by id?
    if (desc.ptyId && ptyById.has(desc.ptyId)) {
      return {
        action: "attach",
        descriptor: { ...desc, ptyId: desc.ptyId },
      };
    }

    if (desc.uuid) {
      // Live pty by uuid (ptyId may have changed after daemon restart)?
      const liveByUuid = ptyByUuid.get(desc.uuid);
      if (liveByUuid) {
        return {
          action: "attach",
          descriptor: { ...desc, ptyId: liveByUuid.id, uuid: desc.uuid },
        };
      }
      // pty gone, but uuid means a disk session exists to resume.
      return {
        action: "resume",
        descriptor: { ...desc, uuid: desc.uuid },
      };
    }

    return { action: "drop", descriptor: desc };
  });
}

/**
 * Pick the `/pty` query that re-opens a single dropped socket, or null if the
 * tab is unrecoverable. This is the single-tab analogue of {@link reconcileTabs}:
 * the reconnect loop reconciles ONE descriptor against the freshly-fetched live
 * ptys and hands the result straight to `connectPty`.
 *
 *   - pty still live (idle drop / WSL2 localhost reaping) → `?attach=<liveId>`,
 *     matching on id first, then on uuid (the id renumbers across a daemon
 *     restart that kept the child alive).
 *   - pty gone but uuid known (daemon restart) → `?session=<uuid>` to resume
 *     `claude --resume` from disk.
 *   - otherwise (ephemeral pty, no uuid) → null: nothing to reconnect to.
 */
export function reconnectQuery(
  desc: TabDescriptor,
  ptys: PtyInfo[],
): string | null {
  const [action] = reconcileTabs([desc], ptys);
  if (!action) return null;
  if (action.action === "attach") return `?attach=${action.descriptor.ptyId}`;
  if (action.action === "resume") {
    return `?session=${encodeURIComponent(action.descriptor.uuid)}`;
  }
  return null;
}

/** Base delay before the first reconnect attempt (attempt 0), in ms. */
export const RECONNECT_BASE_MS = 500;
/**
 * Cap on the reconnect backoff, in ms. A daemon down for a slow `cargo watch`
 * rebuild keeps getting polled at this steady interval rather than backing off
 * into minutes — it mirrors the 3s roster poll.
 */
export const RECONNECT_CAP_MS = 3000;

/**
 * Capped exponential backoff for the reconnect loop. Attempt 0 is the first
 * retry; the delay doubles each attempt and saturates at {@link RECONNECT_CAP_MS}.
 * Negative attempts are clamped to the base delay (defensive).
 */
export function reconnectDelay(attempt: number): number {
  const n = Math.max(0, attempt);
  return Math.min(RECONNECT_CAP_MS, RECONNECT_BASE_MS * 2 ** n);
}
