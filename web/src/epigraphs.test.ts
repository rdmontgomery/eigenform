// Tests for the epigraph picker — pure selection logic, run: `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EPIGRAPHS, pickEpigraph } from "./epigraphs.ts";

test("every epigraph carries both a line and an attribution", () => {
  assert.ok(EPIGRAPHS.length >= 5);
  for (const e of EPIGRAPHS) {
    assert.ok(e.text.trim().length > 0, "text non-empty");
    assert.ok(e.attribution.trim().length > 0, "attribution non-empty");
  }
});

test("pickEpigraph indexes by the injected rng", () => {
  assert.deepEqual(pickEpigraph(() => 0), EPIGRAPHS[0]);
  assert.deepEqual(pickEpigraph(() => 0.999), EPIGRAPHS[EPIGRAPHS.length - 1]);
  // mid-range rounds down into a valid slot, never out of bounds
  assert.deepEqual(pickEpigraph(() => 0.5), EPIGRAPHS[Math.floor(0.5 * EPIGRAPHS.length)]);
});

test("pickEpigraph always returns a member of the list", () => {
  for (let i = 0; i < 50; i++) {
    const e = pickEpigraph();
    assert.ok(EPIGRAPHS.includes(e));
  }
});
