// Tests for artifacts.ts — the pane's pure selection logic.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { pickArtifact, artifactSrc, artifactLabel } from "./artifacts.ts";
import type { ArtifactRow } from "./artifacts.ts";

function row(path: string, o: Partial<ArtifactRow> = {}): ArtifactRow {
  return {
    path,
    name: path.split("/").pop() ?? null,
    kind: "html",
    turn: 1,
    by: "session",
    mtime: "2026-10-02T09:00:00+00:00",
    url: `/artifact/u${path}`,
    ...o,
  };
}

test("pickArtifact: follows the newest existing write", () => {
  const rows = [row("/p/gone.html", { mtime: null }), row("/p/b.html"), row("/p/a.html")];
  assert.deepEqual(pickArtifact(rows, { path: null, pinned: false }), { path: "/p/b.html", pinned: false });
});

test("pickArtifact: a pin holds while its artifact is listed", () => {
  const rows = [row("/p/b.html"), row("/p/a.html")];
  const pinned = { path: "/p/a.html", pinned: true };
  assert.deepEqual(pickArtifact(rows, pinned), pinned);
});

test("pickArtifact: a pin whose artifact vanished falls back to following", () => {
  const rows = [row("/p/b.html")];
  assert.deepEqual(pickArtifact(rows, { path: "/p/a.html", pinned: true }), { path: "/p/b.html", pinned: false });
});

test("pickArtifact: no rows → nothing selected", () => {
  assert.deepEqual(pickArtifact([], { path: "/p/a.html", pinned: true }), { path: null, pinned: false });
});

test("artifactSrc: cache-busts on mtime, so an edit changes the src", () => {
  const a = artifactSrc(row("/p/a.html"));
  const b = artifactSrc(row("/p/a.html", { mtime: "2026-10-02T09:00:05+00:00" }));
  assert.notEqual(a, b);
  assert.ok(a.startsWith("/artifact/u/p/a.html?v="));
});

test("artifactLabel: names the nested writer when it wasn't the session", () => {
  assert.equal(artifactLabel(row("/w/chart.svg", { by: "codex" })), "chart.svg · codex");
  assert.equal(artifactLabel(row("/p/a.html")), "a.html");
});
