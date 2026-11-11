// Inline rename in the forest rail, end to end in the real app: double-click a
// row's label, type a name with spaces, Enter commits it as a localStorage
// override. Rows used to be <button>s, and a button turns a Space typed in a
// nested <input> into a click on itself — which reselected the row, blurred
// the editor and committed the name mid-word. Also covers the neighborhood:
// Enter must not launch the row, Esc must actually close the editor, clicks
// inside the editor stay in it, and clearing the name drops the override.
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
const LS_OVERRIDES = "eigenform:term:overrides:v1";

const UUID = "11111111-1111-4111-8111-111111111111";
const session = (uuid: string, title: string) => ({
  uuid,
  title,
  cwd: "/srv/eigenform",
  recency: new Date().toISOString(),
  live: false,
  state: "idle",
  spark: [],
});
const FOREST = [session(UUID, "auto title"), session("22222222-2222-4222-8222-222222222222", "other")];

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "rail-rename-"));
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

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector(".rail-row-label:text-is('auto title')");
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const overrides = () =>
  page.evaluate((k) => JSON.parse(localStorage.getItem(k) ?? "{}") as Record<string, string>, LS_OVERRIDES);
const tabCount = () => page.$$eval(".tab", (els) => els.length);

async function openEditor(label: string) {
  await page.dblclick(`.rail-row-label:text-is('${label}')`);
  await page.waitForSelector(".rail-rename-input:focus");
}

test("a name with spaces survives typing and commits on Enter, without launching", async () => {
  const tabs = await tabCount();
  await openEditor("auto title");
  await page.keyboard.type("my renamed session");
  assert.equal(await page.inputValue(".rail-rename-input"), "my renamed session");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".rail-row-label:text-is('my renamed session')");
  assert.equal(await page.$(".rail-rename-input"), null);
  assert.equal((await overrides())[UUID], "my renamed session");
  assert.equal(await tabCount(), tabs, "Enter in the editor does not launch the row");
});

test("clicking and double-clicking inside the editor keeps it open", async () => {
  const tabs = await tabCount();
  await openEditor("my renamed session");
  await page.click(".rail-rename-input");
  await page.dblclick(".rail-rename-input");
  assert.ok(await page.$(".rail-rename-input:focus"), "editor still focused");
  assert.equal(await tabCount(), tabs, "double-click in the editor does not launch");
  await page.keyboard.press("Escape");
});

test("Esc closes the editor and keeps the old name", async () => {
  await openEditor("my renamed session");
  await page.keyboard.press("Control+A");
  await page.keyboard.type("discarded");
  await page.keyboard.press("Escape");
  await page.waitForSelector(".rail-row-label:text-is('my renamed session')");
  assert.equal(await page.$(".rail-rename-input"), null);
  assert.equal((await overrides())[UUID], "my renamed session");
});

test("clearing the name drops the override, back to the automatic title", async () => {
  await openEditor("my renamed session");
  await page.keyboard.press("Control+A");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".rail-row-label:text-is('auto title')");
  assert.equal(UUID in (await overrides()), false);
});

test("committing an unchanged name stores no override", async () => {
  await openEditor("other");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".rail-row-label:text-is('other')");
  assert.deepEqual(await overrides(), {});
});
