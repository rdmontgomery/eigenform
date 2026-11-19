// The Links panel groups a crowded session's links: pull requests first, then by
// site, or by who sent them (you, Claude, a subagent). Groups fold one by one and a
// filter box narrows the lot.
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

const UUID = "0000aaaa-0000-4000-8000-000000000001";
const FOREST = [
  {
    uuid: UUID,
    title: "busy",
    peer: null,
    cwd: "/srv/app",
    recency: new Date().toISOString(),
    live: false,
    state: "idle",
    spark: [],
  },
];
const docs = Array.from({ length: 6 }, (_, i) => `https://docs.example.org/page-${i}`).join(" ");
const EXCHANGES = [
  { n: 1, tok: 0, user: `look at https://github.com/acme/app/pull/12 and ${docs}` },
  {
    n: 2,
    tok: 0,
    user: "",
    assistant: "opened https://github.com/acme/app/pull/13, notes at https://example.com/notes",
    tool: {
      kind: "Agent",
      arg: "",
      delta: "",
      subagent: {
        agentType: "general-purpose",
        description: null,
        exchanges: [{ n: 1, tok: 0, user: "", assistant: "see https://example.com/from-sub" }],
      },
    },
  },
];

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "rail-links-"));
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
    "/api/pins": [],
    [`/api/session/${UUID}/json`]: { exchanges: EXCHANGES },
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
  await context.routeWebSocket(/\/pty/, () => {});
  await context.addInitScript((uuid) => {
    if (sessionStorage.getItem("seeded")) return;
    sessionStorage.setItem("seeded", "1");
    localStorage.setItem("eigenform:term:tabs:v1", JSON.stringify([{ uuid, label: "busy", cwd: "/srv/app" }]));
  }, UUID);

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector(".rail-links-group");
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const groups = () => page.locator(".rail-links-group-name").allTextContents();
const counts = () => page.locator(".rail-links-group .rail-group-count").allTextContents();

test("by site: pull requests first, then sites by size", async () => {
  assert.deepEqual(await groups(), ["Pull requests", "docs.example.org", "example.com"]);
  assert.deepEqual(await counts(), ["2", "6", "2"]);
  assert.deepEqual(
    await page.locator(".rail-links-group-body").first().locator(".rail-link-label").allTextContents(),
    ["acme/app#13", "acme/app#12"],
  );
  if (process.env.SCREENSHOT) await page.locator(".rail").screenshot({ path: process.env.SCREENSHOT });
});

test("a group folds on its own", async () => {
  await page.locator(".rail-links-group", { hasText: "docs.example.org" }).click();
  assert.equal(await page.locator(".rail-links-group-body").count(), 2);
  await page.locator(".rail-links-group", { hasText: "docs.example.org" }).click();
  assert.equal(await page.locator(".rail-links-group-body").count(), 3);
});

test("by source: you, Claude, subagents", async () => {
  await page.locator(".rail-links-mode", { hasText: "Source" }).click();
  assert.deepEqual(await groups(), ["Pasted by you", "From Claude", "From subagents"]);
  assert.deepEqual(await counts(), ["7", "2", "1"]);
});

test("the filter narrows every group and keeps focus while typing", async () => {
  await page.locator(".rail-links-filter").fill("pull");
  assert.deepEqual(await groups(), ["Pasted by you", "From Claude"]);
  assert.deepEqual(await counts(), ["1", "1"]);
  assert.equal(await page.evaluate(() => document.activeElement?.className), "rail-links-filter");
  await page.locator(".rail-links-filter").fill("zzz");
  assert.equal(await page.locator(".rail-links-none").textContent(), "no matching links");
  await page.locator(".rail-links-filter").fill("");
});
