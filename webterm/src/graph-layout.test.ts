import { test } from "node:test";
import assert from "node:assert/strict";
import {
  lineageOutline,
  ringLayout,
  arcPath,
  edgeWidth,
  edgeRecency,
  nodeStatus,
  type GraphNode,
  type SessionGraph,
} from "./graph-layout.ts";

const node = (id: string, live = true, label = id): GraphNode => ({ id, label, live, ghost: false });

test("lineage outline lists children under their parent, then the loners", () => {
  const g: SessionGraph = {
    nodes: [node("orch"), node("b-worker"), node("a-worker"), node("lone", false), node("grandkid", false), node("solo")],
    lineage: [
      { from: "orch", to: "b-worker" },
      { from: "orch", to: "a-worker" },
      { from: "a-worker", to: "grandkid" },
    ],
    messages: [],
  };
  const rows = lineageOutline(g);
  assert.deepEqual(
    rows.map((r) => [r.node.id, r.depth, r.parentRow, r.lonelyStart ?? false]),
    [
      ["orch", 0, null, false],
      ["a-worker", 1, 0, false],
      ["grandkid", 2, 1, false],
      ["b-worker", 1, 0, false],
      // Live loners before dead ones.
      ["solo", 0, null, true],
      ["lone", 0, null, false],
    ],
  );
});

test("lineage outline with no edges is just the loners, unmarked", () => {
  const rows = lineageOutline({ nodes: [node("x"), node("y")], lineage: [], messages: [] });
  assert.deepEqual(rows.map((r) => [r.node.id, r.lonelyStart ?? false]), [["x", false], ["y", false]]);
});

test("lineage outline survives a cycle and a second parent", () => {
  const g: SessionGraph = {
    nodes: [node("p"), node("q"), node("r")],
    lineage: [
      { from: "p", to: "q" },
      { from: "q", to: "p" },
      { from: "r", to: "q" },
    ],
    messages: [],
  };
  const rows = lineageOutline(g);
  assert.equal(rows.length, 3);
  assert.deepEqual(new Set(rows.map((r) => r.node.id)), new Set(["p", "q", "r"]));
});

test("ring layout starts at the top and stays inside the box", () => {
  const pos = ringLayout(["a", "b", "c", "d"], 400, 200, 40);
  const a = pos.get("a")!;
  assert.ok(Math.abs(a.x - 200) < 1e-9);
  assert.equal(a.y, 100 - 80);
  for (const p of pos.values()) {
    assert.ok(p.x >= 40 && p.x <= 360 && p.y >= 0 && p.y <= 200, JSON.stringify(p));
  }
  assert.deepEqual(ringLayout(["only"], 400, 200, 40).get("only"), { x: 200, y: 100 });
});

test("arcs between the same pair bend to opposite sides", () => {
  const a = { x: 0, y: 0 };
  const b = { x: 100, y: 0 };
  const there = arcPath(a, b, 20, 10);
  const back = arcPath(b, a, 20, 10);
  assert.equal(there, "M10,0 Q50,-20 90,0");
  assert.equal(back, "M90,0 Q50,20 10,0");
});

test("edge weight grows with count; recency fades with age", () => {
  assert.equal(edgeWidth(1), 1);
  assert.equal(edgeWidth(4), 3);
  assert.equal(edgeWidth(10_000), 5);
  const now = Date.parse("2026-01-01T12:00:00Z");
  assert.equal(edgeRecency("2026-01-01T11:50:00Z", now), 1);
  assert.equal(edgeRecency("2026-01-01T11:30:00Z", now), 0.75);
  assert.equal(edgeRecency("2026-01-01T01:00:00Z", now), 0.5);
  assert.equal(edgeRecency("2025-12-01T00:00:00Z", now), 0.3);
  assert.equal(edgeRecency("garbage", now), 0.3);
});

test("a node's dot follows the rail when the rail lists it, else its forest state", () => {
  const live = (state: string): GraphNode => ({ id: "a", label: "a", live: true, state, ghost: false });
  assert.deepEqual(nodeStatus(live("ready"), { activity: "working", liveness: "eigenform" }), {
    activity: "working",
    liveness: "eigenform",
  });
  assert.deepEqual(nodeStatus(live("working"), null), { activity: "working", liveness: "external" });
  assert.deepEqual(nodeStatus(live("ready"), null), { activity: "waiting", liveness: "external" });
  assert.deepEqual(
    nodeStatus(live("ready"), { activity: "idle", liveness: "none" }),
    { activity: "waiting", liveness: "external" },
    "a rail row that hasn't caught up yet doesn't make a live session read as exited",
  );
  assert.deepEqual(nodeStatus({ ...live("working"), live: false }, { activity: "working", liveness: "eigenform" }), {
    activity: "idle",
    liveness: "none",
  });
  assert.deepEqual(nodeStatus({ id: "peer:x", label: "x", live: false, ghost: true }, null), {
    activity: "idle",
    liveness: "none",
  });
});
