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
 * Human-readable relative time from an ISO-8601 string.
 * Handles both "Z" and "+00:00" backend forms via Date.parse.
 *
 * Tiers:
 *   < 60s    → "just now"
 *   1–59m    → "Nm ago"
 *   1–23h    → "Nh ago"
 *   ≥ 24h    → "Nd ago"
 */
export function relativeRecency(iso: string, now: number): string {
  const ts = new Date(iso).getTime();
  if (isNaN(ts)) return "";
  const diff = Math.max(0, now - ts);
  const s = Math.floor(diff / 1000);
  if (s < 60) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  return `${d}d ago`;
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
