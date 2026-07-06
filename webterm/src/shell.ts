/**
 * shell.ts — Rail roster + top bar (tabs, global controls) + terminal host.
 *
 * Layout (eigenform warm-ink design, Claude Design handoff 2026-06-12):
 *   rail (brand · search · grouped sessions · footer)
 *   │ topbar (tabs · theme toggle · global drawer toggle)
 *   │ term-area (header breadcrumb · term-host with one Terminal per tab)
 *
 * One Terminal per open tab, kept alive while the tab exists, hidden via
 * display:none when inactive. Tab switch calls fit.fit() to correct dimensions.
 *
 * Pure helpers (relativeRecency, ageGroup, inkFor, reconcileTabs, …) live in
 * shell-helpers.ts — no xterm dependency, directly testable with node --test.
 *
 * DRAWER (global, not per-tab): the transcript drawer is toggled by a single
 * persistent control in the top-right and follows the ACTIVE tab. Open state
 * persists across reloads (LS_DRAWER). When the active tab has no session
 * uuid yet, the open drawer shows a placeholder instead of a transcript.
 *
 * THEME: a color scheme (src/themes/schemes.ts) picked from the top bar drives
 * the WHOLE surface — deriveChrome() generates every chrome token from it and
 * the same scheme colors the terminal. Persisted as a scheme id (LS_SCHEME);
 * the legacy light/dark toggle (LS_THEME) migrates to Warm Ink light/dark.
 *
 * LOCALSTORAGE SCHEMA (key "eigenform:term:tabs:v1"):
 *   JSON array of TabDescriptor. Versioned key — bump suffix if schema changes.
 *
 * POLL: rail polls GET /api/pty + GET /api/forest every 3s to update badges.
 * Interval is cleared on visibilitychange → hidden to avoid background fan-out.
 */

import { newTerminal, connectPty, applyFont, applyTermTheme, DEFAULT_FONT } from "./pty.ts";
import type { FontSettings } from "./pty.ts";
import { SCHEMES, DEFAULT_SCHEME_ID, schemeById } from "./themes/schemes.ts";
import type { Scheme } from "./themes/schemes.ts";
import { deriveChrome } from "./themes/derive.ts";
import { buildRoster, ptyActivity } from "./roster.ts";
import type { RosterRow, Liveness, Activity } from "./roster.ts";
import type { PtyInfo, ForestItem } from "./types.ts";
import {
  relativeRecency,
  reconcileTabs,
  reconnectQuery,
  reconnectDelay,
  ageGroup,
  inkFor,
  railFromPointer,
  RAIL_DEFAULT,
  drawerWidthFromPointer,
  DRAWER_DEFAULT_W,
  splitHeightFromPointer,
  REACH_DEFAULT_H,
  seedDue,
  type SeedTiming,
  type AgeGroup,
  type TabDescriptor,
  type TabReconcileAction,
} from "./shell-helpers.ts";
import { shouldAutoRecover } from "./downgrade.ts";
import { mountPicker } from "./picker.ts";
import { mountDrawer } from "./drawer.ts";
import type { DrawerHandle } from "./drawer.ts";
import { mountReachMap } from "./reachmap.ts";
import type { ReachHandle } from "./reachmap.ts";
import { mountEvents } from "./events.ts";
import type { EventsHandle } from "./events.ts";
import { createForestPreview } from "./forest-preview.ts";
import type { ForestPreviewHandle } from "./forest-preview.ts";
import { icon } from "./icons.ts";
import { openInspect } from "./inspect.ts";

// Re-export so callers can reach pure helpers via either module.
export { relativeRecency, reconcileTabs };
export type { TabDescriptor, TabReconcileAction };

// ---------------------------------------------------------------------------
// Internal constants
// ---------------------------------------------------------------------------

const LS_KEY = "eigenform:term:tabs:v1";
const LS_OVERRIDES = "eigenform:term:overrides:v1";
const LS_THEME = "eigenform:term:theme:v1"; // legacy light/dark — migrated to v2
const LS_SCHEME = "eigenform:term:theme:v2"; // scheme id

/** Resolve the active scheme: stored v2 id → migrated v1 light/dark → default. */
function loadSchemeId(): string {
  const v2 = localStorage.getItem(LS_SCHEME);
  if (v2 && schemeById(v2)) return v2;
  const v1 = localStorage.getItem(LS_THEME);
  if (v1 === "light") return "warm-ink-light";
  if (v1 === "dark") return "warm-ink-dark";
  return DEFAULT_SCHEME_ID;
}
const LS_DRAWER = "eigenform:term:drawer:v1";
const LS_DOCK_W = "eigenform:term:drawer-w:v1";
const LS_REACH_H = "eigenform:term:reach-h:v1";
const LS_RAIL = "eigenform:term:rail:v1";
const LS_FONT = "eigenform:term:font:v1";

/** Terminal typefaces offered in the font popover. macOS-first: "System Mono"
 *  resolves to SF Mono / Menlo with no webfont round-trip. */
const TERM_FACES: { label: string; stack: string }[] = [
  { label: "Plex Mono", stack: DEFAULT_FONT.family },
  {
    label: "System Mono",
    stack: 'ui-monospace, "SF Mono", Menlo, "Cascadia Mono", Consolas, monospace',
  },
];

/** Bounds for the font controls (and ⌘ +/− zoom). */
const FONT_BOUNDS = {
  size: { min: 9, max: 24, step: 0.5 },
  lineHeight: { min: 1.0, max: 2.0, step: 0.05 },
  letterSpacing: { min: -2, max: 4, step: 0.5 },
};

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Validate one persisted numeric field, falling back to a default. */
function numField(v: unknown, fallback: number, lo: number, hi: number): number {
  return typeof v === "number" && Number.isFinite(v) ? clamp(v, lo, hi) : fallback;
}

function loadFont(): FontSettings {
  try {
    const raw = JSON.parse(localStorage.getItem(LS_FONT) ?? "{}") as Partial<FontSettings>;
    return {
      family: typeof raw.family === "string" && raw.family ? raw.family : DEFAULT_FONT.family,
      size: numField(raw.size, DEFAULT_FONT.size, FONT_BOUNDS.size.min, FONT_BOUNDS.size.max),
      lineHeight: numField(raw.lineHeight, DEFAULT_FONT.lineHeight, FONT_BOUNDS.lineHeight.min, FONT_BOUNDS.lineHeight.max),
      letterSpacing: numField(raw.letterSpacing, DEFAULT_FONT.letterSpacing, FONT_BOUNDS.letterSpacing.min, FONT_BOUNDS.letterSpacing.max),
    };
  } catch {
    return { ...DEFAULT_FONT };
  }
}

const KNOWN_ACTIVITY = new Set(["working", "waiting", "idle"]);

/**
 * CSS classes for a status dot across the two orthogonal channels:
 *   activity → color + glow (`dot--working|waiting|idle`)
 *   liveness → fill        (`dot--eigenform|external|dead`)
 * Only live provenances (eigenform/external) animate; dead never glows.
 */
function dotClasses(activity: string, liveness: Liveness): string {
  const act = KNOWN_ACTIVITY.has(activity) ? activity : "idle";
  const prov = liveness === "eigenform" ? "eigenform" : liveness === "external" ? "external" : "dead";
  return `dot dot--${act} dot--${prov}`;
}

/** Short turn-state tag for a live row's meta line; null for dead rows.
 *  External (live outside eigenform) rows are prefixed so provenance reads at a
 *  glance without hovering, complementing the hollow-ring dot. */
function livenessTag(activity: Activity, liveness: Liveness): string | null {
  if (liveness === "none") return null;
  const turn = activity === "working" ? "running" : activity === "waiting" ? "your turn" : "live";
  return liveness === "external" ? `· ext · ${turn}` : `· ${turn}`;
}

/** Full hover explanation of a dot's combined state. */
function dotTitle(activity: Activity, liveness: Liveness): string {
  const where =
    liveness === "eigenform"
      ? "eigenform session"
      : liveness === "external"
        ? "running outside eigenform — can't attach"
        : "no live process";
  if (liveness === "none") return where;
  const turn =
    activity === "working"
      ? "assistant running"
      : activity === "waiting"
        ? "waiting for your input"
        : "idle at prompt";
  return `${turn} — ${where}`;
}

