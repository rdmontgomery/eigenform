// xterm 5.x does NOT inject its stylesheet — without this import the viewport/
// screen/rows render in normal flow and the terminal overflows the page.
import "@xterm/xterm/css/xterm.css";
import "./style.css";
import { mountShell } from "./shell.ts";

/**
 * Block the first paint until the terminal webfont is actually rasterised.
 *
 * xterm measures one glyph cell when `term.open()` runs and lays the entire grid
 * out from it. IBM Plex Mono arrives async from the Google Fonts CDN, so opening
 * a terminal before it loads measures the *fallback* (Menlo) — and the metrics
 * never self-correct when Plex swaps in, leaving every cell mis-spaced. Settling
 * the font first makes the measurement honest.
 *
 * Raced against a timeout so a slow or blocked CDN degrades to the fallback font
 * instead of hanging the app on a blank screen.
 */
async function ensureTermFontsReady(timeoutMs = 1500): Promise<void> {
  if (!("fonts" in document)) return;
  const probes = [
    '400 13px "IBM Plex Mono"',
    '500 13px "IBM Plex Mono"',
    '600 13px "IBM Plex Mono"',
  ];
  const ready = Promise.all([
    ...probes.map((spec) => document.fonts.load(spec).catch(() => undefined)),
    document.fonts.ready,
  ]);
  await Promise.race([
    ready,
    new Promise<void>((resolve) => setTimeout(resolve, timeoutMs)),
  ]);
}

async function boot(): Promise<void> {
  await ensureTermFontsReady();
  const app = document.getElementById("app");
  if (app) mountShell(app);
}

void boot();

// Dev live-reload: when the daemon (--dev) signals a frontend rebuild, reload.
// Tabs + ptys survive a reload (ptys live in the daemon; tabs reconcile on boot),
// so a full reload is safe here. Gated on the dev meta the daemon injects.
if (document.querySelector('meta[name="eigenform-dev"]')) {
  const ev = new EventSource("/api/dev/reload");
  let last = 0;
  ev.onmessage = () => {
    const now = Date.now();
    if (now - last < 300) return; // debounce esbuild's multi-file writes
    last = now;
    location.reload();
  };
}
