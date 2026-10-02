// Browser test for finding a tab: ⌘/Ctrl+Shift+F opens the filter, typing
// highlights the matching tabs and dims the rest, ↓ moves the selection, Enter
// jumps to it. Same stubbed-daemon harness as terminal-fit.ts: three live
// ptys pre-saved as open tabs, restored through the production boot path.
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

const pty = (id: string, cwd: string) => ({
  id,
  cwd,
  uuid: null,
  state: "idle",
  spawnedAt: new Date(0).toISOString(),
  lastActivity: new Date(0).toISOString(),
});
const PTYS = [
  pty("pty-a", "/srv/eigenform/webterm"),
  pty("pty-b", "/srv/notes"),
  pty("pty-c", "/srv/eigenform"),
];
const SAVED_TABS = [
  { ptyId: "pty-a", label: "status line", cwd: "/srv/eigenform/webterm", kind: "terminal" },
  { ptyId: "pty-b", label: "journal", cwd: "/srv/notes", kind: "terminal" },
  { ptyId: "pty-c", label: "forest recency", cwd: "/srv/eigenform", kind: "terminal" },
];

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "tab-search-"));
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
    "/api/forest": [],
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
  await page.waitForFunction(() => document.querySelectorAll(".tab").length === 3);
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

/** Each tab's label with its search state: "+" match, "*" selected match, "-" dimmed, " " idle. */
function strip(): Promise<string[]> {
  return page.$$eval(".tab", (tabs) =>
    tabs.map((t) => {
      const mark = t.classList.contains("tab--match-sel")
        ? "*"
        : t.classList.contains("tab--match")
          ? "+"
          : t.classList.contains("tab--nomatch")
            ? "-"
            : " ";
      return mark + t.querySelector(".tab-label")!.textContent;
    }),
  );
}

const activeLabel = () => page.$eval(".tab--active .tab-label", (e) => e.textContent);

test("shortcut opens the filter; typing highlights matches and dims the rest", async () => {
  await page.keyboard.press("Control+Shift+F");
  await page.waitForSelector(".tab-search--open .tab-search-input:focus");
  assert.deepEqual(await strip(), [" status line", " journal", " forest recency"]);

  await page.keyboard.type("eigenform");
  assert.deepEqual(await strip(), ["*status line", "-journal", "+forest recency"]);
  assert.equal(await page.textContent(".tab-search-count"), "1/2");
});

test("arrow moves the selection, Enter jumps to it and closes the filter", async () => {
  await page.keyboard.press("ArrowDown");
  assert.deepEqual(await strip(), ["+status line", "-journal", "*forest recency"]);
  await page.keyboard.press("Enter");
  assert.equal(await activeLabel(), "forest recency");
  assert.equal(await page.$(".tab-search--open"), null);
  assert.deepEqual(await strip(), [" status line", " journal", " forest recency"]);
});

test("no match flags the box; Esc closes without switching tabs", async () => {
  await page.click(".tab-search-btn");
  await page.keyboard.type("zzz");
  assert.deepEqual(await strip(), ["-status line", "-journal", "-forest recency"]);
  assert.ok(await page.$(".tab-search--miss"));
  await page.keyboard.press("Escape");
  assert.equal(await page.$(".tab-search--open"), null);
  assert.equal(await activeLabel(), "forest recency");
});