/** The session's ink hue CSS value, from its most durable key. */
// Color = project: hash on the full cwd path so every session in the same
// directory shares one hue (in both the rail and the tab strip). Falls back to
// a label/chip when the cwd is unknown. Hashing the full path (not the basename)
// keeps unrelated `…/src` dirs from colliding.
function inkVar(cwd: string | undefined, fallback: string): string {
  return `var(--ink-${inkFor(cwd ?? fallback)})`;
}

const GROUP_LABELS: Record<AgeGroup, string> = {
  today: "Today",
  week: "This week",
  earlier: "Earlier",
};
const GROUP_ORDER: AgeGroup[] = ["today", "week", "earlier"];

// ---------------------------------------------------------------------------
// Tab registry type
// ---------------------------------------------------------------------------

interface TabEntry {
  /** Stable per-tab id: ptyId once known, else uuid, else ephemeral timestamp. */
  id: string;
  descriptor: TabDescriptor;
  termEl: HTMLDivElement;
  handle: ReturnType<typeof newTerminal>;
  ptyHandle: ReturnType<typeof connectPty> | null;
  state: string;
  /** true when the pty exited or an attach-miss closed the socket. */
  dead: boolean;
  /** true once the tab is intentionally closed — suppresses reconnect. */
  disposed: boolean;
  /** true while a reconnect loop is in flight (socket dropped, retrying). */
  reconnecting: boolean;
  /** Consecutive reconnect attempts so far — drives the backoff. */
  reconnectAttempt: number;
  /** Pending reconnect timer id, or null. */
  reconnectTimer: number | null;
}

// ---------------------------------------------------------------------------
// mountShell — entry point called by main.ts
// ---------------------------------------------------------------------------

/**
 * Build and mount the shell UI into the given element (#app).
 *
 * Performs initial fetch + roster render, restores persisted tabs, starts the
 * 3-second poll. Returns nothing — the shell owns the DOM from here on.
 *
 * Single-call contract: registers a document-level visibilitychange listener
 * and owns the #app element for the page's lifetime. There is no teardown
 * path — must be called exactly once per page load.
 */
