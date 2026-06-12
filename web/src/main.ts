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
import { loadDensity, saveDensity, loadForestWidth, saveForestWidth, clampForestWidth, type Density } from "./prefs.ts";
import { cacheReading, forkReading, tempColor, type CacheReading, SEED_IDLE, TTL_DEFAULT, TTL_EXTENDED } from "./cooling.ts";
import { loadSession, emptySession, loadForest, watchForest, fetchSession, forkSession, type ForestEntry, type Session } from "./data.ts";
import { buildClock } from "./clock.ts";
import { buildMind } from "./mind.ts";
import { Manuscript } from "./manuscript.ts";
import { buildMasthead, buildForest, buildFurnace, buildColdConfirm, buildForkToast, type ToastInfo } from "./shell.ts";
import { mindGlyph } from "./marks.ts";

// ── state ──────────────────────────────────────────────────────────────────
let idle = SEED_IDLE;
let ttl = TTL_DEFAULT;
let theme: ThemeName = "furnace";
let density: Density = loadDensity();
let lensOn = false;
let toastTimer: number | undefined;
const session = loadSession();
let current: Session = session; // the session data the Manuscript is currently showing
let currentUuid: string | null = null; // its full uuid (the fork source), null for the stub
const cache = (): CacheReading => cacheReading(idle, ttl);

applyTheme(theme);
document.documentElement.dataset.density = density; // drives --scale + compact padding

// ── components ───────────────────────────────────────────────────────────
const masthead = buildMasthead(session, theme, density, () => {
  theme = theme === "furnace" ? "paper" : "furnace";
  applyTheme(theme);
  masthead.setTheme(theme);
  retheme();
}, () => {
  density = density === "normal" ? "compact" : "normal";
  applyDensity(density);
  saveDensity(density);
  masthead.setDensity(density);
});

// data-density on the root drives the CSS (--scale + padding overrides); the xterm
// font is JS-only, so we set it here and refit if the Furnace is open.
function applyDensity(d: Density): void {
  document.documentElement.dataset.density = d;
  term.options.fontSize = d === "compact" ? 11 : 12;
  if (furnaceIsOpen) { try { fit.fit(); sendResize(); } catch { /* hidden */ } }
}

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

const forest = buildForest((entry) => selectSession(entry), (cwd) => startNewSession(cwd));
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

// Draggable splitter on the Forest's right edge — sets --forest-w live, persisted.
document.documentElement.style.setProperty("--forest-w", `${loadForestWidth()}px`);
const resizer = el("div", { class: "forest-resizer", title: "drag to resize the Forest" });
let dragging = false;
resizer.addEventListener("mousedown", (e) => {
  dragging = true;
  (e as MouseEvent).preventDefault();
  document.body.style.cursor = "col-resize";
  document.body.style.userSelect = "none";
});
window.addEventListener("mousemove", (e) => {
  if (!dragging) return;
  const w = clampForestWidth((e as MouseEvent).clientX - forest.node.getBoundingClientRect().left);
  document.documentElement.style.setProperty("--forest-w", `${w}px`);
});
window.addEventListener("mouseup", () => {
  if (!dragging) return;
  dragging = false;
  document.body.style.cursor = "";
  document.body.style.userSelect = "";
  saveForestWidth(parseInt(document.documentElement.style.getPropertyValue("--forest-w"), 10));
});

const root = el("div", { class: "wb" },
  masthead.node, clock.node,
  el("div", { class: "body" }, forest.node, resizer, center, furnace.node));

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
  // resumed session has painted and gone idle (auto-send). Arm pendingPrompt *after*
  // selectSession, which clears it — the resumed pty's paint then triggers the quiet send.
  selectSession({ uuid: newUuid, id: newUuid.slice(0, 6), name: "fork", state: "working", live: true, recency: new Date().toISOString(), branches: 0, active: true, shape: [] });
  pendingPrompt = text.trim() || null;
}

