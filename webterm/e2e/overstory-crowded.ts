// Browser test for a crowded overstory: with many sessions open, every tab must
// still show some of its name, not just its status dot and cache clock. Same
// stubbed-daemon harness as terminal-fit.ts.
//
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
const COUNT = 30;
/** Enough for roughly eight characters of a label at the tab's font size. */
const MIN_LABEL_PX = 48;

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const now = Date.now();
const PTYS = Array.from({ length: COUNT }, (_, n) => ({
  id: `pty-${n}`,
  cwd: `/srv/repo-${n}`,
  uuid: uuid(n),
  state: "idle",
  spawnedAt: new Date(0).toISOString(),
  lastActivity: new Date(0).toISOString(),
}));
const SAVED_TABS = PTYS.map((p, n) => ({
  ptyId: p.id,
  uuid: p.uuid,
  label: `session number ${n}`,
  cwd: p.cwd,
}));
// Every session has a cache clock, the widest case ("55m", "cold").
const FOREST = PTYS.map((p, n) => ({
  uuid: p.uuid,
  title: `session number ${n}`,
  cwd: p.cwd,
  recency: new Date(now).toISOString(),
  live: true,
  state: "ready",
  spark: [],
  cacheAt: new Date(now - 55 * 60_000).toISOString(),
  cacheTtl: 3600,
  peer: `repo-${n}-ab`,
}));

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "overstory-crowded-"));
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
    "/api/pty": PTYS,
    "/api/forest": FOREST,
    "/api/plan-reviews": [],
  };

  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await context.route("**/*", (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== ORIGIN) return route.abort();
    const file = files[url.pathname];
    if (file) return route.fulfill({ body: file.body, contentType: file.type });
    if (url.pathname in json) return route.fulfill({ json: json[url.pathname] });
    return route.fulfill({ status: 404, body: "" });
  });
  await context.addInitScript((tabs) => {
    localStorage.setItem("eigenform:term:tabs:v1", JSON.stringify(tabs));
  }, SAVED_TABS);

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForFunction((n) => document.querySelectorAll(".tab").length === n, COUNT);
  await page.waitForFunction(() => document.querySelectorAll(".tab-cache").length > 0);
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test("every tab in a crowded overstory still shows part of its name", async () => {
  const widths = await page.$$eval(".tab .tab-label", (labels) =>
    labels.map((l) => (l as HTMLElement).getBoundingClientRect().width),
  );
  assert.equal(widths.length, COUNT);
  const narrow = widths.filter((w) => w < MIN_LABEL_PX);
  assert.deepEqual(narrow, [], `label widths: ${widths.map((w) => Math.round(w)).join(", ")}`);
});

test("each tab names its session and its peer name on hover", async () => {
  const titles = await page.$$eval(".tab", (tabs) => tabs.map((t) => (t as HTMLElement).title));
  assert.ok(
    titles.every((t, n) => t.includes(`session number ${n}`) && t.includes(`repo-${n}-ab`)),
    titles.join(" | "),
  );
});

test("a live session's tab shows the name its peers message it by", async () => {
  const subs = await page.$$eval(".tab .tab-sub", (s) => s.map((e) => e.textContent));
  assert.deepEqual(subs, Array.from({ length: COUNT }, (_, n) => `repo-${n}-ab`));
});

test("the active tab is scrolled into view", async () => {
  await page.keyboard.press("Control+Shift+Tab");
  const visible = await page.$eval(".tab--active", (t) => {
    const strip = t.parentElement!.getBoundingClientRect();
    const r = t.getBoundingClientRect();
    return r.left >= strip.left - 1 && r.right <= strip.right + 1;
  });
  assert.ok(visible, "the active tab sits inside the overstory's visible area");
});
