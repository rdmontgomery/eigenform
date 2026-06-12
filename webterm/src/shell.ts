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
 * THEME: dark (default) / light, toggled from the top bar, persisted
 * (LS_THEME). The terminal pane stays dark ink in both — see .term-scope.
 *
 * LOCALSTORAGE SCHEMA (key "eigenform:term:tabs:v1"):
 *   JSON array of TabDescriptor. Versioned key — bump suffix if schema changes.
 *
 * POLL: rail polls GET /api/pty + GET /api/forest every 3s to update badges.
 * Interval is cleared on visibilitychange → hidden to avoid background fan-out.
 */

import { newTerminal, connectPty } from "./pty.ts";
import { buildRoster } from "./roster.ts";
import type { RosterRow } from "./roster.ts";
import type { PtyInfo, ForestItem } from "./types.ts";
import {
  relativeRecency,
  reconcileTabs,
  ageGroup,
  inkFor,
  railFromPointer,
  RAIL_DEFAULT,
  type AgeGroup,
  type TabDescriptor,
  type TabReconcileAction,
} from "./shell-helpers.ts";
import { mountPicker } from "./picker.ts";
import { mountDrawer } from "./drawer.ts";
import type { DrawerHandle } from "./drawer.ts";
import { icon } from "./icons.ts";

// Re-export so callers can reach pure helpers via either module.
export { relativeRecency, reconcileTabs };
export type { TabDescriptor, TabReconcileAction };

// ---------------------------------------------------------------------------
// Internal constants
// ---------------------------------------------------------------------------

const LS_KEY = "eigenform:term:tabs:v1";
const LS_OVERRIDES = "eigenform:term:overrides:v1";
const LS_THEME = "eigenform:term:theme:v1";
const LS_DRAWER = "eigenform:term:drawer:v1";
const LS_RAIL = "eigenform:term:rail:v1";

const KNOWN_STATES = new Set(["working", "waiting", "idle", "exited"]);

/** CSS dot modifier for a state string; unknown forest states render idle. */
function dotClass(state: string): string {
  return `dot dot--${KNOWN_STATES.has(state) ? state : "idle"}`;
}