export function mountShell(appEl: HTMLElement): void {
  // ------------------------------------------------------------------
  // Theme (applied before any layout so first paint is correct)
  // ------------------------------------------------------------------
  let scheme: Scheme = schemeById(loadSchemeId()) ?? SCHEMES[0]!;
  applyChrome(scheme);

  /** Paint the whole surface from a scheme: chrome tokens on :root + every
   *  open terminal's colors. Persisted so it survives reload. */
  function applyScheme(next: Scheme) {
    scheme = next;
    localStorage.setItem(LS_SCHEME, next.id);
    applyChrome(next);
    for (const t of tabs) applyTermTheme(t.handle.term, next.theme);
    renderControls();
    if (themePopover) renderThemePopover();
  }

  function applyChrome(s: Scheme) {
    const root = document.documentElement;
    for (const [k, v] of Object.entries(deriveChrome(s.theme))) {
      root.style.setProperty(k, v);
    }
    root.style.colorScheme = s.dark ? "dark" : "light";
  }

  // ------------------------------------------------------------------
  // Terminal typography (persisted; applied live to every open terminal)
  // ------------------------------------------------------------------
  let font = loadFont();

  /** Merge a patch into the live font settings, persist, and re-lay every grid. */
  function setFont(patch: Partial<FontSettings>) {
    font = { ...font, ...patch };
    localStorage.setItem(LS_FONT, JSON.stringify(font));
    for (const t of tabs) {
      applyFont(t.handle.term, font);
      // Re-measured cell → recompute cols/rows; onResize relays it to the daemon.
      try { t.handle.fit.fit(); } catch { /* zero-size element — ok */ }
    }
  }

  function bumpFontSize(delta: number) {
    const b = FONT_BOUNDS.size;
    setFont({ size: clamp(Math.round((font.size + delta) * 4) / 4, b.min, b.max) });
    if (fontPopover) renderFontPopover();
  }

  // ⌘/Ctrl +/−/0 — terminal zoom (overrides browser page zoom, which is the
  // wrong granularity for a grid we own).
  window.addEventListener("keydown", (e) => {
    if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
    if (e.key === "=" || e.key === "+") { e.preventDefault(); bumpFontSize(0.5); }
    else if (e.key === "-" || e.key === "_") { e.preventDefault(); bumpFontSize(-0.5); }
    else if (e.key === "0") { e.preventDefault(); setFont({ size: DEFAULT_FONT.size }); if (fontPopover) renderFontPopover(); }
  });

  // ------------------------------------------------------------------
  // Skeleton DOM
  // ------------------------------------------------------------------
  appEl.innerHTML = "";
  appEl.className = "shell";

  // rail
  const rail = el("aside", "rail");
  const railBrand = el("div", "rail-brand");
  const brandWord = el("span", "brand-word");
  brandWord.append(icon("mark", 20, 2.2));
  const brandName = el("span", "brand-name");
  brandName.textContent = "eigenform";
  brandWord.append(brandName);
  const newBtn = el("button", "icon-btn icon-btn--boxed");
  newBtn.title = "New session";
  newBtn.append(icon("plus", 15));
  railBrand.append(brandWord, newBtn);

  const railSearch = el("div", "rail-search");
  const searchBox = el("div", "rail-search-box");
  searchBox.append(icon("search", 13));
  const searchInput = el("input", "rail-search-input");
  searchInput.placeholder = "Search sessions";
  searchBox.append(searchInput);
  railSearch.append(searchBox);

  const railScroll = el("div", "rail-scroll scroll");
  const railFoot = el("div", "rail-foot");
  rail.append(railBrand, railSearch, railScroll, railFoot);

  // main column
  const main = el("div", "main");
  const topbar = el("div", "topbar");
  const railBtn = el("button", "icon-btn topbar-rail-btn");
  railBtn.title = "Show sessions";
  const railBtnIcon = icon("panel", 16);
  railBtnIcon.style.transform = "scaleX(-1)"; // left-panel reading of the icon
  railBtn.append(railBtnIcon);
  const tabStrip = el("div", "tab-strip");
  const controls = el("div", "topbar-controls");
  topbar.append(railBtn, tabStrip, controls);

  const termArea = el("div", "term-area");
  const termHeader = el("div", "term-header");
  const termHost = el("div", "term-host");
  // termStack holds the stacked term panes; the dock sits beside it (flex row)
  // and pushes it narrower when open, rather than floating over it.
  const termStack = el("div", "term-stack");
  const dockResizer = el("div", "drawer-resizer");
  dockResizer.title = "drag to resize the inspect panel";
  const drawerDock = el("div", "drawer-dock");
  const reachRegion = el("div", "reach-region");
  const dockVsplit = el("div", "dock-vsplit");
  dockVsplit.title = "drag to resize the reach map / transcript split";
  const transcriptRegion = el("div", "transcript-region");
  // Events pane: a collapsible accordion at the dock's foot. Unlike the reach map
  // + transcript (which are uuid-bound), it's global, so it has no vertical split —
  // it self-collapses to just its header when folded.
  const eventsRegion = el("div", "events-region");
  drawerDock.append(reachRegion, dockVsplit, transcriptRegion, eventsRegion);
  termHost.append(termStack, dockResizer, drawerDock);
  termArea.append(termHeader, termHost);

  main.append(topbar, termArea);
  const resizer = el("div", "rail-resizer");
  resizer.title = "drag to resize the rail · drag far left to hide";
  appEl.append(rail, resizer, main);

  // ------------------------------------------------------------------
  // Rail resize / collapse — woland's splitter pattern: drag sets --rail-w
  // live, state persists on mouseup. Dragging left past the collapse
  // threshold hides the rail; the topbar button (or dragging back right)
  // restores it at its previous width.
  // ------------------------------------------------------------------

  let railW = RAIL_DEFAULT;
  let railCollapsed = false;
  try {
    const saved = JSON.parse(localStorage.getItem(LS_RAIL) ?? "{}") as {
      w?: number;
      collapsed?: boolean;
    };
    // Re-clamp through the drag mapper so a stale/garbage width can't stick.
    if (typeof saved.w === "number") railW = railFromPointer(saved.w, RAIL_DEFAULT).w;
    railCollapsed = saved.collapsed === true;
  } catch {
    // Corrupt entry — keep defaults.
  }

  function applyRail() {
    document.documentElement.style.setProperty("--rail-w", `${railW}px`);
    appEl.classList.toggle("shell--rail-collapsed", railCollapsed);
  }
  applyRail();

  function saveRail() {
    localStorage.setItem(LS_RAIL, JSON.stringify({ w: railW, collapsed: railCollapsed }));
  }

  /** Re-fit the active tab's xterm after the terminal pane changes width. */
  function fitActive() {
    const t = activeTab();
    if (!t) return;
    requestAnimationFrame(() => {
      try { t.handle.fit.fit(); } catch { /* zero-size element — ok */ }
    });
  }

  let railDragging = false;
  resizer.addEventListener("mousedown", (e) => {
    railDragging = true;
    e.preventDefault();
    resizer.classList.add("rail-resizer--dragging");
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  });
  window.addEventListener("mousemove", (e) => {
    if (!railDragging) return;
    const x = e.clientX - appEl.getBoundingClientRect().left;
    const next = railFromPointer(x, railW);
    railW = next.w;
    railCollapsed = next.collapsed;
    applyRail();
  });
  window.addEventListener("mouseup", () => {
    if (!railDragging) return;
    railDragging = false;
    resizer.classList.remove("rail-resizer--dragging");
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    saveRail();
    // One clean resize at drag end (not per-mousemove — a resize storm would
    // SIGWINCH the pty on every pixel; spike 09's repaint handles the single one).
    fitActive();
  });

  railBtn.addEventListener("click", () => {
    railCollapsed = false;
    applyRail();
    saveRail();
    fitActive();
  });

  // ------------------------------------------------------------------
  // Tab registry
  // ------------------------------------------------------------------
  const tabs: TabEntry[] = [];
  let activeTabId: string | null = null;

  // Auto-recover state (see downgrade.ts + the forest poll below).
  // lastInputAt: epoch ms of the user's last keystroke into the ACTIVE tab —
  //   guards against yanking focus / eating a keypress mid-sentence.
  // recovered: source session uuids we've already auto-staged a retry for
  //   (fires at most once per session, even across polls).
  // downgradedUuids: uuids the latest forest poll flagged as downgraded — drives
  //   the topbar recover button's "attention" state for the active session.
  let lastInputAt = 0;
  const recovered = new Set<string>();
  const downgradedUuids = new Set<string>();

  function activeTab(): TabEntry | null {
    return tabs.find((t) => t.id === activeTabId) ?? null;
  }

  function saveTabs() {
    // Strip seedInput: a staged prompt must never survive a reload (it would be
    // re-injected into a resumed pty on boot). It lives only in memory.
    const descriptors: TabDescriptor[] = tabs.map(({ descriptor }) => {
      const { seedInput: _seedInput, ...persisted } = descriptor;
      return persisted;
    });
    localStorage.setItem(LS_KEY, JSON.stringify(descriptors));
  }

  function activateTab(id: string) {
    for (const t of tabs) {
      const isActive = t.id === id;
      t.termEl.style.display = isActive ? "" : "none";
      if (isActive) {
        activeTabId = id;
        // Re-fit on becoming visible so xterm dimensions are correct.
        requestAnimationFrame(() => {
          try { t.handle.fit.fit(); } catch { /* zero-size element — ok */ }
        });
      }
    }
    renderTabStrip();
    renderTermHeader();
    syncDock();
    renderRail();
  }

  function closeTab(id: string) {
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    const t = tabs[idx]!;

    // Mark disposed first so a reconnect-in-flight (or the socket's own onClose)
    // doesn't resurrect the tab we're tearing down.
    t.disposed = true;
    clearReconnect(t);

    // Detach socket — pty stays alive in the daemon.
    t.ptyHandle?.dispose();
    t.ptyHandle = null;

    // Dispose xterm Terminal to free canvas + worker resources.
    t.handle.term.dispose();
    t.termEl.remove();
    tabs.splice(idx, 1);
    saveTabs();

    if (activeTabId === id) {
      activeTabId = null;
      const next = tabs[Math.min(idx, tabs.length - 1)];
      if (next) {
        activateTab(next.id);
        return;
      }
    }
    renderTabStrip();
    renderTermHeader();
    syncDock();
    renderRail();
  }

  async function killTab(id: string) {
    const t = tabs.find((t) => t.id === id);
    if (!t) return;
    const ptyId = t.descriptor.ptyId;
    if (!ptyId) {
      closeTab(id);
      return;
    }
    if (!confirm(`Kill pty ${ptyId}? (The child process will be terminated.)`)) return;
    try {
      await fetch(`/api/pty/${ptyId}`, { method: "DELETE" });
    } catch {
      // Best-effort.
    }
    closeTab(id);
    void refreshRoster();
  }

  // ------------------------------------------------------------------
  // Inspect dock — GLOBAL toggle (persistent control top-right), follows the
  // active tab. A right-docked panel that pushes the terminal narrower (not a
  // floating overlay): reach map on top, transcript below, split by a draggable
  // divider. Both halves follow the active tab's session uuid.
  // ------------------------------------------------------------------

  let drawerOpen = localStorage.getItem(LS_DRAWER) === "1";
  /** Mounted transcript drawer (uuid-bound), or null. */
  let drawerCurrent: { uuid: string; handle: DrawerHandle } | null = null;
  /** Mounted reach map (uuid-bound), or null. */
  let reachCurrent: { uuid: string; handle: ReachHandle } | null = null;
  /** Mounted events pane (global — not uuid-bound), or null. Lives while the dock
   *  is open, independent of the active tab. */
  let eventsCurrent: EventsHandle | null = null;
  /** Placeholder shown when the dock is open but the active tab has no uuid. */
  let dockPlaceholder: HTMLElement | null = null;

  function readNum(key: string, fallback: number): number {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && v > 0 ? v : fallback;
  }

  // Persisted dock geometry: dock width + reach-region height. Re-clamped on
  // read so a stale/garbage value can't wedge the layout.
  let dockW = drawerWidthFromPointer(0, readNum(LS_DOCK_W, DRAWER_DEFAULT_W));
  let reachH = readNum(LS_REACH_H, REACH_DEFAULT_H);

  function applyDockGeometry() {
    document.documentElement.style.setProperty("--drawer-w", `${dockW}px`);
    document.documentElement.style.setProperty("--reach-h", `${reachH}px`);
  }
  applyDockGeometry();

  function saveDockGeometry() {
    localStorage.setItem(LS_DOCK_W, String(dockW));
    localStorage.setItem(LS_REACH_H, String(reachH));
  }

  function setDrawerOpen(open: boolean) {
    drawerOpen = open;
    localStorage.setItem(LS_DRAWER, open ? "1" : "0");
    syncDock();
    // The dock's width changed → the terminal column resized; re-fit once.
    fitActive();
  }

  /** Reconcile the mounted dock (reach + transcript) against (drawerOpen, tab). */
  function syncDock() {
    const open = drawerOpen && tabs.length > 0;
    drawerDock.style.display = open ? "flex" : "none";
    dockResizer.style.display = open ? "" : "none";

    if (!open) {
      reachCurrent?.handle.close();
      reachCurrent = null;
      drawerCurrent?.handle.close();
      drawerCurrent = null;
      eventsCurrent?.close();
      eventsCurrent = null;
      renderControls();
      return;
    }

    // The events pane is global (not uuid-bound) — mount it once while the dock is
    // open, before the per-uuid reach/transcript wiring below.
    if (!eventsCurrent) eventsCurrent = mountEvents(eventsRegion);

    const uuid = activeTab()?.descriptor.uuid ?? null;

    if (uuid) {
      if (dockPlaceholder) {
        dockPlaceholder.remove();
        dockPlaceholder = null;
      }
      reachRegion.style.display = "";
      dockVsplit.style.display = "";

      if (reachCurrent?.uuid !== uuid) {
        reachCurrent?.handle.close();
        // No onClose → the reach map renders without a close button and won't
        // grab Esc; it lives in the dock for as long as the dock is open.
        reachCurrent = {
          uuid,
          handle: mountReachMap(reachRegion, uuid, {
            root: activeTab()?.descriptor.cwd ?? undefined,
          }),
        };
      }
      if (drawerCurrent?.uuid !== uuid) {
        drawerCurrent?.handle.close();
        // onFork: open the forked session as a new tab + refresh the roster so
        // the rail shows it immediately (copy-on-fork — source tab stays open).
        // seedInput stages the edited prompt into the resumed branch, unsent —
        // the daemon never writes it to the branch file.
        drawerCurrent = {
          uuid,
          handle: mountDrawer(
            transcriptRegion,
            uuid,
            (newUuid, text) => {
              openTabWithQuery(`?session=${encodeURIComponent(newUuid)}`, {
                uuid: newUuid,
                label: "fork",
                seedInput: text,
              });
              void refreshRoster();
            },
            // interrupt routes ^C to the ACTIVE tab's socket — the dock is only
            // ever mounted for the active tab's uuid (syncDock invariant).
            { interrupt: () => activeTab()?.ptyHandle?.sendInput("\x03") },
          ),
        };
      }
    } else {
      // No transcript yet — collapse the split to a single placeholder.
      reachCurrent?.handle.close();
      reachCurrent = null;
      drawerCurrent?.handle.close();
      drawerCurrent = null;
      reachRegion.style.display = "none";
      dockVsplit.style.display = "none";
      if (!dockPlaceholder) {
        dockPlaceholder = el("div", "drawer");
        const head = el("div", "drawer-header");
        const title = el("span", "drawer-title");
        title.textContent = "Transcript";
        head.append(title);
        const empty = el("div", "drawer-empty");
        empty.textContent = "no transcript yet — waiting for a session uuid";
        dockPlaceholder.append(head, empty);
        transcriptRegion.append(dockPlaceholder);
      }
    }
    renderControls();
  }

  // ── Dock splitters ─────────────────────────────────────────────────────────
  // Width (the dock's left edge) and the reach/transcript vertical split. Both
  // mirror the rail resizer: drag updates the CSS var live; state persists on
  // mouseup. The width drag re-fits the terminal once at the end — a per-pixel
  // resize would SIGWINCH the pty on every move (spike 09's repaint handles one).
  function makeDragHandle(
    handle: HTMLElement,
    cls: string,
    cursor: string,
    onMove: (e: MouseEvent) => void,
    onEnd: () => void,
  ) {
    let dragging = false;
    handle.addEventListener("mousedown", (e) => {
      dragging = true;
      e.preventDefault();
      handle.classList.add(cls);
      document.body.style.cursor = cursor;
      document.body.style.userSelect = "none";
    });
    window.addEventListener("mousemove", (e) => {
      if (!dragging) return;
      onMove(e);
    });
    window.addEventListener("mouseup", () => {
      if (!dragging) return;
      dragging = false;
      handle.classList.remove(cls);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      onEnd();
    });
  }

  makeDragHandle(
    dockResizer,
    "drawer-resizer--dragging",
    "col-resize",
    (e) => {
      dockW = drawerWidthFromPointer(e.clientX, termHost.getBoundingClientRect().right);
      applyDockGeometry();
    },
    () => {
      saveDockGeometry();
      fitActive();
    },
  );

  makeDragHandle(
    dockVsplit,
    "dock-vsplit--dragging",
    "row-resize",
    (e) => {
      const r = drawerDock.getBoundingClientRect();
      reachH = splitHeightFromPointer(e.clientY, r.top, r.height);
      applyDockGeometry();
    },
    saveDockGeometry,
  );

  // ------------------------------------------------------------------
  // Top bar: global controls (theme · reach · drawer)
  // ------------------------------------------------------------------

  function renderControls() {
    controls.innerHTML = "";

    const themeBtn = el("button", `icon-btn theme-btn${themePopover ? " icon-btn--active" : ""}`);
    themeBtn.title = `Theme — ${scheme.name}`;
    themeBtn.append(icon("palette", 16));
    themeBtn.addEventListener("click", () => toggleThemePopover(themeBtn));

    const fontBtn = el("button", `icon-btn font-btn${fontPopover ? " icon-btn--active" : ""}`);
    fontBtn.title = "Terminal font";
    fontBtn.append(icon("type", 16));
    fontBtn.addEventListener("click", () => toggleFontPopover(fontBtn));

    // Config inventory — skills + memory across resolution layers, token-budgeted.
    // Scoped to the active tab's cwd when one is known, else machine-wide.
    const configBtn = el("button", "icon-btn config-btn");
    configBtn.title = "Config inventory (skills · memory)";
    configBtn.append(icon("sliders", 16));
    configBtn.addEventListener("click", () =>
      openInspect({ cwd: activeTab()?.descriptor.cwd ?? undefined }),
    );

    const sep = el("div", "topbar-sep");

    // Manual Fable→Opus recovery — fork the active session's rewound branch and
    // stage the retry now (the same mechanism the forest poll auto-fires), with the
    // outcome recorded in the Events pane. Disabled until the active tab has a uuid;
    // highlighted (amber "attention") when that session is currently downgraded.
    const activeUuid = activeTab()?.descriptor.uuid ?? null;
    const activeDowngraded = activeUuid !== null && downgradedUuids.has(activeUuid);
    const recoverBtn = el(
      "button",
      `icon-btn recover-btn${activeDowngraded ? " icon-btn--attention" : ""}`,
    );
    recoverBtn.disabled = activeUuid === null;
    recoverBtn.title = activeDowngraded
      ? "Fable downgrade detected — fork & stage a retry (result in Events)"
      : "Trigger Fable→Opus recovery for this session (result in Events)";
    recoverBtn.append(icon("fork", 16));
    recoverBtn.addEventListener("click", () => void triggerRecover());

    // Single "inspect" toggle — opens the docked panel (reach map + transcript).
    const drawerBtn = el("button", `icon-btn${drawerOpen ? " icon-btn--active" : ""}`);
    drawerBtn.title = drawerOpen ? "Hide inspect panel" : "Show inspect panel";
    drawerBtn.append(icon("panel", 16));
    drawerBtn.addEventListener("click", () => setDrawerOpen(!drawerOpen));

    controls.append(themeBtn, fontBtn, configBtn, sep, recoverBtn, drawerBtn);
  }

  // ------------------------------------------------------------------
  // Font popover — typeface + size + line-height + letter-spacing
  // ------------------------------------------------------------------

  let fontPopover: HTMLElement | null = null;

  function toggleFontPopover(anchor: HTMLElement) {
    if (fontPopover) { closeFontPopover(); return; }
    const pop = el("div", "font-pop");
    document.body.append(pop);
    const r = anchor.getBoundingClientRect();
    pop.style.top = `${Math.round(r.bottom + 6)}px`;
    pop.style.right = `${Math.round(window.innerWidth - r.right)}px`;
    fontPopover = pop;
    renderFontPopover();
    renderControls();
    // Capture-phase so a click on a tab/terminal closes us before it acts.
    document.addEventListener("pointerdown", onFontOutside, true);
    document.addEventListener("keydown", onFontKey, true);
  }

  function closeFontPopover() {
    fontPopover?.remove();
    fontPopover = null;
    document.removeEventListener("pointerdown", onFontOutside, true);
    document.removeEventListener("keydown", onFontKey, true);
    renderControls();
  }

  function onFontOutside(e: PointerEvent) {
    const t = e.target as HTMLElement;
    if (fontPopover && !fontPopover.contains(t) && !t.closest(".font-btn")) {
      closeFontPopover();
    }
  }

  function onFontKey(e: KeyboardEvent) {
    if (e.key === "Escape") { e.preventDefault(); closeFontPopover(); }
  }

  /** Rebuild the popover body from current `font` (cheap; values are few). */
  function renderFontPopover() {
    const pop = fontPopover;
    if (!pop) return;
    pop.innerHTML = "";

    // Typeface — a row of pills; the active stack is highlighted.
    const faceRow = el("div", "font-faces");
    for (const f of TERM_FACES) {
      const pill = el("button", `font-face${font.family === f.stack ? " font-face--on" : ""}`);
      pill.textContent = f.label;
      pill.style.fontFamily = f.stack;
      pill.addEventListener("click", () => { setFont({ family: f.stack }); renderFontPopover(); });
      faceRow.append(pill);
    }
    pop.append(faceRow);

    const stepper = (
      label: string,
      value: number,
      fmt: (n: number) => string,
      key: keyof FontSettings,
      bounds: { min: number; max: number; step: number },
    ) => {
      const row = el("div", "font-pop-row");
      const name = el("span", "font-pop-label");
      name.textContent = label;
      const ctl = el("div", "font-step");
      const dec = el("button");
      dec.textContent = "−";
      const val = el("span", "font-step-val");
      val.textContent = fmt(value);
      const inc = el("button");
      inc.textContent = "+";
      const set = (n: number) => {
        setFont({ [key]: clamp(Math.round(n / bounds.step) * bounds.step, bounds.min, bounds.max) } as Partial<FontSettings>);
        renderFontPopover();
      };
      dec.addEventListener("click", () => set(value - bounds.step));
      inc.addEventListener("click", () => set(value + bounds.step));
      ctl.append(dec, val, inc);
      row.append(name, ctl);
      return row;
    };

    pop.append(
      stepper("Size", font.size, (n) => `${n}px`, "size", FONT_BOUNDS.size),
      stepper("Line height", font.lineHeight, (n) => n.toFixed(2), "lineHeight", FONT_BOUNDS.lineHeight),
      stepper("Tracking", font.letterSpacing, (n) => `${n}px`, "letterSpacing", FONT_BOUNDS.letterSpacing),
    );

    const reset = el("button", "font-pop-reset");
    reset.textContent = "Reset to defaults";
    reset.addEventListener("click", () => { setFont({ ...DEFAULT_FONT }); renderFontPopover(); });
    pop.append(reset);
  }

  // ------------------------------------------------------------------
  // Theme popover — pick a scheme by sight (live swatch strips)
  // ------------------------------------------------------------------

  let themePopover: HTMLElement | null = null;

  function toggleThemePopover(anchor: HTMLElement) {
    if (themePopover) { closeThemePopover(); return; }
    const pop = el("div", "theme-pop");
    document.body.append(pop);
    const r = anchor.getBoundingClientRect();
    pop.style.top = `${Math.round(r.bottom + 6)}px`;
    pop.style.right = `${Math.round(window.innerWidth - r.right)}px`;
    themePopover = pop;
    renderThemePopover();
    renderControls();
    document.addEventListener("pointerdown", onThemeOutside, true);
    document.addEventListener("keydown", onThemeKey, true);
  }

  function closeThemePopover() {
    themePopover?.remove();
    themePopover = null;
    document.removeEventListener("pointerdown", onThemeOutside, true);
    document.removeEventListener("keydown", onThemeKey, true);
    renderControls();
  }

  function onThemeOutside(e: PointerEvent) {
    const t = e.target as HTMLElement;
    if (themePopover && !themePopover.contains(t) && !t.closest(".theme-btn")) {
      closeThemePopover();
    }
  }

  function onThemeKey(e: KeyboardEvent) {
    if (e.key === "Escape") { e.preventDefault(); closeThemePopover(); }
  }

  // The 6 ANSI hues shown in a swatch strip — a quick read of a scheme's palette.
  const SWATCH_KEYS = ["red", "yellow", "green", "cyan", "blue", "magenta"] as const;

  function renderThemePopover() {
    const pop = themePopover;
    if (!pop) return;
    pop.innerHTML = "";
    for (const s of SCHEMES) {
      const row = el("button", `theme-row${s.id === scheme.id ? " theme-row--on" : ""}`);

      const swatch = el("div", "theme-swatch");
      swatch.style.background = s.theme.background;
      for (const k of SWATCH_KEYS) {
        const dot = el("span", "theme-dot");
        dot.style.background = s.theme[k];
        swatch.append(dot);
      }
      const fg = el("span", "theme-dot theme-dot--fg");
      fg.style.background = s.theme.foreground;
      swatch.append(fg);

      const name = el("span", "theme-row-name");
      name.textContent = s.name;

      row.append(swatch, name);
      row.addEventListener("click", () => applyScheme(s));
      pop.append(row);
    }
  }

  // ------------------------------------------------------------------
  // Tab strip
  // ------------------------------------------------------------------

  // The tab strip is fully rebuilt on every renderTabStrip call; appearance is
  // derived from the model (TabEntry), so rebuilds carry no stale-DOM risk.
  function renderTabStrip() {
    tabStrip.innerHTML = "";
    for (const t of tabs) {
      const tab = el("div", "tab");
      if (t.id === activeTabId) {
        tab.classList.add("tab--active");
        tab.style.setProperty(
          "--tab-ink",
          inkVar(t.descriptor.cwd, t.descriptor.label),
        );
      }
      if (t.dead) tab.classList.add("tab--dead");

      // Tabs are always eigenform-spawned (you can only open a tab on an
      // attachable pty); a dead/exited pty has no live process.
      const tabLiveness: Liveness = t.dead || t.state === "exited" ? "none" : "eigenform";
      const tabActivity = ptyActivity(t.state);
      const badge = el("span", dotClasses(tabActivity, tabLiveness));
      badge.title = dotTitle(tabActivity, tabLiveness);

      const labelEl = el("span", "tab-label");
      labelEl.textContent = t.descriptor.label;

      const kill = el("button", "tab-kill");
      kill.title = "Kill pty (process terminated)";
      kill.append(icon("stop", 11, 2));
      kill.addEventListener("click", (e) => {
        e.stopPropagation();
        void killTab(t.id);
      });

      const close = el("button", "tab-close");
      close.title = "Detach — close tab, pty stays alive";
      close.append(icon("x", 11, 2));
      close.addEventListener("click", (e) => {
        e.stopPropagation();
        closeTab(t.id);
      });

      tab.append(badge, labelEl, kill, close);
      tab.addEventListener("click", () => activateTab(t.id));
      tabStrip.append(tab);
    }

    const plusBtn = el("button", "tab-new");
    plusBtn.title = "Open new session (fuzzy launcher)";
    plusBtn.append(icon("plus", 16));
    plusBtn.addEventListener("click", () => openPicker(plusBtn));
    tabStrip.append(plusBtn);
  }

  // ------------------------------------------------------------------
  // Terminal header — breadcrumb (cwd) + state chip for the active tab
  // ------------------------------------------------------------------

  function renderTermHeader() {
    termHeader.innerHTML = "";
    const t = activeTab();
    if (!t) {
      const crumb = el("span", "term-crumb");
      crumb.textContent = "no open session";
      termHeader.append(crumb);
      return;
    }

    const crumb = el("span", "term-crumb");
    const cwd = t.descriptor.cwd;
    if (cwd) {
      const trimmed = cwd.replace(/\/+$/, "");
      const slash = trimmed.lastIndexOf("/");
      crumb.append(trimmed.slice(0, slash + 1));
      const base = document.createElement("b");
      base.textContent = trimmed.slice(slash + 1);
      crumb.append(base);
    } else {
      const base = document.createElement("b");
      base.textContent = t.descriptor.label;
      crumb.append(base);
    }

    const right = el("span", "term-header-right");
    const stateChip = el("span", "chip");
    stateChip.textContent = t.dead ? "exited" : t.state;
    right.append(stateChip);

    termHeader.append(crumb, right);
  }

  // ------------------------------------------------------------------
  // Open tab helpers
  // ------------------------------------------------------------------

  function openTabWithQuery(query: string, desc: TabDescriptor): TabEntry {
    const tabId = desc.ptyId ?? desc.uuid ?? `ephemeral-${Date.now()}`;

    // Reuse existing tab if already open.
    const existing = tabs.find((t) => t.id === tabId);
    if (existing) {
      activateTab(tabId);
      return existing;
    }

    const termEl = el("div", "term-pane");
    termStack.append(termEl);

    const handle = newTerminal(font, scheme.theme);
    handle.term.open(termEl);

    const entry: TabEntry = {
      id: tabId,
      descriptor: desc,
      termEl,
      handle,
      ptyHandle: null,
      state: "idle",
      dead: false,
      disposed: false,
      reconnecting: false,
      reconnectAttempt: 0,
      reconnectTimer: null,
    };

    // Track the user's last keystroke into the ACTIVE tab so the auto-recover
    // gate never fires mid-sentence (see the forest poll's shouldAutoRecover).
    // Compares entry.id (mutated in place by onPtyId) so it survives id renumber.
    handle.term.onData(() => {
      if (entry.id === activeTabId) lastInputAt = Date.now();
    });

    connectEntry(entry, query, desc.ptyId);
    tabs.push(entry);
    saveTabs();
    activateTab(entry.id);

    requestAnimationFrame(() => {
      try { handle.fit.fit(); } catch { /* ok */ }
    });

    return entry;
  }

  /**
   * Open (or re-open) the pty socket for `entry` with `query` and wire the
   * protocol handlers. Reused by the initial connect and by the reconnect loop,
   * so a dropped socket transparently re-attaches to the live pty — or resumes
   * the session — without tearing down the tab. `hadPtyId` records whether the
   * tab already knew a ptyId at connect time (used to decide whether the first
   * announced id should become the tab's stable identity).
   */
  function connectEntry(entry: TabEntry, query: string, hadPtyId?: string) {
    // Staged-seed delivery (fable-retry rephrases, fork-edited prompts): type the
    // seed into the pty once its output has settled — NOT on the daemon's session
    // frame. claude ≥2.1.200 resumes keep the session id and announce nothing at
    // startup (spike 13), so output quiescence is the only startup signal that
    // survives claude-internals churn. One-shot by construction: seedInput is
    // cleared at arm time, so a reconnect (which re-runs connectEntry) or reload
    // can never re-inject it. NO trailing newline — stage, never send.
    const seed = entry.descriptor.seedInput;
    let seedTiming: SeedTiming | null = null;
    let seedTimer: number | null = null;
    const disarmSeed = () => {
      if (seedTimer !== null) window.clearInterval(seedTimer);
      seedTimer = null;
      seedTiming = null;
    };
    if (seed) {
      entry.descriptor = { ...entry.descriptor, seedInput: undefined };
      seedTiming = { armedAt: Date.now(), lastOutputAt: null };
      seedTimer = window.setInterval(() => {
        if (seedTiming && seedDue(seedTiming, Date.now())) {
          disarmSeed();
          entry.ptyHandle?.sendInput(seed);
        }
      }, 100);
    }
    entry.ptyHandle = connectPty(query, entry.handle.term, {
      onOutput() {
        if (seedTiming) seedTiming.lastOutputAt = Date.now();
      },
      onPtyId(id) {
        // First frame after a (re)attach: the socket is live again.
        clearReconnect(entry);
        entry.dead = false;
        entry.descriptor = { ...entry.descriptor, ptyId: id };
        // If we opened without a ptyId, update the tab's identity — and carry
        // activeTabId along, or activeTab() would go null for the visible tab
        // (header would blank, drawer would unmount).
        if (entry.id !== id && !hadPtyId) {
          if (activeTabId === entry.id) activeTabId = id;
          entry.id = id;
        }
        saveTabs();
        renderTabStrip();
        if (entry.id === activeTabId) renderTermHeader();
      },
      onSessionUuid(uuid) {
        entry.descriptor = { ...entry.descriptor, uuid };
        saveTabs();
        renderTabStrip();
        // The active tab just gained a transcript — swap the placeholder out.
        if (entry.id === activeTabId) {
          syncDock();
        }
      },
      onExit() {
        // The child genuinely exited — not a transport drop. Don't reconnect.
        disarmSeed(); // never type a seed into a dead pty
        clearReconnect(entry);
        entry.state = "exited";
        entry.dead = true;
        renderTabStrip();
        if (entry.id === activeTabId) renderTermHeader();
      },
      onClose(reason) {
        // A dropped socket loses the seed by design (clearing at arm time is what
        // guarantees no re-injection); stop the timer before any reconnect path.
        disarmSeed();
        if (entry.disposed) return; // tab being closed by the user.
        const attachMiss = reason === "no live pty with that id";
        if (attachMiss && !entry.descriptor.uuid) {
          // Stale ephemeral attach (e.g. a boot-restore race) with nothing to
          // resume from: drop the tab, as before.
          closeTab(entry.id);
          void refreshRoster();
        } else if (reason === "" || attachMiss) {
          // Recoverable drop — daemon restart (cargo watch), idle reaping, or a
          // renumbered/resumable pty. Keep the tab and reconnect with backoff.
          scheduleReconnect(entry);
        } else {
          // Genuine policy close (e.g. "no such directory"): surface and stop.
          clearReconnect(entry);
          entry.dead = true;
          entry.descriptor = { ...entry.descriptor, label: `✗ ${reason}` };
          renderTabStrip();
          if (entry.id === activeTabId) renderTermHeader();
        }
      },
    });
  }

  // ------------------------------------------------------------------
  // Reconnect loop — a dropped pty socket (daemon restart under `cargo watch`,
  // or idle TCP reaping on WSL2 localhost) used to leave the tab silently dead:
  // still focusable, but every keystroke dropped. Instead we reconcile against
  // the live ptys and re-attach (or resume) on the SAME terminal, which is the
  // exact path a fresh browser tab takes — and known to work.
  // ------------------------------------------------------------------

  /** Give up after ~2 min of a never-returning daemon (cap × attempts). */
  const MAX_RECONNECT_ATTEMPTS = 40;

  function clearReconnect(entry: TabEntry) {
    if (entry.reconnectTimer !== null) {
      clearTimeout(entry.reconnectTimer);
      entry.reconnectTimer = null;
    }
    entry.reconnecting = false;
    entry.reconnectAttempt = 0;
  }

  function scheduleReconnect(entry: TabEntry) {
    if (entry.disposed || entry.reconnectTimer !== null) return;
    entry.reconnecting = true;
    entry.dead = false;
    entry.state = "reconnecting";
    renderTabStrip();
    if (entry.id === activeTabId) renderTermHeader();

    const delay = reconnectDelay(entry.reconnectAttempt);
    entry.reconnectTimer = window.setTimeout(() => {
      entry.reconnectTimer = null;
      void attemptReconnect(entry);
    }, delay);
  }

  function giveUpReconnect(entry: TabEntry) {
    entry.reconnecting = false;
    entry.reconnectAttempt = 0;
    entry.dead = true;
    entry.state = "exited";
    renderTabStrip();
    if (entry.id === activeTabId) renderTermHeader();
  }

  async function attemptReconnect(entry: TabEntry) {
    if (entry.disposed) return;

    let ptys: PtyInfo[] | null = null;
    try {
      ptys = (await (await fetch("/api/pty")).json()) as PtyInfo[];
    } catch {
      // Daemon still down (mid-rebuild). Retry with backoff until the ceiling.
      ptys = null;
    }
    if (entry.disposed) return;

    if (ptys !== null) {
      const query = reconnectQuery(entry.descriptor, ptys);
      if (query) {
        // Reset to a fresh grid so the daemon's re-attach snapshot paints clean
        // (same as a brand-new tab), then re-open the socket.
        entry.handle.term.reset();
        const hadPtyId = entry.descriptor.ptyId;
        entry.ptyHandle?.dispose();
        connectEntry(entry, query, hadPtyId);
        return; // onPtyId clears the reconnect state on success.
      }
      // Daemon is up but the session is gone for good — stop trying.
      giveUpReconnect(entry);
      return;
    }

    entry.reconnectAttempt += 1;
    if (entry.reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
      giveUpReconnect(entry);
      return;
    }
    scheduleReconnect(entry);
  }

  // ------------------------------------------------------------------
  // Picker — overlay triggered by the "+" buttons (rail brand + tab strip)
  // ------------------------------------------------------------------

  /** Currently-mounted picker teardown handle (null when picker is closed). */
  let pickerTeardown: (() => void) | null = null;

  function openPicker(anchorEl: HTMLButtonElement) {
    // Idempotent: if already open, close it (toggle behaviour).
    if (pickerTeardown !== null) {
      pickerTeardown();
      pickerTeardown = null;
      return;
    }
    pickerTeardown = mountPicker(
      document.body,
      anchorEl,
      {
        onPick({ path, create }) {
          pickerTeardown = null;
          const query = create
            ? `?new=${encodeURIComponent(path)}&create=1`
            : `?new=${encodeURIComponent(path)}`;
          openTabWithQuery(query, { label: basename(path), cwd: path });
        },
        onDismiss() {
          pickerTeardown = null;
        },
      },
    );
  }

  newBtn.addEventListener("click", () => openPicker(newBtn));

  /** Path basename (everything after the last "/"). */
  function basename(p: string): string {
    const i = p.lastIndexOf("/");
    return i >= 0 ? p.slice(i + 1) : p;
  }

  // ------------------------------------------------------------------
  // Rail: search + grouped roster + footer
  // ------------------------------------------------------------------

  let overrides: Record<string, string> = {};
  try {
    overrides = JSON.parse(localStorage.getItem(LS_OVERRIDES) ?? "{}") as Record<string, string>;
  } catch {
    overrides = {};
  }

  function saveOverride(key: string, value: string) {
    overrides[key] = value;
    localStorage.setItem(LS_OVERRIDES, JSON.stringify(overrides));
  }

  async function fetchRosterData(): Promise<{ ptys: PtyInfo[]; forest: ForestItem[] }> {
    const [ptyRes, forestRes] = await Promise.all([
      fetch("/api/pty"),
      fetch("/api/forest"),
    ]);
    const ptys = (await ptyRes.json()) as PtyInfo[];
    const forest = (await forestRes.json()) as ForestItem[];
    return { ptys, forest };
  }

  /** Latest fetched roster — re-rendered locally on search input / tab switch. */
  let lastRows: RosterRow[] = [];
  let searchQuery = "";

  // ── Forest selection + preview float ──────────────────────────────────────
  // Focusing a row (click or ↑/↓) selects it and previews its transcript; launch
  // is a separate commit (Enter / double-click / the float's Launch button).
  let selectedKey: string | null = null;
  /** Flattened, group-ordered visible rows — the keyboard-nav order. */
  let visibleRows: RosterRow[] = [];
  /** row.key → its rendered rail button, for focus/scroll + selected styling. */
  const rowEls = new Map<string, HTMLElement>();

  const preview: ForestPreviewHandle = createForestPreview({
    onLaunch: (row) => {
      launchRow(row);
      preview.hide();
    },
    onFork: (newUuid, text) => {
      openTabWithQuery(`?session=${encodeURIComponent(newUuid)}`, {
        uuid: newUuid,
        label: "fork",
        seedInput: text,
      });
      void refreshRoster();
    },
  });

  /** Launch/attach a session (the old single-click behavior, now an explicit commit). */
  function launchRow(row: RosterRow) {
    if (row.ptyId) {
      openTabWithQuery(`?attach=${row.ptyId}`, {
        ptyId: row.ptyId,
        uuid: row.uuid,
        label: row.label,
        cwd: row.cwd,
      });
    } else if (row.uuid) {
      openTabWithQuery(`?session=${row.uuid}`, {
        uuid: row.uuid,
        label: row.label,
        cwd: row.cwd,
      });
    }
  }

  /** Focus a row: mark it selected and float its preview. */
  function selectRow(row: RosterRow) {
    selectedKey = row.key;
    for (const [key, elm] of rowEls) {
      elm.classList.toggle("rail-row--selected", key === selectedKey);
    }
    const anchor = rowEls.get(row.key);
    if (anchor) {
      anchor.focus({ preventScroll: true });
      anchor.scrollIntoView({ block: "nearest" });
      preview.show(row, anchor);
    }
  }

  /** Clear selection and dismiss the preview float. */
  function clearSelection() {
    selectedKey = null;
    for (const elm of rowEls.values()) elm.classList.remove("rail-row--selected");
    preview.hide();
  }

  /** Move selection by `delta` through the visible rows (clamped at the ends). */
  function moveSelection(delta: number) {
    if (visibleRows.length === 0) return;
    const i = visibleRows.findIndex((r) => r.key === selectedKey);
    const next = i === -1 ? (delta > 0 ? 0 : visibleRows.length - 1)
                         : Math.min(visibleRows.length - 1, Math.max(0, i + delta));
    selectRow(visibleRows[next]!);
  }

  // Keyboard nav for the forest: ↑/↓ move selection (driving the preview),
  // Enter launches, Esc dismisses. Ignored while a rename input has focus so
  // typing is never hijacked. The search box is a sibling of railScroll, so its
  // own typing is unaffected; ArrowDown from search jumps into the list.
  railScroll.addEventListener("keydown", (e) => {
    if (document.activeElement instanceof HTMLInputElement) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      moveSelection(1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      moveSelection(-1);
    } else if (e.key === "Enter") {
      const row = visibleRows.find((r) => r.key === selectedKey);
      if (row) {
        e.preventDefault();
        launchRow(row);
        preview.hide();
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      clearSelection();
    }
  });

  searchInput.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" && visibleRows.length > 0) {
      e.preventDefault();
      selectRow(visibleRows[0]!);
    }
  });

  // Dismiss the preview when clicking outside it and outside the rail rows
  // (clicking another row re-selects via that row's own handler).
  document.addEventListener("pointerdown", (e) => {
    if (!preview.isOpen()) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.closest(".forest-preview") || t.closest(".rail-row"))) return;
    clearSelection();
  });

  searchInput.addEventListener("input", () => {
    searchQuery = searchInput.value.trim().toLowerCase();
    renderRail();
  });

  /** True when this row backs the active tab. */
  function isActiveRow(row: RosterRow): boolean {
    const d = activeTab()?.descriptor;
    if (!d) return false;
    if (row.ptyId && d.ptyId) return row.ptyId === d.ptyId;
    if (row.uuid && d.uuid) return row.uuid === d.uuid;
    return false;
  }

  function renderRail() {
    // Guard: don't clobber an active inline-rename input.
    if (railScroll.contains(document.activeElement) &&
        document.activeElement instanceof HTMLInputElement &&
        document.activeElement.classList.contains("rail-rename-input")) {
      return;
    }
    const now = Date.now();
    railScroll.innerHTML = "";
    rowEls.clear();
    visibleRows = [];

    const rows = searchQuery
      ? lastRows.filter((r) =>
          r.label.toLowerCase().includes(searchQuery) ||
          r.cwdChip.toLowerCase().includes(searchQuery))
      : lastRows;

    if (rows.length === 0) {
      const empty = el("div", "rail-empty");
      empty.textContent = searchQuery ? "no matching sessions" : "no sessions";
      railScroll.append(empty);
    }

    for (const group of GROUP_ORDER) {
      const groupRows = rows.filter((r) => ageGroup(r.recency, now) === group);
      if (groupRows.length === 0) continue;

      const header = el("div", "rail-group-header");
      const label = el("span", "rail-group-label");
      label.textContent = GROUP_LABELS[group];
      const rule = el("span", "rail-group-rule");
      const count = el("span", "rail-group-count");
      count.textContent = String(groupRows.length);
      header.append(label, rule, count);
      railScroll.append(header);

      for (const row of groupRows) {
        visibleRows.push(row);
        const item = renderRailRow(row, now);
        rowEls.set(row.key, item);
        railScroll.append(item);
      }
    }

    // Preserve selection across re-renders; drop it (and the float) if the
    // selected row is gone (e.g. filtered out or no longer in the roster).
    if (selectedKey !== null) {
      if (rowEls.has(selectedKey)) {
        rowEls.get(selectedKey)!.classList.add("rail-row--selected");
      } else {
        clearSelection();
      }
    }

    renderRailFoot();
  }

  function renderRailRow(row: RosterRow, now: number): HTMLElement {
    const item = el("button", "rail-row");
    item.style.setProperty("--row-ink", inkVar(row.cwd, row.cwdChip));
    if (isActiveRow(row)) item.classList.add("rail-row--active");

    const dotWrap = el("span", "rail-row-dot");
    const dot = el("span", dotClasses(row.activity, row.liveness));
    dot.title = dotTitle(row.activity, row.liveness);
    dotWrap.append(dot);

    const body = el("span", "rail-row-body");
    const labelEl = el("span", "rail-row-label");
    labelEl.textContent = row.label;
    const meta = el("span", "rail-row-meta");
    const project = el("span", "rail-row-project");
    project.textContent = row.cwdChip;
    meta.append(project);
    if (row.msgCount !== undefined) {
      const count = el("span", "rail-row-count");
      count.textContent = `~${row.msgCount}`;
      meta.append(count);
    }
    const tag = livenessTag(row.activity, row.liveness);
    if (tag) {
      const live = el("span", "rail-row-live");
      if (row.liveness === "external") live.classList.add("rail-row-live--external");
      live.textContent = tag;
      live.title = dotTitle(row.activity, row.liveness);
      meta.append(live);
    }
    if (row.downgrade) {
      const downgrade = el("span", "rail-downgrade");
      downgrade.textContent = "fable→opus";
      downgrade.title =
        "Guardrail downgraded this session to Opus — a Fable retry can be staged";
      meta.append(downgrade);
    }
    body.append(labelEl, meta);

    const recencyEl = el("span", "rail-row-recency");
    recencyEl.textContent = relativeRecency(row.recency, now);

    item.append(dotWrap, body, recencyEl);

    if (row.key === selectedKey) item.classList.add("rail-row--selected");

    // Single click / focus → select + preview (no launch). Launch is a separate
    // commit: double-click, Enter, or the float's Launch button.
    item.addEventListener("click", () => selectRow(row));
    item.addEventListener("dblclick", (e) => {
      e.preventDefault();
      launchRow(row);
      preview.hide();
    });

    // Double-click label → inline rename → localStorage override.
    labelEl.addEventListener("dblclick", (e) => {
      e.stopPropagation();
      const input = document.createElement("input");
      input.className = "rail-rename-input";
      input.value = row.label;
      labelEl.replaceWith(input);
      input.focus();
      input.select();

      const commit = () => {
        const newLabel = input.value.trim();
        if (newLabel) {
          const overrideKey = row.uuid ?? row.ptyId;
          if (overrideKey) saveOverride(overrideKey, newLabel);
        }
        void refreshRoster();
      };

      input.addEventListener("blur", commit, { once: true });
      input.addEventListener("keydown", (ev) => {
        if (ev.key === "Enter") {
          input.blur();
        } else if (ev.key === "Escape") {
          input.removeEventListener("blur", commit);
          void refreshRoster();
        }
      });
    });

    return item;
  }

  function renderRailFoot() {
    railFoot.innerHTML = "";
    const working = lastRows.filter((r) => r.liveness !== "none" && r.activity === "working").length;
    const dot = el(
      "span",
      working > 0 ? dotClasses("working", "eigenform") : dotClasses("idle", "eigenform"),
    );
    const label = el("span");
    label.textContent = `${working} working`;
    const total = el("span", "rail-foot-total");
    total.textContent = `${lastRows.length} sessions`;
    railFoot.append(dot, label, total);
  }

  type RecoverOutcome =
    | { ok: true }
    | { ok: false; kind: "http"; status: number }
    | { ok: false; kind: "network" };

  /**
   * POST the Fable→Opus recovery for one source session: fork a rewound Fable
   * branch, open it as a tab, and STAGE the (rephrased) prompt into it — never
   * sent. Shared by the automatic forest-poll path (maybeAutoRecover) and the
   * manual topbar trigger (triggerRecover). All side effects live here; the
   * caller owns the retry/gate policy via the returned outcome. The daemon
   * records a `downgrade-recovered` / `downgrade-recovery-failed` event for
   * every call, so the Events pane is the source of truth for what happened.
   */
  async function recoverDowngrade(uuid: string): Promise<RecoverOutcome> {
    try {
      const res = await fetch(
        "/api/session/" + encodeURIComponent(uuid) + "/recover-downgrade",
        { method: "POST" },
      );
      if (!res.ok) {
        console.warn(`recover-downgrade failed (${res.status}) for ${uuid}`);
        return { ok: false, kind: "http", status: res.status };
      }
      const { branchUuid, stagedText, note } = (await res.json()) as {
        branchUuid: string;
        stagedText: string;
        offendingTurn: string;
        note: string | null;
      };
      // note != null → the rephraser fell back to verbatim. The Events pane
      // already shows this (rephrased:false); the console note keeps the old
      // dev breadcrumb without baking a one-shot note into the persisted label.
      if (note) console.warn(`fable retry rephrase note for ${uuid}: ${note}`);
      openTabWithQuery("?session=" + encodeURIComponent(branchUuid), {
        uuid: branchUuid,
        label: "fable-retry",
        seedInput: stagedText,
      });
      void refreshRoster();
      return { ok: true };
    } catch (err) {
      console.warn(`recover-downgrade errored for ${uuid}:`, err);
      return { ok: false, kind: "network" };
    }
  }

  /**
   * If the ACTIVE session shows a Fable→Opus downgrade (and the user isn't
   * mid-keystroke), auto-stage a forked retry. Fires at most once per source
   * session. `forest` is the raw snapshot from the poll (ForestItem carries uuid
   * + downgrade, so it satisfies DowngradeCandidate structurally).
   */
  async function maybeAutoRecover(forest: ForestItem[]) {
    const hit = shouldAutoRecover({
      activeUuid: activeTab()?.descriptor.uuid ?? null,
      rows: forest,
      handled: recovered,
      lastInputAt,
      now: Date.now(),
      recentInputMs: 1500,
    });
    if (!hit) return;
    // Mark handled BEFORE the await so a slow POST can't double-fire on the next
    // 3s poll. Kept handled on a deliberate non-ok too, so it doesn't spin; only
    // a network blip is retried (clear the mark so the next poll re-attempts).
    recovered.add(hit.uuid);
    const outcome = await recoverDowngrade(hit.uuid);
    if (!outcome.ok && outcome.kind === "network") recovered.delete(hit.uuid);
  }

  /**
   * Manual GUI trigger: recover the ACTIVE session's downgrade on demand and
   * surface the result in the Events pane. Unlike the auto path it ignores the
   * once-per-session gate (deliberately re-runnable — this is the "let me test
   * the mechanism myself" button) but marks the source handled on success so the
   * forest poll won't also fire for it.
   */
  async function triggerRecover() {
    const uuid = activeTab()?.descriptor.uuid ?? null;
    if (!uuid) return;
    // Make the outcome visible: open the dock and expand the Events pane first so
    // the recorded downgrade-recovered / -failed row lands in view.
    if (!drawerOpen) setDrawerOpen(true);
    eventsCurrent?.reveal();
    const outcome = await recoverDowngrade(uuid);
    if (outcome.ok) recovered.add(uuid);
  }

  async function refreshRoster() {
    try {
      const { ptys, forest } = await fetchRosterData();
      lastRows = buildRoster(ptys, forest, overrides);
      renderRail();

      // Track which sessions are currently downgraded (drives the recover button's
      // attention state; renderControls runs later via syncDock).
      downgradedUuids.clear();
      for (const it of forest) if (it.downgrade) downgradedUuids.add(it.uuid);

      // Auto-stage a Fable retry for the active downgraded session (once each).
      void maybeAutoRecover(forest);

      // Update tab state badges + cwd + uuid from live pty data.
      for (const t of tabs) {
        if (!t.descriptor.ptyId || t.dead) continue;
        const live = ptys.find((p) => p.id === t.descriptor.ptyId);
        if (live) {
          t.state = live.state;
          let changed = false;
          if (live.cwd && !t.descriptor.cwd) {
            t.descriptor = { ...t.descriptor, cwd: live.cwd };
            changed = true;
          }
          // A fresh session's uuid is resolved late (the JSONL/pid watcher); adopt it
          // here so the transcript drawer can mount — without this the drawer stays on
          // its "waiting for a session uuid" placeholder forever.
          if (live.uuid && !t.descriptor.uuid) {
            t.descriptor = { ...t.descriptor, uuid: live.uuid };
            changed = true;
          }
          if (changed) saveTabs();
        }
      }
      renderTabStrip();
      renderTermHeader();
      // A newly-adopted uuid on the active tab means the dock can now mount.
      syncDock();
    } catch {
      // Daemon not reachable — keep stale rail.
    }
  }

  // ------------------------------------------------------------------
  // Boot: restore persisted tabs, initial roster, start poll.
  // ------------------------------------------------------------------

  async function boot() {
    renderRail();
    renderTabStrip();
    renderControls();
    renderTermHeader();

    let ptys: PtyInfo[] = [];
    try {
      const data = await fetchRosterData();
      ptys = data.ptys;
      lastRows = buildRoster(data.ptys, data.forest, overrides);
      renderRail();
    } catch {
      // Daemon not available — skip tab restore.
    }

    // Restore persisted tabs.
    try {
      const saved = JSON.parse(localStorage.getItem(LS_KEY) ?? "[]") as TabDescriptor[];
      const actions = reconcileTabs(saved, ptys);
      for (const act of actions) {
        if (act.action === "attach") {
          openTabWithQuery(`?attach=${act.descriptor.ptyId}`, act.descriptor);
        } else if (act.action === "resume") {
          openTabWithQuery(`?session=${act.descriptor.uuid}`, act.descriptor);
        }
        // drop: do nothing.
      }
    } catch {
      localStorage.removeItem(LS_KEY);
    }

    renderTabStrip();
    renderTermHeader();
    syncDock();
  }

  void boot();

  // 3-second roster poll; cancel on hide, restart on show to avoid background fan-out.
  let pollInterval = setInterval(() => void refreshRoster(), 3000);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") {
      clearInterval(pollInterval);
    } else {
      // Page became visible again — refresh immediately then resume polling.
      void refreshRoster();
      pollInterval = setInterval(() => void refreshRoster(), 3000);
    }
  });
}

// ---------------------------------------------------------------------------
// DOM utility
// ---------------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}
