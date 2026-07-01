// Tests for reachviews.ts — the squarified treemap layout. Run: `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { layoutTreemap } from "./reachviews.ts";

test("layoutTreemap: empty input → empty output", () => {
  assert.deepEqual(layoutTreemap([]), []);
});

test("layoutTreemap: a single weight fills the unit square", () => {
  const [r] = layoutTreemap([5]);
  assert.equal(r.x, 0);
  assert.equal(r.y, 0);
  assert.ok(Math.abs(r.w - 1) < 1e-9);
  assert.ok(Math.abs(r.h - 1) < 1e-9);
});

test("layoutTreemap: cell area is proportional to weight", () => {
  const weights = [4, 3, 2, 1];
  const total = weights.reduce((a, b) => a + b, 0);
  const rects = layoutTreemap(weights);
  rects.forEach((r, i) => {
    const area = r.w * r.h;
    assert.ok(
      Math.abs(area - weights[i] / total) < 1e-6,
      `cell ${i} area ${area} ≠ ${weights[i] / total}`,
    );
  });
});

test("layoutTreemap: all cells stay within the unit square", () => {
  const rects = layoutTreemap([7, 1, 5, 3, 2, 9, 4]);
  for (const r of rects) {
    assert.ok(r.x >= -1e-9 && r.y >= -1e-9, "origin in-bounds");
    assert.ok(r.x + r.w <= 1 + 1e-9, "right edge in-bounds");
    assert.ok(r.y + r.h <= 1 + 1e-9, "bottom edge in-bounds");
    assert.ok(r.w > 0 && r.h > 0, "positive area");
  }
});

test("layoutTreemap: total covered area equals the unit square", () => {
  const rects = layoutTreemap([3, 3, 3, 3, 3]);
  const covered = rects.reduce((a, r) => a + r.w * r.h, 0);
  assert.ok(Math.abs(covered - 1) < 1e-6, `covered ${covered} ≠ 1`);
});
