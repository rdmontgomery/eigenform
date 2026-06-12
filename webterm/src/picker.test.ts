/**
 * picker.test.ts — Tests for the pure resolvePick decision function.
 *
 * Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
 *
 * ## resolvePick contract:
 *   - highlighted candidate chosen → path from candidate, create=false
 *   - typed absolute path, matches a candidate → create=false
 *   - typed absolute path, not in candidates → create=true
 *   - typed bare name (no "/"), workspace_root proxy known → resolve to root/name, create=true
 *   - typed bare name, no workspace_root proxy → return null (can't resolve)
 *   - typed empty, no highlight → return null (no-op)
 *
 * ## workspace_root proxy:
 *   The frontend doesn't know workspace_root directly. We derive it from the
 *   first non-recent candidate: these are immediate subdirs of the workspace root,
 *   so their parent IS the workspace root. If no non-recent candidates exist,
 *   bare-name resolution is unavailable.
 *
 * ## Limitation (backlog):
 *   This heuristic fails when workspace_root has no non-recent candidates (e.g.
 *   all known dirs are recents). In that case bare-name input requires the user
 *   to type an absolute path. A future improvement: expose workspace_root from
 *   GET /api/candidates response ({root, candidates} wrapper).
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { resolvePick } from "./picker.ts";
import type { Candidate } from "./types.ts";

function c(path: string, recent = false): Candidate {
  return { path, recent };
}

// ---------------------------------------------------------------------------
// highlighted candidate wins over typed text
// ---------------------------------------------------------------------------

test("highlighted candidate → use its path, create=false", () => {
  const result = resolvePick(
    "eigen",                           // typed
    c("/home/user/projects/eigen"),    // highlighted
    [c("/home/user/projects/eigen"), c("/home/user/projects/foo", false)],
  );
  assert.deepEqual(result, { path: "/home/user/projects/eigen", create: false });
});

test("highlighted recent candidate → use its path, create=false", () => {
  const result = resolvePick(
    "",
    c("/home/user/projects/eigen", true),
    [c("/home/user/projects/eigen", true)],
  );
  assert.deepEqual(result, { path: "/home/user/projects/eigen", create: false });
});

// ---------------------------------------------------------------------------
// typed absolute path
// ---------------------------------------------------------------------------

test("typed absolute path matching a candidate → create=false", () => {
  const candidates = [c("/root/alpha"), c("/root/beta")];
  const result = resolvePick("/root/alpha", null, candidates);
  assert.deepEqual(result, { path: "/root/alpha", create: false });
});

test("typed absolute path not in candidates → create=true", () => {
  const candidates = [c("/root/alpha"), c("/root/beta")];
  const result = resolvePick("/root/gamma", null, candidates);
  assert.deepEqual(result, { path: "/root/gamma", create: true });
});

test("typed absolute path, empty candidates → create=true", () => {
  const result = resolvePick("/root/newproject", null, []);
  assert.deepEqual(result, { path: "/root/newproject", create: true });
});

// ---------------------------------------------------------------------------
// typed bare name (no "/") — resolved via workspace_root proxy
// ---------------------------------------------------------------------------

test("typed bare name, non-recent candidates present → resolved against root, create=true", () => {
  // /workspace/alpha and /workspace/beta are non-recent subdirs → root = /workspace
  const candidates = [c("/workspace/alpha", false), c("/workspace/beta", false)];
  const result = resolvePick("newdir", null, candidates);
  assert.deepEqual(result, { path: "/workspace/newdir", create: true });
});

test("typed bare name, first non-recent candidate used as root proxy", () => {
  // recent candidates are first; the first non-recent candidate is /root/subdir-a
  const candidates = [
    c("/recent/proj", true),
    c("/root/subdir-a", false),
    c("/root/subdir-b", false),
  ];
  const result = resolvePick("mynewdir", null, candidates);
  // root proxy = parent of /root/subdir-a = /root
  assert.deepEqual(result, { path: "/root/mynewdir", create: true });
});

test("typed bare name, no non-recent candidates → null (can't resolve)", () => {
  // All candidates are recent → no non-recent to derive root from
  const candidates = [c("/recent/a", true), c("/recent/b", true)];
  const result = resolvePick("newdir", null, candidates);
  assert.equal(result, null);
});

test("typed bare name, empty candidates → null (can't resolve)", () => {
  const result = resolvePick("newdir", null, []);
  assert.equal(result, null);
});

// ---------------------------------------------------------------------------
// empty input
// ---------------------------------------------------------------------------

test("empty typed, no highlight → null (no-op)", () => {
  const result = resolvePick("", null, [c("/root/a")]);
  assert.equal(result, null);
});

test("empty typed, highlighted candidate → use highlighted", () => {
  const result = resolvePick("", c("/root/a"), [c("/root/a")]);
  assert.deepEqual(result, { path: "/root/a", create: false });
});

// ---------------------------------------------------------------------------
// highlight overrides typed text regardless
// ---------------------------------------------------------------------------

test("highlighted takes priority over typed absolute path", () => {
  const result = resolvePick(
    "/root/typed-path",
    c("/root/highlighted"),
    [c("/root/highlighted"), c("/root/typed-path")],
  );
  assert.deepEqual(result, { path: "/root/highlighted", create: false });
});

// ---------------------------------------------------------------------------
// whitespace-only input is treated as empty
// ---------------------------------------------------------------------------

test("whitespace-only typed with no highlight → null", () => {
  const result = resolvePick("   ", null, [c("/root/a")]);
  assert.equal(result, null);
});
