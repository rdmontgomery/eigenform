// The theme editor repaints the chrome live as colors change, saves the edit as a
// custom theme through the daemon's /api/themes (stubbed here in memory), restores
// it on reload, and imports an .itermcolors file.
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

let dir: string;
let browser: Browser;
let page: Page;
let themes: { id: string; name: string; theme: Record<string, string>; overrides?: Record<string, string> }[] = [];

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "theme-editor-"));
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
    const req = route.request();
    const url = new URL(req.url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname === "/api/themes") return route.fulfill({ json: themes });
    const one = /^\/api\/themes\/(.+)$/.exec(url.pathname);
    if (one && req.method() === "PUT") {
      const t = req.postDataJSON();
      themes = [...themes.filter((x) => x.id !== t.id), t];
      return route.fulfill({ json: themes });
    }
    if (one && req.method() === "DELETE") {
      themes = themes.filter((x) => x.id !== one[1]);
      return route.fulfill({ json: themes });
    }
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

const token = (name: string) =>
  page.evaluate((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(), name);
const hexInput = (label: string) => page.locator(".te-row", { has: page.locator(`.te-label:text-is('${label}')`) }).locator(".te-hex");

test("edits preview live, Cancel restores, Save keeps a custom theme across reload", async () => {
  const bg = await token("--bg");
  await page.click(".theme-btn");
  await page.click("text=Edit colors…");

  await hexInput("Background").fill("#203040");
  await page.waitForFunction((before) => getComputedStyle(document.documentElement).getPropertyValue("--bg").trim() !== before, bg);
  await page.click(".te-btn:text-is('Cancel')");
  assert.equal(await token("--bg"), bg, "cancel puts the old theme back");

  await page.click(".theme-btn");
  await page.click("text=Edit colors…");
  await hexInput("Background").fill("#203040");
  await hexInput("Accent").fill("#33ccff");
  await page.fill(".te-name", "Harbor");
  await page.click(".te-btn:text-is('Save as theme')");
  await page.waitForSelector(".theme-editor", { state: "detached" });

  assert.equal(themes.length, 1);
  assert.equal(themes[0]!.id, "custom-harbor");
  assert.equal(themes[0]!.theme.background, "#203040");
  assert.deepEqual(themes[0]!.overrides, { accent: "#33ccff" });

  const edited = await token("--bg");
  await page.reload();
  await page.waitForSelector(".theme-btn");
  assert.equal(await token("--bg"), edited, "the saved theme is active after reload");
  assert.equal(await page.getAttribute(".theme-btn", "title"), "Theme — Harbor");
});

test("Reset to original restores the built-in palette; Delete drops the theme", async () => {
  await page.click(".theme-btn");
  await page.click("text=Edit colors…");
  await page.click(".te-btn:text-is('Reset to original')");
  assert.equal(await hexInput("Background").inputValue(), "#110b07");
  await page.click(".te-btn:text-is('Delete')");
  await page.waitForSelector(".theme-editor", { state: "detached" });
  assert.deepEqual(themes, []);
  assert.equal(await page.getAttribute(".theme-btn", "title"), "Theme — Warm Ink");
});

test("importing an .itermcolors file adds and applies it", async () => {
  const color = (k: string, r: number, g: number, b: number) =>
    `<key>${k}</key><dict><key>Blue Component</key><real>${b}</real><key>Green Component</key><real>${g}</real><key>Red Component</key><real>${r}</real></dict>`;
  const keys = ["Background Color", "Foreground Color", "Cursor Color", ...Array.from({ length: 16 }, (_, i) => `Ansi ${i} Color`)];
  const xml = `<plist version="1.0"><dict>${keys.map((k, i) => color(k, i === 1 ? 1 : 0.1, 0.2, i === 0 ? 0.1 : 0.5)).join("")}</dict></plist>`;

  await page.click(".theme-btn");
  await page.setInputFiles(".theme-pop input[type=file]", { name: "Deep Sea.itermcolors", mimeType: "application/xml", buffer: Buffer.from(xml) });
  await page.waitForSelector(".theme-row--on .theme-row-name:text-is('Deep Sea')");
  assert.equal(themes[0]?.id, "custom-deep-sea");
  assert.equal(themes[0]?.theme.background, "#1a331a");
});
