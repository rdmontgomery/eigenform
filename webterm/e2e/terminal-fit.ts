// Regression test: the terminal's fitted grid must fit inside the visible
// .term-pane. The pane is padded under the global `box-sizing: border-box`, and
// FitAddon sizes rows from the computed height of xterm's parent element; if
// that height includes the padding, rows × cell height spills past the pane's
// content box and the bottom rows (Claude Code's status line) are clipped.
//
// Drives the real app (src/main.ts → mountShell) against a stubbed daemon, so
// the test follows whatever DOM shell.ts builds around xterm rather than
// mirroring it. One live pty is reported and pre-saved as an open tab, which
// makes boot restore it through the production openTabWithQuery path.
//
// Run: `npm run test:browser`. Lives outside src/ and test/ so plain `npm test`
// (node --test's default globs) stays browser-free. Needs Chromium for
// playwright-core; set CHROMIUM_PATH to use a preinstalled binary.
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

const SIZES: Array<[number, number]> = [
  [1280, 800],
  [1440, 900],
  [1013, 687], // odd sizes: the fit must never round a row past the edge
  [1100, 641],
  [1920, 1080],
];

// Sub-pixel slack for layout rounding; the padding overflow is a whole 6px.
const EPS = 0.5;

const PTY = {
  id: "pty-fit-test",
  cwd: "/tmp",
  uuid: null,
  state: "idle",
  spawnedAt: new Date(0).toISOString(),
  lastActivity: new Date(0).toISOString(),
};
const SAVED_TABS = [{ ptyId: PTY.id, label: "fit", cwd: "/tmp", kind: "terminal" }];

let dir: string;
let browser: Browser;
let page: Page;

before(async () => {
  dir = mkdtempSync(join(tmpdir(), "term-fit-"));
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
    "/api/forest": [],
    "/api/plan-reviews": [],
  };

  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: SIZES[0]![0], height: SIZES[0]![1] } });
  // The stubbed daemon: static assets + the boot-time JSON endpoints. Anything
  // else (webfonts, SSE streams, the pty WebSocket) fails, as it would offline.
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
  await page.waitForSelector(".term-pane .xterm-screen", { state: "attached" });
});

after(async () => {
  await browser?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

interface Fit {
  rows: number;
  cellHeight: number;
  contentTop: number;
  contentBottom: number;
  contentLeft: number;
  contentRight: number;
  gridTop: number;
  gridBottom: number;
  gridLeft: number;
  gridRight: number;
}

/** Where the rendered grid sits relative to the visible pane's content box. */
function measure(): Promise<Fit | null> {
  return page.evaluate(() => {
    const pane = [...document.querySelectorAll<HTMLElement>(".term-pane")]
      .find((p) => p.offsetParent !== null);
    const screen = pane?.querySelector<HTMLElement>(".xterm-screen");
    const rowsEl = pane?.querySelector<HTMLElement>(".xterm-rows");
    if (!pane || !screen || !rowsEl) return null;
    const cs = getComputedStyle(pane);
    const p = pane.getBoundingClientRect();
    const s = screen.getBoundingClientRect();
    const rows = rowsEl.children.length;
    return {
      rows,
      cellHeight: s.height / rows,
      contentTop: p.top + parseFloat(cs.paddingTop),
      contentBottom: p.bottom - parseFloat(cs.paddingBottom),
      contentLeft: p.left + parseFloat(cs.paddingLeft),
      contentRight: p.right - parseFloat(cs.paddingRight),
      gridTop: s.top,
      gridBottom: s.bottom,
      gridLeft: s.left,
      gridRight: s.right,
    };
  });
}

/** Resize the window and wait for the shell's coalesced refit to settle. */
async function resizeAndSettle(width: number, height: number): Promise<Fit> {
  await page.setViewportSize({ width, height });
  let last = "";
  for (let i = 0; i < 50; i++) {
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    const m = await measure();
    const key = JSON.stringify(m);
    if (m && m.rows > 0 && key === last) return m;
    last = key;
  }
  throw new Error(`terminal never settled at ${width}×${height}: ${last}`);
}

for (const [w, h] of SIZES) {
  test(`terminal grid fits the .term-pane content box at ${w}×${h}`, async () => {
    const m = await resizeAndSettle(w, h);
    const where = JSON.stringify(m);

    assert.ok(m.gridTop >= m.contentTop - EPS, `grid starts above the content box: ${where}`);
    assert.ok(m.gridLeft >= m.contentLeft - EPS, `grid starts left of the content box: ${where}`);
    assert.ok(
      m.gridBottom <= m.contentBottom + EPS,
      `${m.rows} rows × ${m.cellHeight}px overflow the pane by ` +
        `${(m.gridBottom - m.contentBottom).toFixed(1)}px: ${where}`,
    );
    assert.ok(m.gridRight <= m.contentRight + EPS, `columns overflow the pane: ${where}`);
    // Not under-fit either: one more row would not have fit.
    assert.ok(m.contentBottom - m.gridBottom < m.cellHeight, `fit left a whole row unused: ${where}`);
  });
}
