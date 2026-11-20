// Clicking a session in the graph panel switches to its tab at once: the graph
// already holds the session's full uuid, so the click takes the rail row's path
// and never waits on `/api/resolve` (a forest scan, seconds on a busy machine).
// The node's status dot is the rail's, and the panel says what the dots mean.
//
// Drives the real app against a stubbed daemon whose `/api/resolve` never answers.
// Run: `npm run test:browser`. Set CHROMIUM_PATH to use a preinstalled binary.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { chromium, type Browser, type Page } from "playwright-core";

const ORIGIN = "http://eigenform.test";
const ROOT = fileURLToPath(new URL("..", import.meta.url));

const ALPHA = "0000aaaa-0000-4000-8000-000000000001";
const BETA = "0000bbbb-0000-4000-8000-000000000002";
const pty = (id: string, uuid: string, state: string) => ({
  id,
  cwd: "/srv/app",
  uuid,
  state,
  spawnedAt: new Date(0).toISOString(),
  lastActivity: new Date().toISOString(),
});
const row = (uuid: string, title: string, state: string) => ({
  uuid,
  title,
  peer: null,
  cwd: "/srv/app",
  recency: new Date().toISOString(),
  live: true,
  state,
  spark: [],
});
const GRAPH = {
  nodes: [
    { id: ALPHA, label: "alpha", cwd: "/srv/app", live: true, state: "working", ghost: false },
    { id: BETA, label: "beta", cwd: "/srv/app", live: true, state: "ready", ghost: false },
  ],
  lineage: [{ from: ALPHA, to: BETA }],
  messages: [],
};

let dir: string;
let browser: Browser;
let page: Page;
let resolves = 0;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "graph-click-"));
  await build({
    entryPoints: [join(ROOT, "src/main.ts")],
    bundle: true,
    format: "esm",
    outdir: dir,
    logLevel: "silent",
  });
  const files: Record<string, { body: Buffer; type: string }> = {
    "/": { body: readFileSync(join(ROOT, "index.html")), type: "text/html" },
    "/dist/main.js": { body: readFileSync(join(dir, "main.js")), type: "text/javascript" },
    "/dist/main.css": { body: readFileSync(join(dir, "main.css")), type: "text/css" },
  };
  const json: Record<string, unknown> = {
    "/api/pty": [pty("1", ALPHA, "working"), pty("2", BETA, "waiting")],
    "/api/forest": [row(ALPHA, "alpha", "working"), row(BETA, "beta", "ready")],
    "/api/graph": GRAPH,
    "/api/plan-reviews": [],
    "/api/pins": [],
  };

  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1400, height: 800 } });
  await context.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== ORIGIN) return route.abort();
    // A slow daemon: the lookup the click must not wait on never answers.
    if (url.pathname.startsWith("/api/resolve/")) {
      resolves++;
      return;
    }
    const file = files[url.pathname];
    if (file) return route.fulfill({ body: file.body, contentType: file.type });
    if (url.pathname in json) return route.fulfill({ json: json[url.pathname] });
    return route.fulfill({ status: 404, body: "" });
  });
  await context.routeWebSocket(/\/pty/, (ws) => {
    const id = new URL(ws.url()).searchParams.get("attach");
    if (id) ws.send(JSON.stringify({ type: "pty", id }));
  });
  await context.addInitScript(() => {
    localStorage.setItem("eigenform:term:drawer:v1", "1");
    localStorage.setItem(
      "eigenform:term:tabs:v1",
      JSON.stringify([
        { ptyId: "1", uuid: "0000aaaa-0000-4000-8000-000000000001", label: "alpha", cwd: "/srv/app", kind: "terminal" },
        { ptyId: "2", uuid: "0000bbbb-0000-4000-8000-000000000002", label: "beta", cwd: "/srv/app", kind: "terminal" },
      ]),
    );
  });

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector(`.sg-node[data-id="${BETA}"]`);
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const active = () => page.locator(".tab--active").textContent();

test("clicking a graph node switches to its tab without a lookup", async () => {
  await page.locator(".tab", { hasText: "alpha" }).click();
  assert.match((await active()) ?? "", /alpha/);
  await page.locator(`.sg-node[data-id="${BETA}"]`).click();
  await page.waitForFunction(() => /beta/.test(document.querySelector(".tab--active")?.textContent ?? ""), null, {
    timeout: 500,
  });
  assert.equal(resolves, 0, "no /api/resolve round trip");
  assert.equal(page.url(), `${ORIGIN}/`, "no navigation");
});

test("a node's dot is the rail's dot, and the panel explains the dots", async () => {
  const cls = (id: string) => page.locator(`.sg-node[data-id="${id}"] .dot`).getAttribute("class");
  assert.equal(await cls(ALPHA), "dot dot--working dot--eigenform");
  assert.equal(await cls(BETA), "dot dot--waiting dot--eigenform");
  assert.match((await page.locator(`.sg-node[data-id="${BETA}"]`).getAttribute("title")) ?? "", /waiting for your input/);
  const legend = (await page.locator(".sgraph-legend").textContent()) ?? "";
  for (const word of ["running", "your turn", "idle", "outside eigenform", "exited"]) {
    assert.ok(legend.includes(word), `legend names "${word}": ${legend}`);
  }
});
