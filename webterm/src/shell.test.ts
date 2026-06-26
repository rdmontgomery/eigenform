// Tests for shell.ts pure helpers.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  relativeRecency,
  reconcileTabs,
  reconnectQuery,
  reconnectDelay,
  RECONNECT_BASE_MS,
  RECONNECT_CAP_MS,
  ageGroup,
  inkFor,
  INK_KEYS,
  railFromPointer,
  RAIL_MIN,
  RAIL_MAX,
  RAIL_COLLAPSE_AT,
  drawerWidthFromPointer,
  DRAWER_MIN_W,
  DRAWER_MAX_W,
  splitHeightFromPointer,
  REACH_MIN_H,
  TRANSCRIPT_MIN_H,
} from "./shell-helpers.ts";
import type { PtyInfo } from "./types.ts";

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

function pty(overrides: Partial<PtyInfo> & { id: string }): PtyInfo {
  return {
    cwd: "/home/user/proj",
    uuid: null,
    state: "idle",
    spawnedAt: "2026-06-11T10:00:00+00:00",
    lastActivity: "2026-06-11T10:00:00+00:00",
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// relativeRecency — human-readable relative time
// ---------------------------------------------------------------------------

test("relativeRecency: just now for < 60s", () => {
  const now = 1_000_000_000_000; // arbitrary ms epoch
  const iso = new Date(now - 30_000).toISOString();
  assert.equal(relativeRecency(iso, now), "just now");
});

test("relativeRecency: compact minutes for 1-59m", () => {
  const now = 1_000_000_000_000;
  const iso2m = new Date(now - 2 * 60_000).toISOString();
  const iso59m = new Date(now - 59 * 60_000).toISOString();
  assert.equal(relativeRecency(iso2m, now), "2m");
  assert.equal(relativeRecency(iso59m, now), "59m");
});

test("relativeRecency: compact hours for 1-23h", () => {
  const now = 1_000_000_000_000;
  const iso1h = new Date(now - 1 * 3_600_000).toISOString();
  const iso23h = new Date(now - 23 * 3_600_000).toISOString();
  assert.equal(relativeRecency(iso1h, now), "1h");
  assert.equal(relativeRecency(iso23h, now), "23h");
});

test("relativeRecency: compact days for >= 24h", () => {
  const now = 1_000_000_000_000;
  const iso1d = new Date(now - 24 * 3_600_000).toISOString();
  const iso3d = new Date(now - 3 * 24 * 3_600_000).toISOString();
  assert.equal(relativeRecency(iso1d, now), "1d");
  assert.equal(relativeRecency(iso3d, now), "3d");
});

test("relativeRecency: returns empty string for unparseable ISO (NaN guard)", () => {
  const now = 1_000_000_000_000;
  assert.equal(relativeRecency("not-a-date", now), "");
  assert.equal(relativeRecency("", now), "");
});

test("relativeRecency: handles +00:00 offset strings from backend", () => {
  // Backend emits +00:00, not Z; both must parse correctly.
  const now = 1_000_000_000_000;
  const isoZ = new Date(now - 5 * 60_000).toISOString(); // "Z" form
  const isoOffset = isoZ.replace("Z", "+00:00"); // "+00:00" form
  assert.equal(relativeRecency(isoZ, now), "5m");
  assert.equal(relativeRecency(isoOffset, now), "5m");
});

// ---------------------------------------------------------------------------
// ageGroup — recency bucketing for the rail (Today / This week / Earlier)
// ---------------------------------------------------------------------------

test("ageGroup: under 24h is today", () => {
  const now = 1_000_000_000_000;
  assert.equal(ageGroup(new Date(now - 30_000).toISOString(), now), "today");
  assert.equal(ageGroup(new Date(now - 23 * 3_600_000).toISOString(), now), "today");
});

test("ageGroup: 24h to 7d is week", () => {
  const now = 1_000_000_000_000;
  assert.equal(ageGroup(new Date(now - 25 * 3_600_000).toISOString(), now), "week");
  assert.equal(ageGroup(new Date(now - 6 * 24 * 3_600_000).toISOString(), now), "week");
});

test("ageGroup: beyond 7d is earlier", () => {
  const now = 1_000_000_000_000;
  assert.equal(ageGroup(new Date(now - 8 * 24 * 3_600_000).toISOString(), now), "earlier");
  assert.equal(ageGroup(new Date(now - 90 * 24 * 3_600_000).toISOString(), now), "earlier");
});

test("ageGroup: unparseable recency falls into earlier", () => {
  const now = 1_000_000_000_000;
  assert.equal(ageGroup("not-a-date", now), "earlier");
  assert.equal(ageGroup("", now), "earlier");
});

// ---------------------------------------------------------------------------
// railFromPointer — rail drag-resize state (width clamp + collapse threshold)
// ---------------------------------------------------------------------------

test("railFromPointer: collapses below the threshold, preserving the last width", () => {
  const state = railFromPointer(RAIL_COLLAPSE_AT - 1, 300);
  assert.equal(state.collapsed, true);
  assert.equal(state.w, 300);
});

test("railFromPointer: at or beyond the threshold the rail is visible", () => {
  assert.equal(railFromPointer(RAIL_COLLAPSE_AT, 244).collapsed, false);
});

test("railFromPointer: clamps width to [RAIL_MIN, RAIL_MAX]", () => {
  assert.equal(railFromPointer(RAIL_COLLAPSE_AT, 244).w, RAIL_MIN);
  assert.equal(railFromPointer(9999, 244).w, RAIL_MAX);
  assert.equal(railFromPointer(300, 244).w, 300);
});

test("railFromPointer: rounds fractional pointer positions", () => {
  assert.equal(railFromPointer(300.6, 244).w, 301);
});

// ---------------------------------------------------------------------------
// drawerWidthFromPointer — the docked drawer lives on the RIGHT, so its drag
// handle sits on its left edge. Dragging left widens the drawer; the width is
// the gap between the pointer and the container's right edge, clamped.
// ---------------------------------------------------------------------------

test("drawerWidthFromPointer: width is the gap to the container's right edge", () => {
  // container right edge at 1000, pointer at 600 → 400px drawer.
  assert.equal(drawerWidthFromPointer(600, 1000), 400);
});

test("drawerWidthFromPointer: clamps to [DRAWER_MIN_W, DRAWER_MAX_W]", () => {
  // pointer near the right edge → narrower than the min → clamp up.
  assert.equal(drawerWidthFromPointer(995, 1000), DRAWER_MIN_W);
  // pointer far left → wider than the max → clamp down.
  assert.equal(drawerWidthFromPointer(-9999, 1000), DRAWER_MAX_W);
});

test("drawerWidthFromPointer: rounds fractional pointer positions", () => {
  assert.equal(drawerWidthFromPointer(599.4, 1000), 401);
});

// ---------------------------------------------------------------------------
// splitHeightFromPointer — the reach map sits on top of the transcript inside
// the drawer. The divider drag sets the reach region's height (pointer y minus
// the region's top), clamped so neither pane collapses below its minimum.
// ---------------------------------------------------------------------------

test("splitHeightFromPointer: height is the pointer offset below the region top", () => {
  // region spans y∈[100, 700] (top 100, height 600); pointer at 400 → 300px reach.
  assert.equal(splitHeightFromPointer(400, 100, 600), 300);
});

test("splitHeightFromPointer: clamps up to REACH_MIN_H near the top", () => {
  assert.equal(splitHeightFromPointer(105, 100, 600), REACH_MIN_H);
});

test("splitHeightFromPointer: leaves the transcript at least TRANSCRIPT_MIN_H", () => {
  // region top 100, height 600 → bottom 700. Max reach = 600 - TRANSCRIPT_MIN_H.
  assert.equal(splitHeightFromPointer(9999, 100, 600), 600 - TRANSCRIPT_MIN_H);
});

test("splitHeightFromPointer: rounds fractional pointer positions", () => {
  assert.equal(splitHeightFromPointer(400.6, 100, 600), 301);
});

// ---------------------------------------------------------------------------
// inkFor — deterministic per-session ink hue assignment
// ---------------------------------------------------------------------------

test("inkFor: deterministic for the same key", () => {
  assert.equal(inkFor("abc-123"), inkFor("abc-123"));
});

test("inkFor: always returns a known ink key", () => {
  for (const key of ["a", "uuid-1", "/home/u/projects/eigen", "", "42"]) {
    assert.ok((INK_KEYS as readonly string[]).includes(inkFor(key)));
  }
});

test("inkFor: distinct keys spread across more than one hue", () => {
  const hues = new Set(
    ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"].map(inkFor),
  );
  assert.ok(hues.size > 1, `expected spread, got ${[...hues].join(",")}`);
});

// ---------------------------------------------------------------------------
// reconcileTabs — reload reconciliation
//
// Given saved tab descriptors + current live ptys, produces a list of actions:
//   {action: "attach", descriptor} — ptyId still alive → attach
//   {action: "resume", descriptor} — ptyId gone but uuid exists → session attach
//   {action: "drop", descriptor}   — neither ptyId alive nor uuid known → drop
// ---------------------------------------------------------------------------

test("reconcileTabs: attach when ptyId still live", () => {
  const ptys: PtyInfo[] = [pty({ id: "10", uuid: "uuid-a" })];
  const saved = [{ ptyId: "10", uuid: "uuid-a", label: "My tab" }];
  const actions = reconcileTabs(saved, ptys);
  assert.equal(actions.length, 1);
  assert.equal(actions[0]!.action, "attach");
  assert.equal(actions[0]!.descriptor.ptyId, "10");
});

test("reconcileTabs: resume when ptyId gone but uuid exists", () => {
  const ptys: PtyInfo[] = []; // daemon restarted, pty gone
  const saved = [{ ptyId: "10", uuid: "uuid-a", label: "My tab" }];
  const actions = reconcileTabs(saved, ptys);
  assert.equal(actions.length, 1);
  assert.equal(actions[0]!.action, "resume");
  assert.equal(actions[0]!.descriptor.uuid, "uuid-a");
});

test("reconcileTabs: drop when ptyId gone and no uuid", () => {
  const ptys: PtyInfo[] = [];
  const saved = [{ ptyId: "10", label: "Ephemeral tab" }];
  const actions = reconcileTabs(saved, ptys);
  assert.equal(actions.length, 1);
  assert.equal(actions[0]!.action, "drop");
});

test("reconcileTabs: attach wins over resume when ptyId still live (even if uuid also present)", () => {
  // ptyId takes priority over uuid fallback — live pty attach is preferred
  const ptys: PtyInfo[] = [pty({ id: "7", uuid: "uuid-b" })];
  const saved = [{ ptyId: "7", uuid: "uuid-b", label: "Tab" }];
  const actions = reconcileTabs(saved, ptys);
  assert.equal(actions[0]!.action, "attach");
});

test("reconcileTabs: handles multiple saved tabs with different outcomes", () => {
  const ptys: PtyInfo[] = [pty({ id: "1", uuid: "alive-uuid" })];
  const saved = [
    { ptyId: "1", uuid: "alive-uuid", label: "Alive" },
    { ptyId: "2", uuid: "dead-uuid", label: "Dead but restorable" },
    { ptyId: "3", label: "Completely gone" },
  ];
  const actions = reconcileTabs(saved, ptys);
  assert.equal(actions.length, 3);
  assert.equal(actions.find((a) => a.descriptor.label === "Alive")?.action, "attach");
  assert.equal(actions.find((a) => a.descriptor.label === "Dead but restorable")?.action, "resume");
  assert.equal(actions.find((a) => a.descriptor.label === "Completely gone")?.action, "drop");
});

test("reconcileTabs: empty saved returns empty actions", () => {
  const actions = reconcileTabs([], [pty({ id: "1" })]);
  assert.equal(actions.length, 0);
});

test("reconcileTabs: empty ptys, tab with uuid resumes; tab without uuid drops", () => {
  const saved = [
    { ptyId: "10", uuid: "some-uuid", label: "Restorable" },
    { ptyId: "11", label: "Not restorable" },
  ];
  const actions = reconcileTabs(saved, []);
  assert.equal(actions.find((a) => a.descriptor.label === "Restorable")?.action, "resume");
  assert.equal(actions.find((a) => a.descriptor.label === "Not restorable")?.action, "drop");
});

// ---------------------------------------------------------------------------
// reconnectQuery — pick the right /pty query to re-open a dropped socket.
//
// A live socket can drop for two reasons we can recover from:
//   - idle reaping (WSL2 localhost, NAT): the pty is still alive in the daemon
//     → re-attach by its (possibly renumbered) id.
//   - daemon restart (cargo watch): the pty is gone but the session is on disk
//     → resume by uuid (spawns `claude --resume`).
// Anything else is unrecoverable → null (give up, mark the tab dead).
//
// This is the single-tab analogue of reconcileTabs, returning the query string
// the reconnect loop hands to connectPty (or null to stop).
// ---------------------------------------------------------------------------

test("reconnectQuery: re-attaches by id when the pty is still live", () => {
  const ptys: PtyInfo[] = [pty({ id: "10", uuid: "uuid-a" })];
  const q = reconnectQuery({ ptyId: "10", uuid: "uuid-a", label: "Tab" }, ptys);
  assert.equal(q, "?attach=10");
});

test("reconnectQuery: re-attaches by renumbered id when uuid still matches a live pty", () => {
  // Daemon restarted and the pty kept running under a NEW id; match on uuid and
  // attach to the live id rather than resuming a duplicate.
  const ptys: PtyInfo[] = [pty({ id: "42", uuid: "uuid-a" })];
  const q = reconnectQuery({ ptyId: "10", uuid: "uuid-a", label: "Tab" }, ptys);
  assert.equal(q, "?attach=42");
});

test("reconnectQuery: resumes by uuid when the pty is gone but the session is on disk", () => {
  const ptys: PtyInfo[] = []; // daemon restarted, pty gone
  const q = reconnectQuery({ ptyId: "10", uuid: "uuid-a", label: "Tab" }, ptys);
  assert.equal(q, "?session=uuid-a");
});

test("reconnectQuery: encodes the uuid in the resume query", () => {
  const q = reconnectQuery({ uuid: "a/b c", label: "Tab" }, []);
  assert.equal(q, `?session=${encodeURIComponent("a/b c")}`);
});

test("reconnectQuery: returns null when the pty is gone and there is no uuid to resume", () => {
  const q = reconnectQuery({ ptyId: "10", label: "Ephemeral" }, []);
  assert.equal(q, null);
});

// ---------------------------------------------------------------------------
// reconnectDelay — capped exponential backoff for the reconnect loop.
// Attempt 0 is the first retry. Delay doubles each attempt, capped so a
// long-down daemon (a slow `cargo watch` rebuild) is still polled steadily.
// ---------------------------------------------------------------------------

test("reconnectDelay: first attempt waits the base delay", () => {
  assert.equal(reconnectDelay(0), RECONNECT_BASE_MS);
});

test("reconnectDelay: doubles each attempt", () => {
  assert.equal(reconnectDelay(1), RECONNECT_BASE_MS * 2);
  assert.equal(reconnectDelay(2), RECONNECT_BASE_MS * 4);
});

test("reconnectDelay: never exceeds the cap", () => {
  assert.equal(reconnectDelay(100), RECONNECT_CAP_MS);
});

test("reconnectDelay: treats negative attempts as the base delay (defensive)", () => {
  assert.equal(reconnectDelay(-1), RECONNECT_BASE_MS);
});

test("reconcileTabs: tab with uuid only (no ptyId) and uuid not in ptys → resume", () => {
  // A tab opened via ?session= never got a ptyId (or ptyId was cleared)
  const saved = [{ uuid: "orphan-uuid", label: "Session-only tab" }];
  const ptys: PtyInfo[] = [];
  const actions = reconcileTabs(saved, ptys);
  assert.equal(actions[0]!.action, "resume");
});

test("reconcileTabs: tab with uuid only and matching pty exists → attach using that ptyId", () => {
  // A tab saved with only uuid: if a live pty claims that uuid, prefer attach
  const ptys: PtyInfo[] = [pty({ id: "99", uuid: "matching-uuid" })];
  const saved = [{ uuid: "matching-uuid", label: "Session tab" }];
  const actions = reconcileTabs(saved, ptys);
  assert.equal(actions[0]!.action, "attach");
  assert.equal(actions[0]!.descriptor.ptyId, "99");
});
