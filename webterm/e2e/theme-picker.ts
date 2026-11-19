// The theme picker lists every scheme under a "Dark" then a "Light" heading, and
// picking one recolors the app: the light/dark color-scheme follows the scheme.
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
import { SCHEMES } from "../src/themes/schemes.ts";

const ORIGIN = "http://eigenform.test";
const ROOT = fileURLToPath(new URL("..", import.meta.url));

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "theme-picker-"));
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
  const json: Record<string, unknown> = { "/api/pty": [], "/api/forest": [], "/api/plan-reviews": [] };

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

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector(".theme-btn");
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test("picker groups schemes as Dark then Light, and a pick sets the color scheme", async () => {
  await page.click(".theme-btn");
  await page.waitForSelector(".theme-pop .theme-row");

  // Headings and rows in document order: each row falls under the right heading.
  const items = await page.locator(".theme-pop .theme-group, .theme-pop .theme-row").evaluateAll((els) =>
    els.map((e) => (e.classList.contains("theme-group") ? `#${e.textContent}` : e.textContent!.trim())),
  );
  const dark = SCHEMES.filter((s) => s.dark).map((s) => s.name);
  const light = SCHEMES.filter((s) => !s.dark).map((s) => s.name);
  assert.deepEqual(items, ["#Dark", ...dark, "#Light", ...light]);
  if (process.env.SCREENSHOT) await page.screenshot({ path: process.env.SCREENSHOT });

  await page.locator(".theme-row", { hasText: "Rosé Pine Dawn" }).click();
  assert.equal(await page.evaluate(() => document.documentElement.style.colorScheme), "light");
  await page.locator(".theme-row", { hasText: /^Kanagawa Dragon$/ }).click();
  assert.equal(await page.evaluate(() => document.documentElement.style.colorScheme), "dark");
});
