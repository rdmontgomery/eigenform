// main.ts — composes the workbench and owns the live cache clock + interaction state.
// The furnace cools in real time; typing the leaf or committing a fork re-lights it.
// The Manuscript/Mind/costs are stubbed (see data.ts); the Furnace pane, the Forest,
// new-session and the leaf→pty path are LIVE, preserved from the original surface.
import "@xterm/xterm/css/xterm.css";
import "./woland.css";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

import { el } from "./dom.ts";
import { applyTheme, currentTheme, PALETTES, type ThemeName } from "./theme.ts";
import { cacheReading, forkReading, tempColor, type CacheReading, SEED_IDLE, TTL_DEFAULT, TTL_EXTENDED } from "./cooling.ts";
import { loadSession, loadForest, fetchSession, type ForestEntry } from "./data.ts";
import { buildClock } from "./clock.ts";
import { buildMind } from "./mind.ts";
import { Manuscript } from "./manuscript.ts";
import { buildMasthead, buildForest, buildFurnace, buildColdConfirm, buildForkToast, type ToastInfo } from "./shell.ts";
import { mindGlyph } from "./marks.ts";

// ── state ──────────────────────────────────────────────────────────────────
let idle = SEED_IDLE;
let ttl = TTL_DEFAULT;
let theme: ThemeName = "furnace";
let lensOn = false;
let toastTimer: number | undefined;
const session = loadSession();
const cache = (): CacheReading => cacheReading(idle, ttl);

applyTheme(theme);

// ── components ───────────────────────────────────────────────────────────
const masthead = buildMasthead(session, theme, () => {
  theme = theme === "furnace" ? "paper" : "furnace";
  applyTheme(theme);
  masthead.setTheme(theme);
  retheme();
});

const clock = buildClock({
  onMind: () => mind.setOpen(true),
  onExtend: () => { ttl = ttl === TTL_DEFAULT ? TTL_EXTENDED : TTL_DEFAULT; applyCache(); },
  extended: () => ttl === TTL_EXTENDED,
});

const mind = buildMind(() => mind.setOpen(!mind.isOpen()));

const manuscript = new Manuscript(session, {
  getCache: cache,
  onCommit: (n) => commit(n),
  onLeafSend: (text) => sendLeaf(text),
});

const forest = buildForest((entry) => selectSession(entry));
const furnace = buildFurnace();
furnace.onToggle(() => { furnace.setOpen(!furnaceOpen()); });

const lensBtn = el("button", { class: "ghost lens-toggle", title: "show what entered/left the mind at each turn", onclick: () => {
  lensOn = !lensOn;
  lensBtn.classList.toggle("on", lensOn);
  manuscript.setLens(lensOn);
} }, mindGlyph("var(--dim)", 12), "per-turn Δ");

const center = el("div", { class: "center" },
  el("div", { class: "ms-head" },
    el("div", { class: "eyebrow", text: "The Manuscript" }),
    el("div", { class: "right" },
      el("span", { class: "tag", text: "one session · one cache · one clock" }),
      lensBtn)),
  mind.node,
  manuscript.node);

const root = el("div", { class: "wb" },
  masthead.node, clock.node,
  el("div", { class: "body" }, forest.node, center, furnace.node));

document.getElementById("root")!.replaceChildren(root);

// ── the cooling tick — the only per-second update ──────────────────────────
function applyCache(): void {
  const c = cache();
  root.style.setProperty("--temp", String(c.temp));
  root.style.setProperty("--temp-color", tempColor(c.temp));
  root.classList.toggle("cold", c.cold);
  root.classList.toggle("urgent", !c.cold && c.remaining <= 45);
  clock.update(c);
  manuscript.tick(c);
}
applyCache();
window.setInterval(() => { idle += 1; applyCache(); }, 1000);

function relight(): void { idle = 0; applyCache(); }

// ── fork / leaf moments ────────────────────────────────────────────────────
function showToast(info: ToastInfo): void {
  const t = buildForkToast(info, session);
  center.appendChild(t);
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.remove(), 4200);
}

function commit(n: number): void {
  const c = cache();
  const fork = forkReading(n, c);
  manuscript.closeEdit();
  if (c.cold) {
    const scrim = buildColdConfirm(fork, c, () => { scrim.remove(); showToast({ kind: "fork", n, drops: fork.drops, prefix: fork.prefix, cold: true }); relight(); }, () => scrim.remove(), session);
    center.appendChild(scrim);
    return;
  }
  showToast({ kind: "fork", n, drops: fork.drops, prefix: fork.prefix, cold: false });
  relight();
}

function sendLeaf(text: string): void {
  if (sendPrompt(text)) { /* piped to the live pty */ }
  relight();
  showToast({ kind: "send" });
}