function showError(msg: string): void {
  const t = el("div", { class: "toast" }, el("span", { class: "msg", text: msg }));
  center.appendChild(t);
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.remove(), 4200);
}

function sendLeaf(text: string): void {
  relight();
  showToast({ kind: "send" });
  // Echo the prompt into the Manuscript immediately — the author should see what they asked
  // while the turn streams, not a blank page (a new session has no prior turns) until the JSONL
  // round-trips. The real transcript replaces this echo on the next render.
  manuscript.showPending(text);
  // A new session's pty may still be booting claude's TUI (no JSONL/uuid yet). Hand the first
  // prompt to the quiet-send path: it delivers once the pty idles, then beginLive. The direct
  // send (below) is for an established session whose TUI is already at the prompt.
  if (awaitingBirth) { pendingPrompt = text; schedulePendingSend(); return; }
  if (sendPrompt(text)) beginLive(); // stream the response into the Manuscript until it lands
}

// ── the live pty (the real Furnace stream) ──────────────────────────────────
// The Furnace is a dark instrument in BOTH themes (its bg is dark in paper too), so the
// terminal foreground is always light — never the page ink, which would go dark-on-dark.
const TERM_FG = PALETTES.furnace.ink;
const term = new Terminal({
  fontFamily: '"IBM Plex Mono", ui-monospace, Menlo, Consolas, monospace',
  fontSize: density === "compact" ? 11 : 12,
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
  // The term was opened while the pane was hidden; on first show, fit + refresh so the
  // renderer paints the buffer it has been parsing all along.
  if (open) queueMicrotask(() => { try { fit.fit(); sendResize(); term.refresh(0, term.rows - 1); term.focus(); } catch { /* not yet sized */ } });
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
// finished painting the resumed transcript and is idling at the prompt). A brand-new
// session's first prompt rides the same quiet-send path while its TUI boots — see sendLeaf.
let pendingPrompt: string | null = null;
let pendingTimer: number | undefined;

// True between "new session opened" and "its JSONL is born". While set, a leaf send is the
// session's FIRST prompt and must wait for the booting pty to idle, not fire immediately —
// distinct from currentUuid===null, which is also true for the startup sample (no pty).
let awaitingBirth = false;
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
let liveBaseline = 0; // completed-turn count when the turn began; swap when it grows
let liveTicker: number | undefined;
let liveQuiet: number | undefined;
let liveThrottle: number | undefined;

// a turn is "complete" in the JSONL once its turn_duration system row lands — session_json
// surfaces that as the exchange's `system` field. That, not pty-quiet, is the swap signal.
function completedCount(s: Session): number {
  return s.exchanges.filter((e) => !e.leaf && !!e.system).length;
}

function beginLive(): void {
  clearLive();
  inFlight = true;
  liveStart = Date.now();
  liveBaseline = completedCount(current);
  renderLive();
  resetQuiet();
  liveTicker = window.setInterval(renderLive, 1000);
}
function onPtyOutput(): void {
  if (!inFlight) return;
  resetQuiet();
  if (liveThrottle === undefined) {
    liveThrottle = window.setTimeout(() => { liveThrottle = undefined; renderLive(); }, 50);
  }
}
function resetQuiet(): void {
  // backstop only — the real swap is gated on the JSONL (see renderManuscript). This just
  // rescues a stuck live region if the watch/JSONL never reports the completed turn.
  window.clearTimeout(liveQuiet);
  liveQuiet = window.setTimeout(endLive, 12000);
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

// The live terminal's recent text, reconstructed into logical lines. claude's TUI (no
// alt-screen) appends the response into scrollback while constantly redrawing a bottom
// chrome block (spinner, ❯ input, rules, the host/mode status). We rejoin soft-wrapped
// continuations, drop that chrome, and dedupe consecutive lines (a mid-redraw frame can
// momentarily double a line). The result is the response growing, not a jumping window.
function termTail(maxLines = 20): string {
  const buf = term.buffer.active;
  const startY = Math.max(0, buf.length - 400);
  const logical: string[] = [];
  let cur = "";
  let started = false;
  for (let y = startY; y < buf.length; y++) {
    const line = buf.getLine(y);
    if (!line) continue;
    const text = line.translateToString(true);
    if (line.isWrapped && started) {
      cur += text; // soft-wrap continuation of the same logical line
    } else {
      if (started) logical.push(cur);
      cur = text;
      started = true;
    }
  }
  if (started) logical.push(cur);

  const out: string[] = [];
  for (const s of logical) {
    if (!s.trim() || isChrome(s)) continue;
    if (out.length && out[out.length - 1] === s) continue; // dedupe a doubled redraw frame
    out.push(s);
  }
  return out.slice(-maxLines).join("\n");
}
// Lines that are TUI chrome, not response text — observed from the raw pty stream.
function isChrome(s: string): boolean {
  const t = s.trim();
  if (!t) return true;
  if (/^[╭╮╰╯│┃─━└┘┌┐┤├┬┴┼▏▕|]+$/.test(t)) return true; // borders / rules
  if (t.includes("❯")) return true; // the input prompt
  if (/^[│|].*[│|]$/.test(t)) return true; // a bordered input-box row
  if (/\bctx:\s*\d|\bINSERT\b|auto mode|shift\+tab|\? for shortcuts|esc to interrupt|bypass permissions|⏵|for newline|to cycle/i.test(t)) return true;
  // the "cooking" footer: a verb + ellipsis + a parenthesised "(51s · 21k tokens · …)".
  // Match its distinctive shape, not a bare mention of "tokens" (the user talks about
  // tokens in prose), so real response lines survive.
  if (/^[✻✢✳✶✷✺✸✹◐◓◑◒◴◷◵◶*∗·•]\s*\S/.test(t) && t.includes("…")) return true; // "✻ Verb…"
  if (/\(\s*\d+(\.\d+)?\s*s\b[^)]*\btokens?\b/i.test(t)) return true; // "(51s · 21k tokens"
  if (/…\s*\(\s*(esc|\d)/i.test(t)) return true; // "verb… (esc" / "verb… (51s"
  if (/^[A-Za-z]+…\s*\d*$/.test(t)) return true; // "Sketching… 2"
  if (/\bfor \d+m?\s*\d*s\s*$/.test(t)) return true; // "Crunched for 1m 6s"
  return false;
}

function sendResize(): void {
  // When the Furnace is open, fit to its pane (a faithful raw view). When it's collapsed
  // (the default), claude still needs a sane width — give it a comfortable fixed one so the
  // streamed text the Manuscript tails wraps wide, not at a cramped side-panel column.
  if (furnaceIsOpen) {
    try { fit.fit(); } catch { /* hidden */ }
  } else if (term.cols !== 100) {
    term.resize(100, Math.max(term.rows || 30, 30));
  }
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
  // Viewing a real, already-born session: drop any new-session pending state so a stale first
  // prompt can't be delivered into this pty. (doFork re-arms pendingPrompt *after* this call.)
  awaitingBirth = false;
  pendingPrompt = null;
  bornBaseline = null;
  activeSession = entry.uuid;
  connectPty(`?session=${encodeURIComponent(entry.uuid)}`);
  followManuscript(entry.uuid);
  renderForest();
}

// Launch a fresh `claude` in the chosen directory. The daemon spawns it (?new=<cwd>)
// and watches the projects dir for the new session's JSONL; when it reports the uuid,
// onSessionBorn takes over (follow + refresh). Until then, the Furnace shows it boot.
function startNewSession(cwd: string): void {
  activeSession = null;
  currentUuid = null;
  if (es) { es.close(); es = null; }
  // Reset to an unwritten session: clean page + epigraph, zero live-baseline, no stale prompt.
  awaitingBirth = true;
  pendingPrompt = null;
  // Snapshot the sessions we already know, so the first uuid the Forest adds is *this* newborn.
  bornBaseline = new Set(forestEntries.map((e) => e.uuid).filter((u): u is string => !!u));
  window.clearTimeout(pendingTimer);
  current = emptySession();
  masthead.setSession(current);
  manuscript.setEmpty(); // clean page + a random epigraph until the first prompt births the JSONL
  connectPty("?new=" + encodeURIComponent(cwd));
}

function onSessionBorn(uuid: string): void {
  if (activeSession === uuid) return; // already adopted (pty message + Forest can both report it)
  awaitingBirth = false; // the JSONL exists now; subsequent leaf sends go direct
  bornBaseline = null; // stop watching the Forest for a newborn — we have it
  activeSession = uuid;
  followManuscript(uuid);
  renderForest();
}

// Forest-fallback birth: the pty `session` message is a single best-effort packet; if it's
// missed the Manuscript would hang on the epigraph forever. The live Forest snapshot is an
// independent, reliable signal — while awaiting a new session, adopt the first uuid that
// appears in the Forest but wasn't known when the session opened.
let bornBaseline: Set<string> | null = null;
function maybeAdoptNewborn(): void {
  if (!awaitingBirth || !bornBaseline) return;
  const fresh = forestEntries.find((e) => e.uuid && !bornBaseline!.has(e.uuid));
  if (fresh?.uuid) onSessionBorn(fresh.uuid);
}

// Render the chosen session into the Manuscript and follow it live: re-fetch the
// structured transcript on each SSE 'change', pinned to the leaf if we were near it.
// Opening a manuscript lands at the bottom — the leaf, ready to send — while later
// SSE re-renders keep the reader where they are (stick to bottom only if near it).
function followManuscript(uuid: string): void {
  void renderManuscript(uuid, true);
  if (es) es.close();
  es = new EventSource(`/api/watch/${encodeURIComponent(uuid)}`);
  es.onmessage = () => void renderManuscript(uuid);
}

async function renderManuscript(uuid: string, landAtBottom = false): Promise<void> {
  const s = await fetchSession(uuid);
  if (!s) return;
  const scroller = manuscript.scroller;
  const nearBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  const prev = scroller.scrollTop;
  current = s;
  currentUuid = uuid;
  manuscript.setSession(s);
  masthead.setSession(s);
  // gated swap: if the completed turn just landed in the JSONL, drop the live region in
  // this same task — the clean turn and the streaming region change in one paint.
  if (inFlight && completedCount(s) > liveBaseline) {
    inFlight = false;
    stopLiveTimers();
    manuscript.setLive(null);
  }
  scroller.scrollTop = (landAtBottom || nearBottom) ? scroller.scrollHeight : prev;
}

// The live Forest: the daemon pushes the corroborated snapshot over SSE on change.
// We keep the latest entries and re-mark which one is being viewed before rendering,
// so selecting a session highlights immediately (before the next push).
let forestEntries: ForestEntry[] = [];
function renderForest(): void {
  for (const e of forestEntries) e.active = e.uuid === activeSession;
  forest.fill(forestEntries);
}
async function initForest(): Promise<void> {
  forestEntries = await loadForest(); // paint once before the first SSE frame lands
  renderForest();
  watchForest((entries) => { forestEntries = entries; renderForest(); maybeAdoptNewborn(); });
}

// ── dev live-reload — never drops a live session ────────────────────────────
function devLiveReload(): void {
  if (!document.querySelector('meta[name="eigenform-dev"]')) return;
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
void initForest();
void loadProjectDirs();
devLiveReload();
void previewRecent();

// fill the new-session directory datalist from the daemon's recent project cwds
async function loadProjectDirs(): Promise<void> {
  try {
    const dirs = (await (await fetch("/api/projects")).json()) as string[];
    forest.setProjectDirs(dirs);
  } catch {
    /* no projects dir configured — the input still accepts a typed path */
  }
}

async function previewRecent(): Promise<void> {
  try {
    const uuid = (await (await fetch("/api/recent")).text()).trim();
    if (uuid) followManuscript(uuid); // render + follow, but DON'T resume the pty (no tokens)
  } catch {
    /* no sessions — keep the sample */
  }
}
