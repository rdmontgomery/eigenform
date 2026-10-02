// Tab snooze, end to end in the real app (src/main.ts → mountShell) against a
// stubbed daemon that keeps snoozes in memory the way /api/snoozes does:
// snoozing closes the tab and stores it; once due, the poll claims it and the
// tab comes back in the background, flashing, with ⏰ in the page title until
// it is clicked; cancelling from the shelf drops it for good.
//
// Run: `npm run test:browser`. Needs Chromium for playwright-core; set
// CHROMIUM_PATH to use a preinstalled binary.
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

const pty = (id: string) => ({
  id,
  cwd: "/tmp",
  uuid: null,
  state: "idle",
  spawnedAt: new Date(0).toISOString(),
  lastActivity: new Date(0).toISOString(),
});
const SAVED_TABS = [
  { ptyId: "pty-a", label: "alpha", cwd: "/tmp", kind: "terminal" },
  { ptyId: "pty-b", label: "bravo", cwd: "/tmp", kind: "terminal" },
];

interface StoredSnooze { id: string; until: number; snoozedAt: number; tab: { label: string } }
const store: StoredSnooze[] = [];
let seq = 0;

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "snooze-"));
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
    "/api/pty": [pty("pty-a"), pty("pty-b")],
    "/api/forest": [],
    "/api/plan-reviews": [],
  };

  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await context.route("**/*", (route) => {
    const req = route.request();
    const url = new URL(req.url());
    if (url.origin !== ORIGIN) return route.abort();
    const file = files[url.pathname];
    if (file) return route.fulfill({ body: file.body, contentType: file.type });
    if (url.pathname === "/api/snoozes" && req.method() === "POST") {
      const body = req.postDataJSON() as { until: number; tab: { label: string } };
      const s = { id: `s${++seq}`, until: body.until, snoozedAt: Date.now(), tab: body.tab };
      store.push(s);
      return route.fulfill({ status: 201, json: s });
    }
    if (url.pathname === "/api/snoozes") {
      return route.fulfill({ json: [...store].sort((a, b) => a.until - b.until) });
    }
    const del = url.pathname.match(/^\/api\/snoozes\/(.+)$/);
    if (del && req.method() === "DELETE") {
      const i = store.findIndex((s) => s.id === decodeURIComponent(del[1]!));
      if (i < 0) return route.fulfill({ status: 404, body: "" });
      const [gone] = store.splice(i, 1);
      return route.fulfill({ json: gone });
    }
    if (url.pathname in json) return route.fulfill({ json: json[url.pathname] });
    return route.fulfill({ status: 404, body: "" });
  });
  await context.addInitScript((tabs) => {
    if (!sessionStorage.getItem("seeded")) {
      localStorage.setItem("eigenform:term:tabs:v1", JSON.stringify(tabs));
      sessionStorage.setItem("seeded", "1");
    }
  }, SAVED_TABS);

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForFunction(() => document.querySelectorAll(".tab").length === 2);
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const labels = () =>
  page.$$eval(".tab .tab-label", (els) => els.map((e) => e.textContent));

/** Pretend the page was just shown again — runs the same refresh the poll does. */
const poke = () =>
  page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));

test("snoozing a tab closes it and stores its wake time", async () => {
  const tabA = page.locator(".tab", { hasText: "alpha" });
  await tabA.hover();
  await tabA.locator(".tab-snooze").click();
  await page.waitForSelector(".snooze-menu");
  const opts = await page.$$eval(".snooze-opt > span:first-child", (els) => els.map((e) => e.textContent));
  assert.ok(opts.includes("30 minutes") && opts.includes("3 days"), `presets: ${opts}`);

  const t0 = Date.now();
  await page.click(".snooze-opt:has-text('30 minutes')");
  await page.waitForFunction(() => document.querySelectorAll(".tab").length === 1);
  assert.deepEqual(await labels(), ["bravo"]);
  assert.equal(store.length, 1);
  const wait = store[0]!.until - t0;
  assert.ok(wait > 29 * 60_000 && wait < 31 * 60_000, `wakes in ${wait}ms`);
  assert.equal(store[0]!.tab.label, "alpha");
  assert.equal(await page.textContent(".snooze-count"), "1");
});

test("a due snooze reopens in the background, flashing, until clicked", async () => {
  const title = await page.title();
  store[0]!.until = Date.now() - 1000;
  await poke();
  await page.waitForSelector(".tab--woke");
  assert.deepEqual(await labels(), ["bravo", "alpha"]);
  assert.equal(store.length, 0, "the wake claimed (deleted) the snooze");
  // Did not steal focus from the tab being worked in.
  assert.equal(await page.textContent(".tab--active .tab-label"), "bravo");
  assert.match(await page.title(), /^⏰ 1 woke/);
  assert.equal(await page.$(".snooze-shelf-btn"), null, "shelf hides when empty");

  await page.click(".tab--woke");
  assert.equal(await page.$(".tab--woke"), null);
  assert.equal(await page.textContent(".tab--active .tab-label"), "alpha");
  assert.equal(await page.title(), title.replace(/^⏰ \d+ woke · /, ""));
});

test("a custom duration snoozes; cancelling from the shelf drops it", async () => {
  const tabB = page.locator(".tab", { hasText: "bravo" });
  await tabB.hover();
  await tabB.locator(".tab-snooze").click();
  await page.fill(".snooze-input", "nonsense");
  assert.equal(await page.isDisabled(".snooze-go"), true);
  await page.fill(".snooze-input", "3d");
  const t0 = Date.now();
  await page.press(".snooze-input", "Enter");
  await page.waitForFunction(() => document.querySelectorAll(".tab").length === 1);
  const wait = store[0]!.until - t0;
  assert.ok(Math.abs(wait - 3 * 86_400_000) < 60_000, `wakes in ${wait}ms`);

  await page.click(".snooze-shelf-btn");
  await page.waitForSelector(".snooze-shelf .snooze-row");
  assert.match((await page.textContent(".snooze-row-when"))!, /wakes in 2d 23h|wakes in 3d/);
  await page.click(".snooze-row-btn[title^='Cancel']");
  await page.waitForFunction(() => !document.querySelector(".snooze-shelf-btn"));
  assert.equal(store.length, 0);
  await poke();
  assert.deepEqual(await labels(), ["alpha"], "a cancelled snooze never comes back");
});
