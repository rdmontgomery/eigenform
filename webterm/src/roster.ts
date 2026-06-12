/**
 * roster.ts — Pure roster data layer: merges registry ptys + forest items into
 * a sorted, labelled list suitable for the sidebar.
 *
 * PURE: no fetch, no DOM, no Date.now(). Recency sorting is lexicographic on
 * ISO-8601 strings (rfc3339). This is correct because: rfc3339 timestamps are
 * monotonically comparable as ASCII strings when in the same UTC-offset
 * representation. The backend emits +00:00 offsets (not the "Z" short-form),
 * but both serialize identically to the same byte-order: lexicographic
 * comparison within a group (all same-offset) gives chronological order.
 *
 * STATE NOTES:
 * - Live rows use PtyState ("working"|"waiting"|"idle"|"exited") from the
 *   registry.  "waiting" appears only for active claude sessions.
 * - Disk-only forest rows use the state string from the JSONL (same union, but
 *   not guaranteed by the type — kept as `string` in RosterRow to stay honest).
 * - A forest row may carry live=true (claude running outside our registry).
 *   We CANNOT attach to such sessions, so they are NOT promoted to the live
 *   group. They appear in the forest (disk) group with their forest state
 *   preserved. This is intentional and documented.
 *
 * OVERRIDE KEYS (v1):
 * - Overrides are keyed by session uuid. UUID is the durable identifier.
 * - As a fallback, overrides may also be keyed by ptyId. This supports shell
 *   callers that only know the pty fd number before the uuid is resolved.
 * - When both keys are present in `overrides`, uuid wins.
 * - Rows without uuid (pty spawned but not yet reconciled) can accept a ptyId
 *   override, but this override is NOT durable — if the daemon restarts and
 *   reassigns ids, the override key will be stale. Document this to callers.
 *
 * DUPLICATE UUID (fork weirdness / --resume twice):
 * - If two registry ptys claim the same uuid, the one with the higher
 *   lastActivity ISO string wins the merge slot (lexicographic = chronological
 *   for UTC rfc3339). The losing pty becomes an independent live row with its
 *   uuid cleared (to prevent re-merging) and is sorted in the live group by its
 *   own lastActivity.
 */

import type { PtyInfo, ForestItem } from "./types.ts";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

export interface RosterRow {
  /** Stable rendering key: "pty:<id>" for registry rows, "uuid:<uuid>" for
   *  disk-only rows. After merge, the merged row uses "pty:<id>" (live wins). */
  key: string;
  label: string;
  cwdChip: string;
  /** Full cwd path when known (live pty cwd, else forest cwd). Drives the
   *  terminal-header breadcrumb; cwdChip remains the short rail chip. */
  cwd?: string;
  /** State string. For live rows: PtyState. For disk-only rows: forest state
   *  string. Kept as string (not a union) to stay honest about provenance. */
  state: string;
  /** True when this row is backed by a live registry pty. Forest-only rows with
   *  forest.live=true are NOT promoted — they remain live=false in the roster. */
  live: boolean;
  ptyId?: string;
  /**
   * Session uuid, if known. Absent (undefined) when the pty has not yet been
   * reconciled to a session uuid. Note: PtyInfo.uuid is null (from the registry
   * JSON); RosterRow normalises this to undefined-when-absent. Renderers should
   * use truthiness (`if (row.uuid)`) to guard uuid-dependent actions.
   */
  uuid?: string;
  /** ISO-8601 string used for within-group sorting (most-recent first).
   *  For live rows: lastActivity from pty. For disk-only rows: recency from
   *  forest. Lexicographic comparison within a group is correct because all
   *  timestamps in the group share the same UTC offset (+00:00 from the backend),
   *  making byte-order equivalent to chronological order. */
  recency: string;
}

// ---------------------------------------------------------------------------
// deriveLabel input
// ---------------------------------------------------------------------------