/** The session's ink hue CSS value, from its most durable key. */
function inkVar(uuid: string | undefined, ptyId: string | undefined, fallback: string): string {
  return `var(--ink-${inkFor(uuid ?? ptyId ?? fallback)})`;
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
  let theme = localStorage.getItem(LS_THEME) === "light" ? "light" : "dark";
  applyTheme(theme);

  function applyTheme(t: string) {
    document.documentElement.classList.toggle("theme-light", t === "light");
  }

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

  const termArea = el("div", "term-area term-scope");
  const termHeader = el("div", "term-header");
  const termHost = el("div", "term-host");
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

  function activeTab(): TabEntry | null {
    return tabs.find((t) => t.id === activeTabId) ?? null;
  }

  function saveTabs() {
    const descriptors: TabDescriptor[] = tabs.map((t) => t.descriptor);
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
    syncDrawer();
    renderRail();
  }

  function closeTab(id: string) {
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    const t = tabs[idx]!;

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
    syncDrawer();
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
  // Drawer — GLOBAL toggle (persistent control top-right), follows the
  // active tab. Replaces the old per-tab hamburger, which could open the
  // drawer but never close it.
  // ------------------------------------------------------------------

  let drawerOpen = localStorage.getItem(LS_DRAWER) === "1";
  /** Mounted transcript drawer (uuid-bound), or null. */
  let drawerCurrent: { uuid: string; handle: DrawerHandle } | null = null;
  /** Placeholder panel shown when the drawer is open but the active tab has
   *  no session uuid yet. */
  let drawerPlaceholder: HTMLElement | null = null;

  function setDrawerOpen(open: boolean) {
    drawerOpen = open;
    localStorage.setItem(LS_DRAWER, open ? "1" : "0");
    syncDrawer();
  }

  /** Reconcile the mounted drawer against (drawerOpen, active tab). */
  function syncDrawer() {
    const uuid = drawerOpen ? (activeTab()?.descriptor.uuid ?? null) : null;

    if (drawerPlaceholder) {
      drawerPlaceholder.remove();
      drawerPlaceholder = null;
    }

    if (!drawerOpen || tabs.length === 0) {
      drawerCurrent?.handle.close();
      drawerCurrent = null;
      renderControls();
      return;
    }

    if (uuid) {
      if (drawerCurrent?.uuid !== uuid) {
        drawerCurrent?.handle.close();
        // onFork: open the forked session as a new tab + refresh the roster so
        // the rail shows it immediately (copy-on-fork — source tab stays open).
        drawerCurrent = {
          uuid,
          handle: mountDrawer(
            termHost,
            uuid,
            (newUuid) => {
              openTabWithQuery(`?session=${encodeURIComponent(newUuid)}`, {
                uuid: newUuid,
                label: "fork",
              });
              void refreshRoster();
            },
            // interrupt routes ^C to the ACTIVE tab's socket — the drawer is
            // only ever mounted for the active tab's uuid (syncDrawer invariant).
            { interrupt: () => activeTab()?.ptyHandle?.sendInput("\x03") },
          ),
        };
      }
    } else {
      drawerCurrent?.handle.close();
      drawerCurrent = null;
      drawerPlaceholder = el("div", "drawer");
      const head = el("div", "drawer-header");
      const title = el("span", "drawer-title");
      title.textContent = "Transcript";
      head.append(title);
      const empty = el("div", "drawer-empty");
      empty.textContent = "no transcript yet — waiting for a session uuid";
      drawerPlaceholder.append(head, empty);
      termHost.append(drawerPlaceholder);
    }
    renderControls();
  }

  // ------------------------------------------------------------------
  // Top bar: global controls (theme · drawer)
  // ------------------------------------------------------------------

  function renderControls() {
    controls.innerHTML = "";

    const themeBtn = el("button", "icon-btn");
    themeBtn.title = theme === "dark" ? "Light mode" : "Dark mode";
    themeBtn.append(icon(theme === "dark" ? "sun" : "moon", 16));
    themeBtn.addEventListener("click", () => {
      theme = theme === "dark" ? "light" : "dark";
      localStorage.setItem(LS_THEME, theme);
      applyTheme(theme);
      renderControls();
    });

    const sep = el("div", "topbar-sep");

    const drawerBtn = el("button", `icon-btn${drawerOpen ? " icon-btn--active" : ""}`);
    drawerBtn.title = drawerOpen ? "Hide transcript" : "Show transcript";
    drawerBtn.append(icon("panel", 16));
    drawerBtn.addEventListener("click", () => setDrawerOpen(!drawerOpen));

    controls.append(themeBtn, sep, drawerBtn);
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
          inkVar(t.descriptor.uuid, t.descriptor.ptyId, t.descriptor.label),
        );
      }
      if (t.dead) tab.classList.add("tab--dead");

      const badge = el("span", dotClass(t.state));

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
    termHost.append(termEl);

    const handle = newTerminal();
    handle.term.open(termEl);

    const entry: TabEntry = {
      id: tabId,
      descriptor: desc,
      termEl,
      handle,
      ptyHandle: null,
      state: "idle",
      dead: false,
    };

    const ptyHandle = connectPty(query, handle.term, {
      onPtyId(id) {
        entry.descriptor = { ...entry.descriptor, ptyId: id };
        // If we opened without a ptyId, update the tab's identity — and carry
        // activeTabId along, or activeTab() would go null for the visible tab
        // (header would blank, drawer would unmount).
        if (entry.id !== id && !desc.ptyId) {
          if (activeTabId === entry.id) activeTabId = id;
          entry.id = id;
        }
        saveTabs();
        renderTabStrip();
      },
      onSessionUuid(uuid) {
        entry.descriptor = { ...entry.descriptor, uuid };
        saveTabs();
        renderTabStrip();
        // The active tab just gained a transcript — swap the placeholder out.
        if (entry.id === activeTabId) syncDrawer();
      },
      onExit() {
        entry.state = "exited";
        entry.dead = true;
        renderTabStrip();
        if (entry.id === activeTabId) renderTermHeader();
      },
      onClose(reason) {
        if (reason === "no live pty with that id") {
          // Attach-miss: drop the tab and refresh roster.
          closeTab(entry.id);
          void refreshRoster();
        } else if (reason) {
          // Policy close or other close with a reason: mark dead and annotate label
          // so the user can see WHY the tab died (e.g. "create outside workspace root").
          entry.dead = true;
          entry.descriptor = { ...entry.descriptor, label: `✗ ${reason}` };
          renderTabStrip();
          if (entry.id === activeTabId) renderTermHeader();
        } else {
          // Any other close: mark dead so user can see + manually close.
          entry.dead = true;
          renderTabStrip();
          if (entry.id === activeTabId) renderTermHeader();
        }
      },
    });

    entry.ptyHandle = ptyHandle;
    tabs.push(entry);
    saveTabs();
    activateTab(entry.id);

    requestAnimationFrame(() => {
      try { handle.fit.fit(); } catch { /* ok */ }
    });

    return entry;
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
        railScroll.append(renderRailRow(row, now));
      }
    }

    renderRailFoot();
  }

  function renderRailRow(row: RosterRow, now: number): HTMLElement {
    const item = el("button", "rail-row");
    item.style.setProperty("--row-ink", inkVar(row.uuid, row.ptyId, row.cwdChip));
    if (isActiveRow(row)) item.classList.add("rail-row--active");

    const dotWrap = el("span", "rail-row-dot");
    dotWrap.append(el("span", dotClass(row.state)));

    const body = el("span", "rail-row-body");
    const labelEl = el("span", "rail-row-label");
    labelEl.textContent = row.label;
    const meta = el("span", "rail-row-meta");
    const project = el("span", "rail-row-project");
    project.textContent = row.cwdChip;
    meta.append(project);
    if (row.live) {
      const live = el("span", "rail-row-live");
      live.textContent = "· live";
      meta.append(live);
    }
    body.append(labelEl, meta);

    const recencyEl = el("span", "rail-row-recency");
    recencyEl.textContent = relativeRecency(row.recency, now);

    item.append(dotWrap, body, recencyEl);

    // Click: open/attach to this session.
    item.addEventListener("click", () => {
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
    const working = lastRows.filter((r) => r.live && r.state === "working").length;
    const dot = el("span", working > 0 ? "dot dot--working" : "dot dot--idle");
    const label = el("span");
    label.textContent = `${working} working`;
    const total = el("span", "rail-foot-total");
    total.textContent = `${lastRows.length} sessions`;
    railFoot.append(dot, label, total);
  }

  async function refreshRoster() {
    try {
      const { ptys, forest } = await fetchRosterData();
      lastRows = buildRoster(ptys, forest, overrides);
      renderRail();

      // Update tab state badges + cwd from live pty data.
      for (const t of tabs) {
        if (!t.descriptor.ptyId || t.dead) continue;
        const live = ptys.find((p) => p.id === t.descriptor.ptyId);
        if (live) {
          t.state = live.state;
          if (live.cwd && !t.descriptor.cwd) {
            t.descriptor = { ...t.descriptor, cwd: live.cwd };
            saveTabs();
          }
        }
      }
      renderTabStrip();
      renderTermHeader();
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
    syncDrawer();
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
