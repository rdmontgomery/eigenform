// A session's peer name printed in a terminal is a link: hovering it underlines it,
// and clicking it opens that session in this window (a `?session=` pty socket), with
// no browser tab or `/s/` round trip. A word naming no session stays plain text. A
// session link the installed app captures (the browser hands it over through
// `launchQueue`) switches sessions the same way, without reloading the page.
//
// Drives the real app against a stubbed daemon whose pty socket prints one line.
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

const OTHER = "0000bbbb-0000-4000-8000-000000000002";
const PTY = {
  id: "7",
  cwd: "/srv/app",
  uuid: null,
  state: "idle",
  spawnedAt: new Date(0).toISOString(),
  lastActivity: new Date(0).toISOString(),
};
const FOREST = [
  {
    uuid: OTHER,
    title: "the other one",
    peer: "app-dd",
    cwd: "/srv/app",
    recency: new Date().toISOString(),
    live: false,
    state: "idle",
    spark: [],
  },
];

let dir: string;
let browser: Browser;
let page: Page;
const sockets: string[] = [];

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "term-link-"));
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
    "/api/pty": [PTY],
    "/api/forest": FOREST,
    "/api/plan-reviews": [],
    "/api/pins": [],
    "/api/resolve/app-dd": { session: { uuid: OTHER } },
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
  await context.routeWebSocket(/\/pty/, (ws) => {
    sockets.push(ws.url());
    if (new URL(ws.url()).searchParams.get("attach") !== PTY.id) return;
    ws.send(JSON.stringify({ type: "pty", id: PTY.id }));
    ws.send(Buffer.from("hand it to app-dd or nobody-known\r\n"));
  });
  await context.addInitScript(() => {
    const w = window as unknown as { __launch?: (p: unknown) => void };
    Object.defineProperty(window, "launchQueue", {
      value: { setConsumer: (f: (p: unknown) => void) => (w.__launch = f) },
    });
  });
  await context.addInitScript((id) => {
    localStorage.setItem(
      "eigenform:term:tabs:v1",
      JSON.stringify([{ ptyId: id, label: "here", cwd: "/srv/app", kind: "terminal" }]),
    );
  }, PTY.id);

  page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e));
  await page.goto(`${ORIGIN}/`);
  await page.waitForSelector(".xterm-rows :text('app-dd')");
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

/** The on-screen centre of `word` in the terminal's rendered rows. */
function centreOf(word: string) {
  return page.evaluate((word) => {
    const walker = document.createTreeWalker(document.querySelector(".xterm-rows")!, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const i = n.textContent!.indexOf(word);
      if (i < 0) continue;
      const r = document.createRange();
      r.setStart(n, i);
      r.setEnd(n, i + word.length);
      const b = r.getBoundingClientRect();
      return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
    }
    return null;
  }, word);
}

test("an unknown word is not a link", async () => {
  const at = await centreOf("nobody-known");
  assert.ok(at);
  await page.mouse.move(at.x, at.y);
  await page.waitForTimeout(200);
  assert.equal(
    await page.locator(".xterm-screen").evaluate((e) => e.classList.contains("xterm-cursor-pointer")),
    false,
  );
});

test("clicking a peer name opens that session in place", async () => {
  const at = await centreOf("app-dd");
  assert.ok(at);
  await page.mouse.move(at.x - 3, at.y);
  await page.mouse.move(at.x, at.y);
  await page.waitForFunction(() => document.querySelector(".xterm-screen")?.classList.contains("xterm-cursor-pointer"));
  await page.mouse.click(at.x, at.y);
  await waitFor(() => sockets.some((u) => new URL(u).searchParams.get("session") === OTHER));
  assert.equal(page.url(), `${ORIGIN}/`, "no navigation");
});

async function waitFor(cond: () => boolean, ms = 3000) {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error(`timed out; sockets: ${sockets.join(", ")}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

test("a captured /s/ link handed over by the browser opens the session in place", async () => {
  const active = () => page.locator(".tab--active").textContent();
  await page.locator(".tab", { hasText: "here" }).click();
  assert.match((await active()) ?? "", /here/);
  await page.evaluate((url) => {
    (window as unknown as { __launch: (p: unknown) => void }).__launch({ targetURL: url });
  }, `${ORIGIN}/s/app-dd`);
  await page.waitForFunction(() => /the other one/.test(document.querySelector(".tab--active")?.textContent ?? ""));
  assert.equal(page.url(), `${ORIGIN}/`, "no navigation");
});