export interface LabelInput {
  aiTitle?: string | null;
  cwd?: string | null;
  firstPrompt?: string | null;
  override?: string | null;
}

// Max length for firstPrompt snippet before truncation with ellipsis.
// 40 chars chosen as a comfortable sidebar width budget — document: callers
// may want to re-truncate for their display context.
const SNIPPET_MAX = 40;

/** Return the basename of a path, or "" for null/root paths. */
function basename(cwd: string | null): string {
  if (!cwd) return "";
  const trimmed = cwd.replace(/\/+$/, "");
  const slash = trimmed.lastIndexOf("/");
  const base = slash >= 0 ? trimmed.slice(slash + 1) : trimmed;
  return base;
}

// ---------------------------------------------------------------------------
// deriveLabel
// ---------------------------------------------------------------------------

/**
 * Derive a display label from available session metadata.
 *
 * Degradation chain (highest to lowest priority):
 *   override → aiTitle → cwd basename → firstPrompt snippet → "new session"
 *
 * firstPrompt is trimmed and capped at SNIPPET_MAX (40) chars with "..."
 * appended if truncated.
 */
export function deriveLabel(input: LabelInput): string {
  // 1. User override beats everything.
  if (input.override) return input.override;

  // 2. AI-generated title from the JSONL transcript.
  if (input.aiTitle) return input.aiTitle;

  // 3. CWD basename.
  if (input.cwd) {
    const base = basename(input.cwd);
    if (base) return base;
    // cwd="/" produces empty base — fall through.
  }

  // 4. First-prompt snippet.
  if (input.firstPrompt) {
    const s = input.firstPrompt.trim();
    if (s) {
      return s.length > SNIPPET_MAX ? s.slice(0, SNIPPET_MAX) + "..." : s;
    }
  }

  // 5. Final fallback.
  return "new session";
}

// ---------------------------------------------------------------------------
// buildRoster helpers
// ---------------------------------------------------------------------------

/** Return the cwd basename for a sidebar chip, or "" for null/root. */
function cwdChip(cwd: string | null): string {
  return basename(cwd);
}

// ---------------------------------------------------------------------------
// buildRoster
// ---------------------------------------------------------------------------

/**
 * Build a sorted roster from live registry ptys + disk forest items.
 *
 * Ordering:
 *   1. Live (registry-backed) rows first, sorted by lastActivity DESC.
 *   2. Disk-only (forest) rows second, sorted by recency DESC.
 *
 * Merge: a pty row and a forest row sharing the same uuid are collapsed into
 * one row. The live/registry side wins for state and ptyId; the forest side
 * contributes aiTitle (since AI titles live in the JSONL, not the registry).
 * CWD prefers the pty side (live, current) over the forest side.
 */
