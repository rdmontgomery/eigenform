import { test } from "node:test";
import assert from "node:assert/strict";

import { prettyModel } from "./forest-preview.ts";

test("prettyModel: opus id → family + dotted version", () => {
  assert.equal(prettyModel("claude-opus-4-8"), "Opus 4.8");
});

test("prettyModel: sonnet/haiku/fable families", () => {
  assert.equal(prettyModel("claude-sonnet-4-6"), "Sonnet 4.6");
  assert.equal(prettyModel("claude-haiku-4-5-20251001"), "Haiku 4.5");
  assert.equal(prettyModel("claude-fable-5"), "Fable 5");
});

test("prettyModel: null/empty → empty string", () => {
  assert.equal(prettyModel(null), "");
  assert.equal(prettyModel(""), "");
});

test("prettyModel: unknown id strips the claude- prefix", () => {
  assert.equal(prettyModel("claude-experimental-x"), "experimental-x");
});
