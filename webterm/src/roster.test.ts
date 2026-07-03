// Tests for the roster data layer — pure merge + label derivation.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  buildRoster,
  deriveLabel,
} from "./roster.ts";
import type { PtyInfo, ForestItem } from "./types.ts";

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

function pty(overrides: Partial<PtyInfo> & { id: string }): PtyInfo {
  return {
    cwd: "/home/user/proj",
    uuid: null,
    state: "idle",
    spawnedAt: "2026-06-11T10:00:00Z",
    lastActivity: "2026-06-11T10:00:00Z",
    ...overrides,
  };
}

function forest(overrides: Partial<ForestItem> & { uuid: string }): ForestItem {
  return {
    title: null,
    cwd: "/home/user/proj",
    recency: "2026-06-11T09:00:00Z",
    live: false,
    state: "idle",
    spark: [],
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// deriveLabel
// ---------------------------------------------------------------------------

test("label degrades: aiTitle -> cwd basename -> first-prompt snippet -> 'new session'", () => {
  assert.equal(
    deriveLabel({ aiTitle: "Fix the parser", cwd: "/h/p/eigen" }),
    "Fix the parser",
  );
  assert.equal(deriveLabel({ cwd: "/h/p/eigen" }), "eigen");
  assert.equal(
    deriveLabel({ firstPrompt: "please look at..." }),
    "please look at...",
  );
  assert.equal(deriveLabel({}), "new session");
});

test("label: null cwd handled gracefully", () => {
  assert.equal(deriveLabel({ cwd: null }), "new session");
});

test("label: empty cwd basename falls through to next tier", () => {
  // cwd="/" has no basename — falls through to firstPrompt then fallback
  assert.equal(deriveLabel({ cwd: "/" }), "new session");
  assert.equal(deriveLabel({ cwd: "/", firstPrompt: "hi" }), "hi");
});

test("label: firstPrompt is trimmed and capped at 40 chars with ellipsis", () => {
  const long = "a".repeat(50);
  const result = deriveLabel({ firstPrompt: long });
  assert.equal(result.length, 43); // 40 chars + "..."
  assert.ok(result.endsWith("..."));

  const short = "short prompt";
  assert.equal(deriveLabel({ firstPrompt: short }), short);

  const exactly40 = "b".repeat(40);
  assert.equal(deriveLabel({ firstPrompt: exactly40 }), exactly40); // no ellipsis
});

test("label: firstPrompt trims leading/trailing whitespace before capping", () => {
  assert.equal(deriveLabel({ firstPrompt: "  trimmed  " }), "trimmed");
});

test("user override beats every derived label", () => {
  // override provided via buildRoster; test through deriveLabel's override param
  assert.equal(
    deriveLabel({ aiTitle: "Fix the parser", cwd: "/h/p/eigen", override: "My session" }),
    "My session",
  );
  assert.equal(deriveLabel({ override: "Custom name" }), "Custom name");
});

// ---------------------------------------------------------------------------
// buildRoster — msgCount (approximate turn count from the spark)
// ---------------------------------------------------------------------------

test("forest-only row carries msgCount from spark length", () => {
  const rows = buildRoster([], [forest({ uuid: "aaa", spark: [1, 2, 3] })], {});
  assert.equal(rows[0]!.msgCount, 3);
});

test("live row merged with a forest item inherits its spark count", () => {
  const ptys: PtyInfo[] = [pty({ id: "1", uuid: "aaa" })];
  const forestItems: ForestItem[] = [forest({ uuid: "aaa", spark: [9, 9] })];
  const rows = buildRoster(ptys, forestItems, {});
  assert.equal(rows[0]!.live, true);
  assert.equal(rows[0]!.msgCount, 2);
});

test("live-only row with no forest match has no msgCount", () => {
  const rows = buildRoster([pty({ id: "1", uuid: null })], [], {});
  assert.equal(rows[0]!.msgCount, undefined);
});

// ---------------------------------------------------------------------------
// buildRoster — downgrade (Fable→Opus guardrail marker)
// ---------------------------------------------------------------------------

test("forest row with a downgrade threads the offendingTurn onto the roster row", () => {
  const rows = buildRoster(
    [],
    [forest({ uuid: "aaa", downgrade: { offendingTurn: "u2" } })],
    {},
  );
  assert.equal(rows[0]!.downgrade?.offendingTurn, "u2");
});

test("forest row without a downgrade yields a nullish downgrade", () => {
  const rows = buildRoster([], [forest({ uuid: "aaa" })], {});
  assert.ok(!rows[0]!.downgrade);
});

// ---------------------------------------------------------------------------
// buildRoster — ordering
// ---------------------------------------------------------------------------

test("live ptys sort above recent forest rows", () => {
  const ptys: PtyInfo[] = [
    pty({ id: "1", uuid: null, lastActivity: "2026-06-11T08:00:00Z" }),
  ];
  const forestItems: ForestItem[] = [
    forest({ uuid: "aaa", recency: "2026-06-11T10:00:00Z" }), // newer than pty
  ];
  const rows = buildRoster(ptys, forestItems, {});
  assert.equal(rows[0]!.live, true, "live pty row should come first");
  assert.equal(rows[1]!.live, false, "forest-only row should come second");
});

test("within live group, rows sort by lastActivity most-recent-first", () => {
  const ptys: PtyInfo[] = [
    pty({ id: "1", uuid: null, lastActivity: "2026-06-11T08:00:00Z" }),
    pty({ id: "2", uuid: null, lastActivity: "2026-06-11T10:00:00Z" }),
    pty({ id: "3", uuid: null, lastActivity: "2026-06-11T09:00:00Z" }),
  ];
  const rows = buildRoster(ptys, [], {});
  assert.equal(rows[0]!.ptyId, "2");
  assert.equal(rows[1]!.ptyId, "3");
  assert.equal(rows[2]!.ptyId, "1");
});

test("within forest group, rows sort by recency most-recent-first", () => {
  const forestItems: ForestItem[] = [
    forest({ uuid: "aaa", recency: "2026-06-11T08:00:00Z" }),
    forest({ uuid: "bbb", recency: "2026-06-11T10:00:00Z" }),
    forest({ uuid: "ccc", recency: "2026-06-11T09:00:00Z" }),
  ];
  const rows = buildRoster([], forestItems, {});
  assert.equal(rows[0]!.uuid, "bbb");
  assert.equal(rows[1]!.uuid, "ccc");
  assert.equal(rows[2]!.uuid, "aaa");
});

// ---------------------------------------------------------------------------
// buildRoster — merge by uuid
// ---------------------------------------------------------------------------

test("a forest row with the same uuid as a pty merges into one row (live wins)", () => {
  const ptys: PtyInfo[] = [
    pty({ id: "42", uuid: "shared-uuid", state: "working" }),
  ];
  const forestItems: ForestItem[] = [
    forest({ uuid: "shared-uuid", title: "AI Title From JSONL", state: "idle" }),
  ];
  const rows = buildRoster(ptys, forestItems, {});
  assert.equal(rows.length, 1, "should produce one merged row");
  assert.equal(rows[0]!.ptyId, "42", "ptyId comes from registry");
  assert.equal(rows[0]!.uuid, "shared-uuid");
  assert.equal(rows[0]!.live, true);
  assert.equal(rows[0]!.state, "working", "live state wins for active pty");
  assert.equal(rows[0]!.label, "AI Title From JSONL", "aiTitle from forest side");
});

test("pty rows without uuid do not merge with any forest row", () => {
  const ptys: PtyInfo[] = [pty({ id: "1", uuid: null })];
  const forestItems: ForestItem[] = [
    forest({ uuid: "some-uuid" }),
  ];
  const rows = buildRoster(ptys, forestItems, {});
  assert.equal(rows.length, 2, "no merge when pty has no uuid");
});

test("forest row with live=true and no registry pty stays in forest group (not merged)", () => {
  // A forest row can claim live=true (claude running outside our registry).
  // We cannot attach to it so it stays in the disk/forest group.
  const forestItems: ForestItem[] = [
    forest({ uuid: "live-outside", live: true, state: "working" }),
    forest({ uuid: "normal", live: false }),
  ];
  const rows = buildRoster([], forestItems, {});
  // Both should be in forest group (no registry ptys)
  assert.equal(rows.every((r) => r.live === false), true,
    "forest-only rows are never promoted to live group even if forest.live=true");
  // But their state is preserved
  const liveRow = rows.find((r) => r.uuid === "live-outside");
  assert.ok(liveRow !== undefined);
  assert.equal(liveRow.state, "working");
});

// ---------------------------------------------------------------------------
// buildRoster — duplicate uuid (two ptys claiming same uuid)
// ---------------------------------------------------------------------------

test("two ptys claiming the same uuid: most-recently-active wins, other becomes unattached live row", () => {
  // Fork weirdness / --resume twice. Deterministic rule: the pty with the
  // higher lastActivity ISO string wins the merge slot; the other remains as
  // an independent live row (uuid cleared so it doesn't re-merge).
  const ptys: PtyInfo[] = [
    pty({ id: "10", uuid: "dup-uuid", lastActivity: "2026-06-11T09:00:00Z" }),
    pty({ id: "20", uuid: "dup-uuid", lastActivity: "2026-06-11T10:00:00Z" }), // newer wins
  ];
  const forestItems: ForestItem[] = [
    forest({ uuid: "dup-uuid", title: "From Forest" }),
  ];
  const rows = buildRoster(ptys, forestItems, {});
  // Should be 2 rows: winner (merged with forest) + loser (unattached live)
  assert.equal(rows.length, 2);
  const winner = rows.find((r) => r.uuid === "dup-uuid");
  assert.ok(winner !== undefined, "merged row retains the uuid");
  assert.equal(winner.ptyId, "20", "newer pty wins the merge");
  assert.equal(winner.label, "From Forest");
  const loser = rows.find((r) => r.ptyId === "10");
  assert.ok(loser !== undefined);
  assert.equal(loser.uuid, undefined, "loser's uuid cleared to avoid re-merge");
  assert.equal(loser.live, true, "loser is still a live row");
});

// ---------------------------------------------------------------------------
// buildRoster — overrides
// ---------------------------------------------------------------------------

test("override keyed by uuid takes precedence over derived label", () => {
  const forestItems: ForestItem[] = [
    forest({ uuid: "my-uuid", title: "AI Title" }),
  ];
  const rows = buildRoster([], forestItems, { "my-uuid": "My Custom Name" });
  assert.equal(rows[0]!.label, "My Custom Name");
});

test("override keyed by ptyId is used when uuid is unavailable", () => {
  // v1 limitation: pty rows without uuid can't be renamed durably via uuid key.
  // Shell may key overrides by ptyId as a fallback — we support both, uuid first.
  const ptys: PtyInfo[] = [pty({ id: "99", uuid: null })];
  const rows = buildRoster(ptys, [], { "99": "Pty Override" });
  assert.equal(rows[0]!.label, "Pty Override");
});

test("uuid override beats ptyId override when both present", () => {
  const ptys: PtyInfo[] = [pty({ id: "99", uuid: "my-uuid" })];
  const rows = buildRoster(ptys, [], { "my-uuid": "By UUID", "99": "By PtyId" });
  assert.equal(rows[0]!.label, "By UUID");
});

// ---------------------------------------------------------------------------
// buildRoster — cwdChip
// ---------------------------------------------------------------------------

test("cwdChip is basename of cwd, empty string for null cwd", () => {
  const ptys: PtyInfo[] = [
    pty({ id: "1", cwd: "/home/user/myproject", uuid: null }),
    pty({ id: "2", cwd: null, uuid: null }),
  ];
  const rows = buildRoster(ptys, [], {});
  const r1 = rows.find((r) => r.ptyId === "1");
  const r2 = rows.find((r) => r.ptyId === "2");
  assert.equal(r1!.cwdChip, "myproject");
  assert.equal(r2!.cwdChip, "");
});

// ---------------------------------------------------------------------------
// buildRoster — key uniqueness
// ---------------------------------------------------------------------------

test("every row has a unique key", () => {
  const ptys: PtyInfo[] = [
    pty({ id: "1", uuid: "uuid-a" }),
    pty({ id: "2", uuid: null }),
  ];
  const forestItems: ForestItem[] = [
    forest({ uuid: "uuid-b" }),
    forest({ uuid: "uuid-a" }), // merges with pty 1
  ];
  const rows = buildRoster(ptys, forestItems, {});
  const keys = rows.map((r) => r.key);
  const unique = new Set(keys);
  assert.equal(unique.size, keys.length, "all keys must be unique");
});
