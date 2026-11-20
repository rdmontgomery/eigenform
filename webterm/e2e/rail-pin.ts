// Pinning a session from the forest preview moves its row into a "Pinned" group at
// the top of the rail, through the daemon's /api/pins (stubbed here in memory), and
// unpinning puts it back in its age group.
//
// Run: `pnpm test:browser`. Set CHROMIUM_PATH to use a preinstalled binary.
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

const OLD = "0000aaaa-0000-4000-8000-000000000001";
const NEW = "0000bbbb-0000-4000-8000-000000000002";
const session = (uuid: string, title: string, ageDays: number) => ({
  uuid,
  title,
  peer: null,
  cwd: "/srv/app",
  recency: new Date(Date.now() - ageDays * 86_400_000).toISOString(),
  live: false,
  state: "idle",
  spark: [],
});
const FOREST = [session(NEW, "fresh work", 0), session(OLD, "commander", 30)];

let dir: string;
let browser: Browser;
let page: Page;
let pins: { uuid: string; pinnedAt: number }[] = [];

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "rail-pin-"));
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
    "/api/pty": [],
    "/api/forest": FOREST,
    "/api/plan-reviews": [],
  };

  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await context.route("**/*", (route) => {
    const req = route.request();
    const url = new URL(req.url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname === "/api/pins") {
      if (req.method() === "POST") {
        const { session: uuid } = req.postDataJSON() as { session: string };
        if (!pins.some((p) => p.uuid === uuid)) pins.push({ uuid, pinnedAt: Date.now() });
      }
      return route.fulfill({ json: pins });
    }
    const unpin = /^\/api\/pins\/(.+)$/.exec(url.pathname);
    if (unpin && req.method() === "DELETE") {
      pins = pins.filter((p) => p.uuid !== unpin[1]);
      return route.fulfill({ json: pins });
    }
    const file = files[url.pathname];
    if (file) return route.fulfill({ body: file.body, contentType: file.type });
    if (url.pathname in json) return route.fulfill({ json: json[url.pathname] });
    return route.fulfill({ status: 404, body: "" });
  });

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector(".rail-row-label:text-is('commander')");
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const groups = () => page.locator(".rail-group-label").allTextContents();
const firstRow = () => page.locator(".rail-row-label").first().textContent();

test("Pin moves a session to the top of the rail; Unpin puts it back", async () => {
  assert.equal(await firstRow(), "fresh work");
  assert.ok(!(await groups()).includes("Pinned"));

  await page.click(".rail-row-label:text-is('commander')");
  await page.locator(".forest-preview-pin:text-is('Pin')").click();
  await page.waitForSelector(".forest-preview-pin:text-is('Unpin')");
  assert.deepEqual(pins.map((p) => p.uuid), [OLD]);
  assert.equal((await groups())[0], "Pinned");
  assert.equal(await firstRow(), "commander");
  if (process.env.SCREENSHOT) await page.screenshot({ path: process.env.SCREENSHOT });

  await page.locator(".forest-preview-pin:text-is('Unpin')").click();
  await page.waitForSelector(".forest-preview-pin:text-is('Pin')");
  assert.deepEqual(pins, []);
  assert.ok(!(await groups()).includes("Pinned"));
  assert.equal(await firstRow(), "fresh work");
});

test("each row's pin toggle pins and unpins without opening the preview", async () => {
  const row = (title: string) => page.locator(".rail-row", { has: page.locator(`.rail-row-label:text-is('${title}')`) });
  await page.keyboard.press("Escape");
  await row("commander").locator(".rail-row-pin").click();
  await page.waitForSelector(".rail-row-pin--on");
  assert.deepEqual(pins.map((p) => p.uuid), [OLD]);
  assert.equal((await groups())[0], "Pinned");
  assert.equal(await firstRow(), "commander");
  assert.equal(await page.locator(".rail-row-pin").count(), 2, "every row shows a toggle");

  await row("commander").locator(".rail-row-pin--on").click();
  await page.waitForFunction(() => !document.querySelector(".rail-row-pin--on"));
  assert.deepEqual(pins, []);
  assert.equal(await firstRow(), "fresh work");
});