// ── the live pty (the real Furnace stream) ──────────────────────────────────
const term = new Terminal({
  fontFamily: '"IBM Plex Mono", ui-monospace, Menlo, Consolas, monospace',
  fontSize: 12,
  cursorBlink: true,
  theme: { background: PALETTES[theme].furnaceBg, foreground: PALETTES[theme].ink },
});
const fit = new FitAddon();
term.loadAddon(fit);
term.open(furnace.termHost);

let furnaceIsOpen = false;
function furnaceOpen(): boolean { return furnaceIsOpen; }
const realSetOpen = furnace.setOpen;
furnace.setOpen = (open: boolean) => {
  furnaceIsOpen = open;
  realSetOpen(open);
  if (open) queueMicrotask(() => { try { fit.fit(); sendResize(); term.focus(); } catch { /* not yet sized */ } });
};

function retheme(): void {
  term.options.theme = { background: PALETTES[theme].furnaceBg, foreground: PALETTES[theme].ink };
}

let ws: WebSocket | null = null;
let onData: { dispose(): void } | null = null;
let es: EventSource | null = null;
let activeSession: string | null = null;

function sendResize(): void {
  try { fit.fit(); } catch { /* hidden */ }
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
}
window.addEventListener("resize", sendResize);

function connectPty(query = ""): void {
  if (ws) { ws.onclose = null; ws.close(); }
  if (onData) { onData.dispose(); onData = null; }
  term.reset();
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const sock = new WebSocket(`${proto}://${location.host}/pty${query}`);
  sock.binaryType = "arraybuffer";
  ws = sock;
  sock.onopen = () => {
    sendResize();
    onData = term.onData((d) => { if (sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify({ type: "stdin", data: d })); });
    term.focus();
  };
  sock.onmessage = (ev) => {
    if (ev.data instanceof ArrayBuffer) { term.write(new Uint8Array(ev.data)); return; }
    try { const msg = JSON.parse(ev.data as string); if (msg.type === "session" && typeof msg.uuid === "string") onSessionBorn(msg.uuid); } catch { /* non-JSON */ }
  };
  sock.onclose = () => term.write("\r\n\x1b[2m[woland: pty disconnected]\x1b[0m\r\n");
}

// Bracketed-paste the (possibly multi-line) text so claude's TUI inserts it literally,
// then a SEPARATE \r so it reads as a discrete Enter — same idea as tmux send-keys.
function sendPrompt(text: string): boolean {
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;
  const sock = ws;
  sock.send(JSON.stringify({ type: "stdin", data: `\x1b[200~${text}\x1b[201~` }));
  window.setTimeout(() => { if (sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify({ type: "stdin", data: "\r" })); }, 60);
  return true;
}

function selectSession(entry: ForestEntry): void {
  if (!entry.uuid) return;
  activeSession = entry.uuid;
  connectPty(`?session=${encodeURIComponent(entry.uuid)}`);
  furnace.setOpen(true);
  followManuscript(entry.uuid);
  void refreshForest(entry.uuid);
}

function onSessionBorn(uuid: string): void {
  activeSession = uuid;
  followManuscript(uuid);
  void refreshForest(uuid);
}

// Render the chosen session into the Manuscript and follow it live: re-fetch the
// structured transcript on each SSE 'change', pinned to the leaf if we were near it.
function followManuscript(uuid: string): void {
  void renderManuscript(uuid);
  if (es) es.close();
  es = new EventSource(`/api/watch/${encodeURIComponent(uuid)}`);
  es.onmessage = () => void renderManuscript(uuid);
}

async function renderManuscript(uuid: string): Promise<void> {
  const s = await fetchSession(uuid);
  if (!s) return;
  const scroller = manuscript.scroller;
  const nearBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  const prev = scroller.scrollTop;
  manuscript.setSession(s);
  masthead.setSession(s);
  scroller.scrollTop = nearBottom ? scroller.scrollHeight : prev;
}

async function refreshForest(activeUuid?: string): Promise<void> {
  const entries = await loadForest();
  if (activeUuid) for (const e of entries) e.active = e.uuid === activeUuid;
  forest.fill(entries);
}

// ── dev live-reload — never drops a live session ────────────────────────────
function devLiveReload(): void {
  if (!document.querySelector('meta[name="eigen-dev"]')) return;
  const ev = new EventSource("/api/dev/reload");
  let last = 0;
  ev.onmessage = () => {
    const now = Date.now();
    if (now - last < 300) return;
    last = now;
    if (activeSession) { console.warn("[woland dev] frontend changed — refresh manually to keep the live session"); return; }
    location.reload();
  };
}

// ── startup: a shell in the Furnace (no tokens), the Forest from disk ────────
theme = currentTheme();
connectPty();
void refreshForest();
devLiveReload();
