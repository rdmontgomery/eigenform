// The theme picker: every scheme under "Dark" then "Light"; a pick recolors the app and
// lands in "Recent"; the editor reads a pasted scheme and saves it to /api/themes
// (stubbed here in memory) as a "Custom" theme; Shimmer shows its overlay.
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
let themes: { id: string; name: string; theme: Record<string, string> }[] = [];

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
    const req = route.request();
    const url = new URL(req.url());
    if (url.origin !== ORIGIN) return route.abort();
    if (url.pathname === "/api/themes") return route.fulfill({ json: themes });
    const put = /^\/api\/themes\/(.+)$/.exec(url.pathname);
    if (put && req.method() === "PUT") {
      const body = req.postDataJSON() as { name: string; theme: Record<string, string> };
      themes = [...themes.filter((t) => t.id !== put[1]), { id: put[1]!, ...body }];
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

const items = () =>
  page.locator(".theme-pop .theme-group, .theme-pop .theme-row").evaluateAll((els) =>
    els.map((e) => (e.classList.contains("theme-group") ? `#${e.textContent}` : e.textContent!.trim())),
  );
const colorScheme = () => page.evaluate(() => document.documentElement.style.colorScheme);
const cssVar = (name: string) =>
  page.evaluate((n) => document.documentElement.style.getPropertyValue(n), name);

test("picker groups schemes as Dark then Light; picks set the color scheme and fill Recent", async () => {
  await page.click(".theme-btn");
  await page.waitForSelector(".theme-pop .theme-row");

  const dark = SCHEMES.filter((s) => s.dark).map((s) => s.name);
  const light = SCHEMES.filter((s) => !s.dark).map((s) => s.name);
  assert.deepEqual(await items(), ["#Dark", ...dark, "#Light", ...light]);
  if (process.env.SCREENSHOT) await page.screenshot({ path: process.env.SCREENSHOT });

  await page.locator(".theme-row-pick", { hasText: "Rosé Pine Dawn" }).click();
  assert.equal(await colorScheme(), "light");
  await page.locator(".theme-row-pick", { hasText: /^Kanagawa Dragon$/ }).click();
  assert.equal(await colorScheme(), "dark");

  const list = await items();
  assert.deepEqual(list.slice(0, 3), ["#Recent", "Kanagawa Dragon", "Rosé Pine Dawn"]);
});

test("a pasted scheme previews live, saves as a Custom theme, and Cancel restores", async () => {
  const before = await cssVar("--bg");
  await page.locator(".theme-new").click();
  await page.waitForSelector(".theme-edit-paste");

  await page.fill(".theme-edit-paste", [
    "# kitty", "foreground #e0e0e0", "background #300030", "color1 #ff5555", "color4 #5577ff",
  ].join("\n"));
  await page.waitForSelector(".theme-edit-status:text('Read 4 colors.')");
  assert.notEqual(await cssVar("--bg"), before, "previewing");
  assert.equal(await page.inputValue(".theme-field-hex[aria-label='Background hex']"), "#300030");

  await page.locator(".theme-edit-btn", { hasText: "Cancel" }).click();
  assert.equal(await cssVar("--bg"), before, "Cancel restores");

  await page.locator(".theme-new").click();
  await page.fill(".theme-edit-paste", "background #300030\nforeground #e0e0e0");
  await page.fill(".theme-edit-name", "Plum");
  if (process.env.SCREENSHOT_EDIT) await page.screenshot({ path: process.env.SCREENSHOT_EDIT });
  await page.locator(".theme-edit-btn", { hasText: "Save" }).click();
  await page.waitForSelector(".theme-group:text-is('Custom')");

  assert.deepEqual(themes.map((t) => [t.id, t.name, t.theme.background]), [["plum", "Plum", "#300030"]]);
  const list = await items();
  assert.deepEqual(list.slice(list.indexOf("#Custom"), list.indexOf("#Custom") + 2), ["#Custom", "Plum"]);
  assert.equal(await page.locator(".theme-row--on .theme-row-name").first().textContent(), "Plum");
});

test("Shimmer shows the rolling overlay, Off hides it", async () => {
  const shown = () => page.locator(".shimmer").evaluate((e) => !(e as HTMLElement).hidden);
  assert.equal(await shown(), false);
  await page.locator(".theme-seg-btn", { hasText: "Vivid" }).click();
  assert.equal(await shown(), true);
  assert.match(await page.locator(".shimmer-band").evaluate((e) => (e as HTMLElement).style.backgroundImage), /linear-gradient/);
  if (process.env.SCREENSHOT_SHIMMER) {
    await page.keyboard.press("Escape");
    await page.screenshot({ path: process.env.SCREENSHOT_SHIMMER });
    await page.click(".theme-btn");
  }
  await page.locator(".theme-seg-btn", { hasText: "Off" }).click();
  assert.equal(await shown(), false);
});
