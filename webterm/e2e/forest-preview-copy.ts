// The forest preview's "Copy id" button copies the previewed session's uuid,
// says so, and is absent for a row with no uuid yet. The test origin is not a
// secure context, so this exercises the execCommand fallback a webview needs.
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

const UUID = "11111111-1111-4111-8111-111111111111";
const FOREST = [
  {
    uuid: UUID,
    title: "copy me",
    cwd: "/srv/eigenform",
    recency: new Date().toISOString(),
    live: false,
    state: "idle",
    spark: [],
  },
];

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "preview-copy-"));
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
    const url = new URL(route.request().url());
    if (url.origin !== ORIGIN) return route.abort();
    const file = files[url.pathname];
    if (file) return route.fulfill({ body: file.body, contentType: file.type });
    if (url.pathname in json) return route.fulfill({ json: json[url.pathname] });
    return route.fulfill({ status: 404, body: "" });
  });
  await context.addInitScript(() => {
    document.addEventListener("copy", () => {
      const active = document.activeElement;
      (window as unknown as { copied: string }).copied = active instanceof HTMLTextAreaElement ? active.value : "";
    });
  });

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector(".rail-row-label:text-is('copy me')");
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test("Copy id copies the previewed session's uuid and says so", async () => {
  await page.click(".rail-row-label:text-is('copy me')");
  const button = page.locator(".forest-preview-copy");
  await button.waitFor({ state: "visible" });
  assert.equal(await button.getAttribute("title"), UUID);
  await button.click();
  await page.waitForSelector(".forest-preview-copy:text-is('Copied')");
  assert.equal(await page.evaluate(() => (window as unknown as { copied: string }).copied), UUID);
  assert.ok(await page.isVisible(".forest-preview"), "copying leaves the preview open");
  if (process.env.SCREENSHOT) await page.screenshot({ path: process.env.SCREENSHOT });
  await page.waitForSelector(".forest-preview-copy:text-is('Copy id')", { timeout: 3000 });
});
