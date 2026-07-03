// Pure decision helper for auto-staging a Fable retry on a downgraded session.
// No DOM, no fetch — just the "should we fire right now?" gate. The side effects
// (POST /recover-downgrade, open the branch tab, seed the input) live in shell.ts.

/** A minimal view of a forest row for the auto-recover decision. */
export interface DowngradeCandidate {
  uuid: string;
  downgrade?: { offendingTurn: string } | null;
}

/**
 * Decide whether to auto-stage a Fable retry for the ACTIVE session right now.
 * Fires at most once per session (caller records handled uuids), only for the
 * active session, only when a downgrade is present, and never within
 * `recentInputMs` of the user's last keystroke (so it can't yank focus / eat a
 * keypress mid-sentence).
 */
export function shouldAutoRecover(args: {
  activeUuid: string | null;
  rows: DowngradeCandidate[];
  handled: ReadonlySet<string>;
  lastInputAt: number; // epoch ms of the last keystroke into the active pty (0 if none)
  now: number; // epoch ms
  recentInputMs: number; // e.g. 1500
}): { uuid: string; offendingTurn: string } | null {
  const { activeUuid, rows, handled, lastInputAt, now, recentInputMs } = args;
  if (!activeUuid) return null;
  if (handled.has(activeUuid)) return null;
  if (now - lastInputAt < recentInputMs) return null;
  const row = rows.find((r) => r.uuid === activeUuid);
  const d = row?.downgrade;
  if (!d) return null;
  return { uuid: activeUuid, offendingTurn: d.offendingTurn };
}
