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
import { loadSession, loadForest, fetchSession, forkSession, type ForestEntry, type Session } from "./data.ts";
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
let current: Session = session; // the session data the Manuscript is currently showing
let currentUuid: string | null = null; // its full uuid (the fork source), null for the stub
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
  onCommit: (n, text) => commit(n, text),
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
  const t = buildForkToast(info, current);
  center.appendChild(t);
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.remove(), 4200);
}

function commit(n: number, text: string): void {
  const c = cache();
  const fork = forkReading(n, c, current.total);
  const e = current.exchanges.find((x) => x.n === n);
  const src = currentUuid; // fork the session being VIEWED, resumed or not
  const turn = e?.uuid;
  const live = Boolean(src && turn); // a real fork needs a real source + a turn uuid
  manuscript.closeEdit();

  const proceed = (): void => {
    if (live && src && turn) void doFork(src, turn, text, fork);
    // no real source (the built-in sample): be honest rather than fake success
    else showError("This is the sample — pick a session from the Forest to fork a real one.");
  };

  if (c.cold) {
    const scrim = buildColdConfirm(fork, c, () => { scrim.remove(); proceed(); }, () => scrim.remove(), current);
    center.appendChild(scrim);
  } else {
    proceed();
  }
}

// Real edit-then-fork: write the branch, announce it, then ENTER it — resume it in the
// Furnace and follow it in the Manuscript (selectSession), and it appears in the Forest.
async function doFork(src: string, turn: string, text: string, fork: ReturnType<typeof forkReading>): Promise<void> {
  const newUuid = await forkSession(src, turn, text);
  if (!newUuid) { showError("fork failed — the source is untouched"); return; }
  showToast({ kind: "fork", n: fork.n, drops: fork.drops, prefix: fork.prefix, cold: fork.cold });
  relight();
  // the branch rewinds to before the edited turn; deliver the edited prompt live once the
  // resumed session has painted and gone idle (auto-send).
  pendingPrompt = text.trim() || null;
  selectSession({ uuid: newUuid, id: newUuid.slice(0, 6), name: "fork", turns: 0, branches: 0, active: true, shape: [] });
}

function showError(msg: string): void {
  const t = el("div", { class: "toast" }, el("span", { class: "msg", text: msg }));
  center.appendChild(t);
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.remove(), 4200);
}

function sendLeaf(text: string): void {
  const sent = sendPrompt(text);
  relight();
  showToast({ kind: "send" });
  if (sent) beginLive(); // stream the response into the Manuscript until it lands
}

