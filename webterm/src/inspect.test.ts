// Tests for inspect.ts — pure config-inventory shaping.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  fmtTokens,
  skillStatus,
  inspectSummary,
  type InspectData,
  type InspectSkill,
} from "./inspect.ts";

function skill(overrides: Partial<InspectSkill> & { name: string }): InspectSkill {
  return {
    description: "",
    path: `/skills/${overrides.name}`,
    size: 0,
    tokens: 0,
    wins: true,
    namespaced: false,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// fmtTokens — matches the Rust fmt_tokens
// ---------------------------------------------------------------------------

test("fmtTokens: under 1k is a plain count", () => {
  assert.equal(fmtTokens(0), "~0 tok");
  assert.equal(fmtTokens(999), "~999 tok");
});

test("fmtTokens: 1k and above is abbreviated to one decimal", () => {
  assert.equal(fmtTokens(1000), "~1.0k tok");
  assert.equal(fmtTokens(1234), "~1.2k tok");
  assert.equal(fmtTokens(20500), "~20.5k tok");
});

// ---------------------------------------------------------------------------
// skillStatus — namespaced > shadowed/wins
// ---------------------------------------------------------------------------

test("skillStatus: namespaced wins over the wins flag", () => {
  assert.equal(skillStatus(skill({ name: "a", namespaced: true, wins: false })), "namespaced");
  assert.equal(skillStatus(skill({ name: "a", namespaced: true, wins: true })), "namespaced");
});

test("skillStatus: bare-name reflects the wins flag", () => {
  assert.equal(skillStatus(skill({ name: "a", wins: true })), "wins");
  assert.equal(skillStatus(skill({ name: "a", wins: false })), "shadowed");
});

// ---------------------------------------------------------------------------
// inspectSummary — counts roll up across layers, shadowed counted
// ---------------------------------------------------------------------------

test("inspectSummary: rolls up layers, skills, memory, shadowed", () => {
  const data: InspectData = {
    tokens: 1500,
    layers: [
      {
        label: "global",
        tokens: 900,
        skills: [
          skill({ name: "review", wins: false }), // shadowed
          skill({ name: "brainstorm", wins: true }),
        ],
        memory: [],
      },
      {
        label: "repo",
        tokens: 600,
        skills: [skill({ name: "review", wins: true })],
        memory: [
          { name: "auth", description: "x", kind: "project", path: "/m/auth", size: 0, tokens: 0 },
        ],
      },
    ],
  };
  const s = inspectSummary(data);
  assert.equal(s.layers, 2);
  assert.equal(s.tokens, 1500);
  assert.equal(s.skills, 3);
  assert.equal(s.memory, 1);
  assert.equal(s.shadowed, 1);
});

test("inspectSummary: empty inventory is all zeros", () => {
  const s = inspectSummary({ tokens: 0, layers: [] });
  assert.deepEqual(s, { layers: 0, tokens: 0, skills: 0, memory: 0, shadowed: 0 });
});