export function buildRoster(
  ptys: PtyInfo[],
  forest: ForestItem[],
  overrides: Record<string, string>,
): RosterRow[] {
  // ------------------------------------------------------------------
  // Step 1: Resolve duplicate-uuid ptys.
  //   Group ptys by uuid. For each uuid with multiple claimants, the one with
  //   the highest lastActivity wins; the rest have their uuid cleared.
  // ------------------------------------------------------------------
  interface MutablePty {
    info: PtyInfo;
    uuid: string | null; // may be cleared if this pty lost a uuid contest
  }

  const byUuid = new Map<string, PtyInfo[]>();
  for (const p of ptys) {
    if (p.uuid !== null) {
      const bucket = byUuid.get(p.uuid);
      if (bucket) {
        bucket.push(p);
      } else {
        byUuid.set(p.uuid, [p]);
      }
    }
  }

  const mutPtys: MutablePty[] = ptys.map((p) => ({ info: p, uuid: p.uuid }));

  // For each contested uuid: sort bucket by lastActivity DESC, winner = first.
  for (const [uuid, bucket] of byUuid.entries()) {
    if (bucket.length <= 1) continue;
    // Sort descending by lastActivity (ISO strings compare correctly as ASCII).
    bucket.sort((a, b) => (a.lastActivity > b.lastActivity ? -1 : a.lastActivity < b.lastActivity ? 1 : 0));
    const winnerId = bucket[0]!.id;
    // Clear uuid from all losers.
    for (const mp of mutPtys) {
      if (mp.uuid === uuid && mp.info.id !== winnerId) {
        mp.uuid = null;
      }
    }
  }

  // ------------------------------------------------------------------
  // Step 2: Build a uuid→ForestItem map for merge lookup.
  // ------------------------------------------------------------------
  const forestByUuid = new Map<string, ForestItem>();
  for (const fi of forest) {
    forestByUuid.set(fi.uuid, fi);
  }

  // Track which forest uuids have been merged so we don't emit them again.
  const mergedForestUuids = new Set<string>();

  // ------------------------------------------------------------------
  // Step 3: Emit live (registry-backed) rows.
  // ------------------------------------------------------------------
  const liveRows: RosterRow[] = [];

  for (const mp of mutPtys) {
    const p = mp.info;
    const resolvedUuid = mp.uuid;

    // Attempt merge with a forest item of the same uuid.
    let aiTitle: string | null = null;
    let forestCwd: string | null = null;
    if (resolvedUuid !== null) {
      const fi = forestByUuid.get(resolvedUuid);
      if (fi) {
        aiTitle = fi.title;
        forestCwd = fi.cwd;
        mergedForestUuids.add(resolvedUuid);
      }
    }

    // Choose cwd: pty (live) preferred, forest as fallback.
    const effectiveCwd = p.cwd ?? forestCwd;

    // Resolve override: uuid key first, ptyId key as fallback.
    const overrideVal =
      (resolvedUuid !== null ? overrides[resolvedUuid] : undefined) ??
      overrides[p.id] ??
      null;

    const label = deriveLabel({
      aiTitle,
      cwd: effectiveCwd,
      override: overrideVal,
    });

    const row: RosterRow = {
      key: `pty:${p.id}`,
      label,
      cwdChip: cwdChip(effectiveCwd),
      state: p.state,
      live: true,
      ptyId: p.id,
      recency: p.lastActivity,
    };
    if (effectiveCwd !== null) {
      row.cwd = effectiveCwd;
    }
    if (resolvedUuid !== null) {
      row.uuid = resolvedUuid;
    }

    liveRows.push(row);
  }

  // Sort live rows: most-recent lastActivity first.
  liveRows.sort((a, b) =>
    a.recency > b.recency ? -1 : a.recency < b.recency ? 1 : 0,
  );

  // ------------------------------------------------------------------
  // Step 4: Emit disk-only (forest) rows.
  //   Includes forest rows with live=true that have no matching registry pty
  //   — we cannot attach to them, so they remain in the disk group.
  // ------------------------------------------------------------------
  const diskRows: RosterRow[] = [];

  for (const fi of forest) {
    if (mergedForestUuids.has(fi.uuid)) continue; // already merged

    const overrideVal = overrides[fi.uuid] ?? null;
    const label = deriveLabel({
      aiTitle: fi.title,
      cwd: fi.cwd,
      override: overrideVal,
    });

    diskRows.push({
      key: `uuid:${fi.uuid}`,
      label,
      cwdChip: cwdChip(fi.cwd),
      cwd: fi.cwd,
      state: fi.state,
      // NOTE: forest.live=true here means claude is running outside our
      // registry. We cannot attach, so this row is live=false in the roster.
      live: false,
      uuid: fi.uuid,
      recency: fi.recency,
    });
  }

  // Sort disk rows: most-recent recency first.
  diskRows.sort((a, b) =>
    a.recency > b.recency ? -1 : a.recency < b.recency ? 1 : 0,
  );

  return [...liveRows, ...diskRows];
}