// ── the live pty (the real Furnace stream) ──────────────────────────────────
// The Furnace is a dark instrument in BOTH themes (its bg is dark in paper too), so the
// terminal foreground is always light — never the page ink, which would go dark-on-dark.
const TERM_FG = PALETTES.furnace.ink;
const term = new Terminal({
  fontFamily: '"IBM Plex Mono", ui-monospace, Menlo, Consolas, monospace',
  fontSize: 12,
  cursorBlink: true,
  theme: { background: PALETTES[theme].furnaceBg, foreground: TERM_FG },
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
  term.options.theme = { background: PALETTES[theme].furnaceBg, foreground: TERM_FG };
}

let ws: WebSocket | null = null;
let onData: { dispose(): void } | null = null;
let es: EventSource | null = null;
let activeSession: string | null = null;

// After a fork resumes, the edited prompt is delivered live. We can't know exactly when
// claude's TUI is ready, so we send once its output has been quiet for a beat (it has
// finished painting the resumed transcript and is idling at the prompt).
let pendingPrompt: string | null = null;
let pendingTimer: number | undefined;
function schedulePendingSend(): void {
  if (!pendingPrompt) return;
  window.clearTimeout(pendingTimer);
  pendingTimer = window.setTimeout(() => {
    if (pendingPrompt && sendPrompt(pendingPrompt)) { pendingPrompt = null; beginLive(); }
  }, 1500);
}

// ── live turn streaming ─────────────────────────────────────────────────────
// The JSONL only persists the assistant turn at completion, so during generation the
// Manuscript would sit dead. Instead we tap the pty: xterm has already parsed the TUI,
// so we read its buffer tail into a live "responding" region until the turn lands.
let inFlight = false;
let liveStart = 0;
let liveTicker: number | undefined;
let liveQuiet: number | undefined;
let liveThrottle: number | undefined;

function beginLive(): void {
  clearLive();
  inFlight = true;
  liveStart = Date.now();
  renderLive();
  resetQuiet();
  liveTicker = window.setInterval(renderLive, 1000);
}
function onPtyOutput(): void {
  if (!inFlight) return;
  resetQuiet();
  if (liveThrottle === undefined) {
    liveThrottle = window.setTimeout(() => { liveThrottle = undefined; renderLive(); }, 100);
  }
}
function resetQuiet(): void {
  window.clearTimeout(liveQuiet);
  liveQuiet = window.setTimeout(endLive, 2500); // pty quiet for 2.5s ⇒ the turn finished
}
function renderLive(): void {
  if (!inFlight) return;
  manuscript.setLive(termTail(), Math.floor((Date.now() - liveStart) / 1000));
}
function stopLiveTimers(): void {
  window.clearInterval(liveTicker);
  window.clearTimeout(liveQuiet);
  window.clearTimeout(liveThrottle);
  liveThrottle = undefined;
}
function clearLive(): void {
  // hard clear (session switch) — drop the region immediately
  inFlight = false;
  stopLiveTimers();
  manuscript.setLive(null);
}
async function endLive(): Promise<void> {
  // seamless settle: render the clean landed turn FIRST, then remove the live region in
  // the same continuation (one paint) so streaming and the new leaf swap without a flash.
  if (!inFlight) return;
  inFlight = false;
  stopLiveTimers();
  if (activeSession) await renderManuscript(activeSession);
  manuscript.setLive(null);
}

// The last meaningful lines of the live terminal. xterm has parsed the TUI for us, so we
// read its buffer (newest-first, bounded) rather than the raw ANSI, and drop input-box /
// hint chrome. Imperfect by nature — claude's TUI redraws — but it shows real motion.
function termTail(maxLines = 16): string {
  const buf = term.buffer.active;
  const out: string[] = [];
  for (let y = buf.length - 1; y >= 0 && out.length < maxLines; y--) {
    const line = buf.getLine(y);
    if (!line) continue;
    const s = line.translateToString(true);
    if (!s.trim() || isChrome(s)) continue;
    out.push(s);
  }
  return out.reverse().join("\n");
}
function isChrome(s: string): boolean {
  const t = s.trim();
  if (/^[╭╮╰╯│─└┘┌┐\s]*$/.test(t)) return true; // box-drawing only
  if (t.startsWith("❯")) return true; // the input prompt line
  if (/shortcuts|esc to interrupt|\? for|bypass permissions|⏵/i.test(t)) return true;
  return false;
}

function sendResize(): void {
  try { fit.fit(); } catch { /* hidden */ }
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
}
window.addEventListener("resize", sendResize);

function connectPty(query = ""): void {
  if (ws) { ws.onclose = null; ws.close(); }
  if (onData) { onData.dispose(); onData = null; }
  clearLive(); // a session switch ends any in-flight live region
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
    if (ev.data instanceof ArrayBuffer) { term.write(new Uint8Array(ev.data)); schedulePendingSend(); onPtyOutput(); return; }
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
  current = s;
  currentUuid = uuid;
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

// ── startup: a shell in the Furnace (no tokens), the Forest from disk, and a
//    read-only preview of the most recent real session so the Manuscript shows
//    (and can fork) real data by default — the built-in sample is only a fallback.
theme = currentTheme();
connectPty();
void refreshForest();
devLiveReload();
void previewRecent();

async function previewRecent(): Promise<void> {
  try {
    const uuid = (await (await fetch("/api/recent")).text()).trim();
    if (uuid) followManuscript(uuid); // render + follow, but DON'T resume the pty (no tokens)
  } catch {
    /* no sessions — keep the sample */
  }
}
