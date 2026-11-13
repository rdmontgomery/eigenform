// Browser test for the overstory's sort menu (a one-off reorder by last message
// sent, or grouped by directory) and the cache clock's contrast. Same
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

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const now = Date.now();
const minsAgo = (m: number) => new Date(now - m * 60_000).toISOString();
// label, cwd, minutes since the last prompt (null: none found)
const SESSIONS: [string, string, number | null][] = [
  ["alpha", "/u/src/app/main", 30],
  ["beta", "/u/notes", 5],
  ["gamma", "/u/src/app", 50],
  ["delta", "/u/src/other", null],
  ["epsilon", "/u/src/app/feat-x", 1],
];
const PTYS = SESSIONS.map(([, cwd], n) => ({
  id: `pty-${n}`,
  cwd,
  uuid: uuid(n),
  state: "idle",
  spawnedAt: new Date(0).toISOString(),
  lastActivity: new Date(0).toISOString(),
}));
const SAVED_TABS = SESSIONS.map(([label, cwd], n) => ({ ptyId: `pty-${n}`, uuid: uuid(n), label, cwd }));
const FOREST = SESSIONS.map(([label, cwd, sent], n) => ({
  uuid: uuid(n),
  title: label,
  cwd,
  recency: new Date(now).toISOString(),
  live: true,
  state: "ready",
  spark: [],
  cacheAt: minsAgo(10),
  cacheTtl: 3600,
  lastPromptAt: sent === null ? null : minsAgo(sent),
}));

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "overstory-sort-"));
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
    localStorage.setItem("eigenform:term:theme:v2", "warm-ink-light");
  }, SAVED_TABS);

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForFunction((n) => document.querySelectorAll(".tab").length === n, SESSIONS.length);
  await page.waitForFunction(() => document.querySelectorAll(".tab-cache").length > 0);
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const order = () =>
  page.$$eval(".tab .tab-label", (labels) => labels.map((l) => l.textContent));

async function pick(choice: string) {
  await page.click(".topbar-sort-btn");
  await page.click(`.tab-order-menu .snooze-opt:has-text("${choice}")`);
}

test("last messaged, newest first; a session with no prompt goes last", async () => {
  await pick("newest first");
  assert.deepEqual(await order(), ["epsilon", "beta", "alpha", "gamma", "delta"]);
});

test("last messaged, oldest first", async () => {
  await pick("oldest first");
  assert.deepEqual(await order(), ["gamma", "alpha", "beta", "epsilon", "delta"]);
});

test("grouping after a sort keeps the sort inside each directory", async () => {
  await pick("newest first");
  await pick("Group by directory");
  assert.deepEqual(await order(), ["epsilon", "alpha", "gamma", "beta", "delta"]);
});

test("the order is saved, and the tabs stay put when the roster refreshes", async () => {
  const saved = await page.evaluate(() =>
    (JSON.parse(localStorage.getItem("eigenform:term:tabs:v1")!) as { label: string }[]).map((t) => t.label),
  );
  assert.deepEqual(saved, ["epsilon", "alpha", "gamma", "beta", "delta"]);
  await page.waitForTimeout(3500);
  assert.deepEqual(await order(), saved);
});

/** WCAG contrast ratio between two `rgb(...)` colours. */
function contrast(a: string, b: string): number {
  const lum = (c: string) => {
    const [r, g, bl] = c.match(/[\d.]+/g)!.slice(0, 3).map(Number).map((v) => {
      const s = v / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * bl!;
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

test("the cache clock reads against its tab in a light theme, active or not", async () => {
  const pairs = await page.$$eval(".tab", (tabs) =>
    tabs.map((t) => {
      const clock = t.querySelector(".tab-cache") as HTMLElement;
      const toRgb = (c: string) => {
        const cv = document.createElement("canvas").getContext("2d")!;
        cv.fillStyle = c;
        cv.fillRect(0, 0, 1, 1);
        const [r, g, b] = cv.getImageData(0, 0, 1, 1).data;
        return `rgb(${r}, ${g}, ${b})`;
      };
      let bgEl: HTMLElement | null = t as HTMLElement;
      let bg = "rgba(0, 0, 0, 0)";
      while (bgEl && /rgba\(0, 0, 0, 0\)|transparent/.test(bg)) {
        bg = getComputedStyle(bgEl).backgroundColor;
        bgEl = bgEl.parentElement;
      }
      return { active: t.classList.contains("tab--active"), fg: toRgb(getComputedStyle(clock).color), bg: toRgb(bg) };
    }),
  );
  for (const p of pairs) {
    assert.ok(contrast(p.fg, p.bg) >= 4.5, `${p.active ? "active" : "inactive"} tab: ${p.fg} on ${p.bg}`);
  }
});
