/**
 * shell.ts — the composition root: skeleton DOM, the tab registry, pty
 * connect/reconnect, the tab strip + terminal header, and boot. Each surface
 * with its own state lives in its own module and talks back through callbacks:
 *
 *   rail.ts        session roster: search, age groups, selection + preview, rename
 *   rail-links.ts  the rail's Links section (URLs in the active tab's chat)
 *   dock.ts        inspect dock: reach map + transcript + events, splitters
 *   appearance.ts  color scheme + terminal font, their popovers, ⌘ zoom keys
 *   status.ts      status-dot / ink presentation shared by rail + tab strip
 *   snooze-ui.ts   tab snooze menu + snoozed-tabs shelf (snooze.ts: pure helpers)
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
 * persists across reloads (dock.ts). When the active tab has no session
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
 * ROSTER: the forest (every session on disk — the expensive scan) is PUSHED by
 * the daemon over SSE (GET /api/watch/forest) only when it changes; the rail
 * polls just GET /api/pty (the in-memory host registry, cheap) every 3s. Both
 * stop on visibilitychange → hidden and resume on show.
 */

import { newTerminal, connectPty, applyFont, applyTermTheme } from "./pty.ts";
import { buildRoster, ptyActivity } from "./roster.ts";
import type { RosterRow, Liveness } from "./roster.ts";
import type { PtyInfo, ForestItem } from "./types.ts";
import {
  relativeRecency,
  reconcileTabs,
  reorderTabs,
  reconnectQuery,
  reconnectDelay,
  tabSubtitle,
  railFromPointer,
  RAIL_DEFAULT,
  seedDue,
  type SeedTiming,
  type TabDescriptor,
  type TabReconcileAction,
} from "./shell-helpers.ts";
import { mountPicker } from "./picker.ts";
import { createAppearance } from "./appearance.ts";
import { mountRailLinks } from "./rail-links.ts";
import { mountRail } from "./rail.ts";
import { mountDock } from "./dock.ts";
import { mountArtifactDock } from "./artifact-dock.ts";
import type { PlanReview } from "./artifacts.ts";
import { stagedPayload } from "./annotate.ts";
import { dotClasses, dotTitle, inkVar } from "./status.ts";
import { icon } from "./icons.ts";
import { openInspect } from "./inspect.ts";
import { openClaims } from "./claims.ts";
import { el } from "./dom.ts";
import { mountTabSearch } from "./tab-search.ts";
import { dueSnoozes, snoozableTab, type Snooze } from "./snooze.ts";
import { openSnoozeMenu, openSnoozeShelf } from "./snooze-ui.ts";

// Re-export so callers can reach pure helpers via either module.
export { relativeRecency, reconcileTabs };
export type { TabDescriptor, TabReconcileAction };

// ---------------------------------------------------------------------------
// Internal constants
// ---------------------------------------------------------------------------

const LS_KEY = "eigenform:term:tabs:v1";
const LS_RAIL = "eigenform:term:rail:v1";


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
  /** true when a snooze just reopened this tab and it hasn't been looked at yet. */
  woke: boolean;
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
  /** Snoozed tabs as the daemon last reported them (see the Snooze section). */
  let snoozes: Snooze[] = [];
  /** Page title without the wake marker. */
  const baseTitle = document.title;

  // ------------------------------------------------------------------
  // Appearance — scheme + typography (applied before any layout so first
  // paint is correct); the shell supplies the per-terminal side effects.
  // ------------------------------------------------------------------
  const appearance = createAppearance({
    onScheme: (next) => {
      for (const t of tabs) applyTermTheme(t.handle.term, next.theme);
    },
    onFont: (font) => {
      for (const t of tabs) {
        applyFont(t.handle.term, font);
        // Re-measured cell → recompute cols/rows; onResize relays it to the daemon.
        try { t.handle.fit.fit(); } catch { /* zero-size element — ok */ }
      }
    },
    onChromeChange: () => renderControls(),
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
  const searchClear = el("button", "rail-search-clear");
  searchClear.title = "Clear search";
  searchClear.append(icon("x", 12));
  searchClear.hidden = true;
  searchBox.append(searchInput, searchClear);
  railSearch.append(searchBox);

  const railScroll = el("div", "rail-scroll scroll");
  const railLinks = el("div", "rail-links");
  const railFoot = el("button", "rail-foot");
  railFoot.addEventListener("click", () => showClaims());
  rail.append(railBrand, railSearch, railScroll, railLinks, railFoot);

  // main column
  const main = el("div", "main");
  const topbar = el("div", "topbar");
  const railBtn = el("button", "icon-btn topbar-rail-btn");
  const railBtnIcon = icon("panel", 16);
  railBtnIcon.style.transform = "scaleX(-1)"; // left-panel reading of the icon
  railBtn.append(railBtnIcon);
  const tabStrip = el("div", "tab-strip");
  // Drop on the strip's empty background (past the last tab) — append the
  // dragged tab at the end. Per-tab drop handlers (in renderTabStrip) target
  // e.target === a .tab element, so this only fires on the bare strip.
  tabStrip.addEventListener("dragover", (e) => {
    if (!dragTabId || e.target !== tabStrip) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
  });
  tabStrip.addEventListener("drop", (e) => {
    if (e.target !== tabStrip) return;
    e.preventDefault();
    const draggedId = dragTabId;
    dragTabId = null;
    if (!draggedId) return;
    applyTabOrder(reorderTabs(tabs.map((x) => x.id), draggedId, null, false));
    saveTabs();
    renderTabStrip();
  });
  const controls = el("div", "topbar-controls");
  // Find-a-tab filter (see tab-search.ts): highlights matches, dims the rest.
  const tabSearch = mountTabSearch({
    items: () =>
      tabs.map((t) => ({
        id: t.id,
        label: t.descriptor.label,
        cwd: t.descriptor.cwd,
        uuid: t.descriptor.uuid,
        ptyId: t.descriptor.ptyId,
      })),
    activate: (id) => activateTab(id),
    onChange: () => renderTabStrip(),
    onClose: () => activeTab()?.handle.term.focus(),
  });
  topbar.append(railBtn, tabSearch.el, tabStrip, controls);

  const termArea = el("div", "term-area");
  const termHeader = el("div", "term-header");
  const termHost = el("div", "term-host");
  // termStack holds the stacked term panes; the dock sits beside it (flex row)
  // and pushes it narrower when open, rather than floating over it.
  const termStack = el("div", "term-stack");
  // The inspect dock (reach map + transcript + events) sits beside the stack.
  const dock = mountDock(termHost, {
    onFork: (newUuid, text) => {
      openTabWithQuery(`?session=${encodeURIComponent(newUuid)}`, {
        uuid: newUuid,
        label: "fork",
        seedInput: text,
      });
      void refreshRoster();
    },
    onResize: () => fitActive(),
    onChange: () => renderControls(),
  });
  // The artifact pane (what the session made; annotate; plan reviews) sits between
  // the stack and the dock. See artifact-dock.ts / artifacts.ts for its sandboxing.
  const artifactDock = mountArtifactDock({
    stage: stageIntoActive,
    onResize: () => fitActive(),
  });
  termHost.append(termStack, artifactDock.resizer, artifactDock.el, dock.resizer, dock.el);
  termArea.append(termHeader, termHost);

  main.append(topbar, termArea);
  const resizer = el("div", "rail-resizer");
  resizer.title = "drag to resize the rail · drag far left to hide";
  appEl.append(rail, resizer, main);

  // ------------------------------------------------------------------
  // Rail resize / collapse — woland's splitter pattern: drag sets --rail-w
  // live, state persists on mouseup. Dragging left past the collapse
  // threshold hides the rail; the topbar button toggles it (re-expanding
  // restores the previous width), as does dragging back right.
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
    railBtn.title = railCollapsed ? "Show sessions" : "Hide sessions";
    railBtn.classList.toggle("icon-btn--active", !railCollapsed);
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

  // A browser-window or mobile-viewport resize does not reach xterm on its own:
  // the FitAddon only recomputes cols/rows when we call fit(). Without this the
  // grid keeps its old dimensions after a resize — claude's multi-line status bar
  // spills below the viewport and the transcript stops reflowing until an
  // unrelated re-fit (toggling the drawer) happens to nudge it. Coalesce the
  // event storm from a drag into one fit per frame. visualViewport catches the
  // mobile browser-chrome show/hide that a plain window `resize` misses.
  let refitQueued = false;
  const scheduleRefit = () => {
    if (refitQueued) return;
    refitQueued = true;
    requestAnimationFrame(() => {
      refitQueued = false;
      fitActive();
    });
  };
  window.addEventListener("resize", scheduleRefit);
  window.visualViewport?.addEventListener("resize", scheduleRefit);

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
    railCollapsed = !railCollapsed;
    applyRail();
    saveRail();
    fitActive();
  });

  // ------------------------------------------------------------------
  // Tab registry
  // ------------------------------------------------------------------
  const tabs: TabEntry[] = [];
  let activeTabId: string | null = null;
  /** Id of the tab currently being drag-reordered, or null when idle. */
  let dragTabId: string | null = null;

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

  /** Rearrange `tabs` in place to match `order` (a permutation of tab ids). */
  function applyTabOrder(order: string[]) {
    const byId = new Map(tabs.map((t) => [t.id, t] as const));
    tabs.length = 0;
    for (const id of order) {
      const t = byId.get(id);
      if (t) tabs.push(t);
    }
  }

  function activateTab(id: string) {
    for (const t of tabs) {
      const isActive = t.id === id;
      t.termEl.style.display = isActive ? "" : "none";
      if (isActive) {
        activeTabId = id;
        t.woke = false;
        // Re-fit on becoming visible so xterm dimensions are correct.
        requestAnimationFrame(() => {
          try { t.handle.fit.fit(); } catch { /* zero-size element — ok */ }
        });
      }
    }
    renderTabStrip();
    renderTermHeader();
    syncDock();
    railView.render();
    syncWakeTitle();
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
    railView.render();
    syncWakeTitle();
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
  // Inspect dock (see dock.ts) — global toggle, follows the active tab.
  // ------------------------------------------------------------------

  function setArtifactOpen(open: boolean) {
    artifactDock.setOpen(open);
    syncDock();
    fitActive();
  }

  /**
   * Type `text` into the active tab's terminal input WITHOUT submitting it (see
   * stagedPayload: bracketed paste when the TUI enabled it, else one flattened line).
   * The human reviews it in place and presses Enter. False when there's no live pty.
   */
  function stageIntoActive(text: string): boolean {
    const t = activeTab();
    if (!t?.ptyHandle) return false;
    t.ptyHandle.sendInput(stagedPayload(text, t.handle.term.modes.bracketedPasteMode));
    t.handle.term.focus();
    return true;
  }

  /** Plan gate: plans Claude is waiting on (ExitPlanMode hooks parked in the daemon). */
  async function refreshReviews() {
    try {
      const res = await fetch("/api/plan-reviews");
      const next: PlanReview[] = res.ok ? ((await res.json()) as PlanReview[]) : [];
      if (artifactDock.setReviews(next, activeTab()?.descriptor.uuid ?? null)) {
        syncDock();
        fitActive();
      }
      railView.render();
    } catch {
      // daemon unreachable — keep the last list
    }
  }

  function setDrawerOpen(open: boolean) {
    dock.setOpen(open);
    syncDock();
    // The dock's width changed → the terminal column resized; re-fit once.
    fitActive();
  }

  /** Reconcile the dock and the rail's Links section against the active tab. */
  function syncDock() {
    // The rail's Links section tracks the active tab's uuid the same way the
    // dock does, but it's a permanent sidebar fixture — not gated by the toggle.
    syncLinks();
    // The artifact pane follows the same tab changes (its own open state).
    artifactDock.sync(activeTab()?.descriptor ?? null, tabs.length > 0);
    dock.sync(activeTab()?.descriptor ?? null, tabs.length > 0);
  }

  // Rail "Links" — URLs mentioned in the active tab's chat (see rail-links.ts).
  // Always mounted; tracks the active tab's uuid via syncLinks() (from syncDock).
  const links = mountRailLinks(railLinks);
  function syncLinks() {
    links.sync(activeTab()?.descriptor.uuid ?? null);
  }

  // ------------------------------------------------------------------
  // Top bar: global controls (theme · reach · drawer)
  // ------------------------------------------------------------------

  function renderControls() {
    controls.innerHTML = "";

    const themeBtn = el("button", `icon-btn theme-btn${appearance.themePopover.isOpen() ? " icon-btn--active" : ""}`);
    themeBtn.title = `Theme — ${appearance.scheme().name}`;
    themeBtn.append(icon("palette", 16));
    themeBtn.addEventListener("click", () => appearance.themePopover.toggle(themeBtn));

    const fontBtn = el("button", `icon-btn font-btn${appearance.fontPopover.isOpen() ? " icon-btn--active" : ""}`);
    fontBtn.title = "Terminal font";
    fontBtn.append(icon("type", 16));
    fontBtn.addEventListener("click", () => appearance.fontPopover.toggle(fontBtn));

    // Config inventory — skills + memory across resolution layers, token-budgeted.
    // Scoped to the active tab's cwd when one is known, else machine-wide.
    const configBtn = el("button", "icon-btn config-btn");
    configBtn.title = "Config inventory (skills · memory)";
    configBtn.append(icon("sliders", 16));
    configBtn.addEventListener("click", () =>
      openInspect({ cwd: activeTab()?.descriptor.cwd ?? undefined }),
    );

    // Active sessions — every session Claude claims is running, to end or sweep.
    const claimsBtn = el("button", "icon-btn claims-btn");
    claimsBtn.title = "Active sessions (what Claude thinks is running)";
    claimsBtn.append(icon("pulse", 16));
    claimsBtn.addEventListener("click", showClaims);

    const sep = el("div", "topbar-sep");

    // Single "inspect" toggle — opens the docked panel (reach map + transcript).
    const drawerBtn = el("button", `icon-btn${dock.isOpen() ? " icon-btn--active" : ""}`);
    drawerBtn.title = dock.isOpen() ? "Hide inspect panel" : "Show inspect panel";
    drawerBtn.append(icon("panel", 16));
    drawerBtn.addEventListener("click", () => setDrawerOpen(!dock.isOpen()));

    // Artifact pane toggle — what the session made, rendered beside the terminal.
    const artifactOpen = artifactDock.isOpen();
    const artifactBtn = el("button", `icon-btn${artifactOpen ? " icon-btn--active" : ""}`);
    artifactBtn.title = artifactOpen
      ? "Hide artifact pane"
      : "Show artifact pane (HTML · SVG · markdown the session wrote)";
    artifactBtn.append(icon("window", 16));
    artifactBtn.addEventListener("click", () => {
      setArtifactOpen(!artifactDock.isOpen());
      renderControls();
    });

    controls.append(themeBtn, fontBtn, configBtn, claimsBtn, sep, artifactBtn, drawerBtn);

    // Snoozed-tabs shelf — only present while something is snoozed.
    if (snoozes.length > 0) {
      const shelfBtn = el("button", "icon-btn snooze-shelf-btn");
      shelfBtn.title = `Snoozed tabs (${snoozes.length})`;
      shelfBtn.append(icon("clock", 16));
      const count = el("span", "snooze-count");
      count.textContent = String(snoozes.length);
      shelfBtn.append(count);
      shelfBtn.addEventListener("click", () =>
        openSnoozeShelf(shelfBtn, snoozes, {
          onWake: (s) => void wakeSnooze(s, true),
          onCancel: (s) => void cancelSnooze(s),
        }),
      );
      controls.prepend(shelfBtn);
    }
  }

  function showClaims() {
    openClaims({
      onChange: () => void refreshRoster(),
      onOpenPty: (ptyId) => {
        const row = railView.rows().find((r) => r.ptyId === ptyId);
        if (row) launchRow(row);
        else openTabWithQuery(`?attach=${ptyId}`, { ptyId, label: "session" });
      },
    });
  }

  // ------------------------------------------------------------------
  // Tab strip
  // ------------------------------------------------------------------

  // The tab strip is fully rebuilt on every renderTabStrip call; appearance is
  // derived from the model (TabEntry), so rebuilds carry no stale-DOM risk.
  function renderTabStrip() {
    tabStrip.innerHTML = "";
    tabSearch.sync();
    for (const t of tabs) {
      const tab = el("div", "tab");
      tab.dataset.tabId = t.id;
      tab.classList.add(...tabSearch.classFor(t.id));
      // Every tab carries its project ink (the subtitle uses it); only the
      // active tab turns it into the top border.
      tab.style.setProperty("--tab-ink", inkVar(t.descriptor.cwd, t.descriptor.label));
      if (t.id === activeTabId) tab.classList.add("tab--active");
      if (t.dead) tab.classList.add("tab--dead");
      if (t.woke) tab.classList.add("tab--woke");

      // Drag-to-reorder: native HTML5 DnD, no library. The dragged tab's id
      // rides in dragTabId (closure state) rather than dataTransfer alone,
      // since dataTransfer.getData is unreadable during dragover in some
      // browsers — we only need it for the drop itself.
      tab.draggable = true;
      tab.addEventListener("dragstart", (e) => {
        dragTabId = t.id;
        tab.classList.add("tab--dragging");
        if (e.dataTransfer) {
          e.dataTransfer.effectAllowed = "move";
          e.dataTransfer.setData("text/plain", t.id);
        }
      });
      tab.addEventListener("dragend", () => {
        dragTabId = null;
        tab.classList.remove("tab--dragging");
        for (const el of tabStrip.querySelectorAll(".tab--drag-before, .tab--drag-after")) {
          el.classList.remove("tab--drag-before", "tab--drag-after");
        }
      });
      tab.addEventListener("dragover", (e) => {
        if (!dragTabId || dragTabId === t.id) return;
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        const rect = tab.getBoundingClientRect();
        const before = e.clientX - rect.left < rect.width / 2;
        tab.classList.toggle("tab--drag-before", before);
        tab.classList.toggle("tab--drag-after", !before);
      });
      tab.addEventListener("dragleave", () => {
        tab.classList.remove("tab--drag-before", "tab--drag-after");
      });
      tab.addEventListener("drop", (e) => {
        e.preventDefault();
        const before = tab.classList.contains("tab--drag-before");
        tab.classList.remove("tab--drag-before", "tab--drag-after");
        const draggedId = dragTabId;
        dragTabId = null;
        if (!draggedId) return;
        applyTabOrder(reorderTabs(tabs.map((x) => x.id), draggedId, t.id, before));
        saveTabs();
        renderTabStrip();
      });

      // Tabs are always eigenform-spawned (you can only open a tab on an
      // attachable pty); a dead/exited pty has no live process.
      const tabLiveness: Liveness = t.dead || t.state === "exited" ? "none" : "eigenform";
      const tabActivity = ptyActivity(t.state);
      const badge = el("span", dotClasses(tabActivity, tabLiveness));
      badge.title = dotTitle(tabActivity, tabLiveness);

      // Two-line text stack: title, then the launch dir (like the rail's chip).
      const textEl = el("span", "tab-text");
      const labelEl = el("span", "tab-label");
      labelEl.textContent = t.descriptor.label;
      textEl.append(labelEl);
      const sub = tabSubtitle(t.descriptor.cwd, t.descriptor.label);
      if (sub) {
        const subEl = el("span", "tab-sub");
        subEl.textContent = sub;
        subEl.title = t.descriptor.cwd ?? "";
        textEl.append(subEl);
      }

      const kill = el("button", "tab-kill");
      kill.title = "Kill pty (process terminated)";
      kill.append(icon("stop", 11, 2));
      kill.addEventListener("click", (e) => {
        e.stopPropagation();
        void killTab(t.id);
      });

      const snooze = el("button", "tab-snooze");
      snooze.title = "Snooze — close now, reopen later";
      snooze.append(icon("clock", 11, 2));
      snooze.addEventListener("click", (e) => {
        e.stopPropagation();
        openSnoozeMenu(snooze, t.descriptor.label, (until) => void snoozeTab(t.id, until));
      });

      const close = el("button", "tab-close");
      close.title = "Detach — close tab, pty stays alive";
      close.append(icon("x", 11, 2));
      close.addEventListener("click", (e) => {
        e.stopPropagation();
        closeTab(t.id);
      });

      tab.append(badge);
      if (t.descriptor.kind === "terminal") {
        const termIco = el("span", "tab-kind-ico");
        termIco.title = "Plain terminal — no transcript";
        termIco.append(icon("terminal", 11, 2));
        tab.append(termIco);
      }
      tab.append(textEl, snooze, kill, close);
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

  /**
   * Open a tab (or focus it if already open). `background` opens it without
   * stealing focus from the active tab — how a woken snooze comes back.
   */
  function openTabWithQuery(
    query: string,
    desc: TabDescriptor,
    opts: { background?: boolean } = {},
  ): TabEntry {
    const tabId = desc.ptyId ?? desc.uuid ?? `ephemeral-${Date.now()}`;
    const background = opts.background === true && activeTabId !== null;

    // Reuse existing tab if already open.
    const existing = tabs.find((t) => t.id === tabId);
    if (existing) {
      if (!background) activateTab(tabId);
      return existing;
    }

    const termEl = el("div", "term-pane");
    // xterm opens on an unpadded inner box: FitAddon measures its parent, and
    // .term-pane's padding would otherwise be counted as usable rows.
    const fitEl = el("div", "term-fit");
    termEl.append(fitEl);
    termStack.append(termEl);

    const handle = newTerminal(appearance.font(), appearance.scheme().theme);
    handle.term.open(fitEl);

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
      woke: false,
    };

    connectEntry(entry, query, desc.ptyId);
    tabs.push(entry);
    saveTabs();
    if (background) {
      termEl.style.display = "none";
      renderTabStrip();
      return entry;
    }
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
    // Staged-seed delivery (fork-edited prompts): type the
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
        onPick({ path, create, kind }) {
          pickerTeardown = null;
          const param = kind === "terminal" ? "term" : "new";
          const query = create
            ? `?${param}=${encodeURIComponent(path)}&create=1`
            : `?${param}=${encodeURIComponent(path)}`;
          openTabWithQuery(query, { label: basename(path), cwd: path, kind });
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
  // Rail: search + grouped roster + footer (see rail.ts). The shell owns
  // what a row *does* — launch, fork, which one is active.
  // ------------------------------------------------------------------

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

  /** True when this row backs the active tab. */
  function isActiveRow(row: RosterRow): boolean {
    const d = activeTab()?.descriptor;
    if (!d) return false;
    if (row.ptyId && d.ptyId) return row.ptyId === d.ptyId;
    if (row.uuid && d.uuid) return row.uuid === d.uuid;
    return false;
  }

  const railView = mountRail(
    { scroll: railScroll, foot: railFoot, searchInput, searchClear },
    {
      isActive: isActiveRow,
      onLaunch: launchRow,
      onFork: (newUuid, text) => {
        openTabWithQuery(`?session=${encodeURIComponent(newUuid)}`, {
          uuid: newUuid,
          label: "fork",
          seedInput: text,
        });
        void refreshRoster();
      },
      onRefresh: () => void refreshRoster(),
      hasPlanReview: (row) =>
        !!row.uuid && artifactDock.reviews().some((r) => r.sessionId === row.uuid),
    },
  );

  /** Latest forest snapshot pushed by /api/watch/forest; null until the first push. */
  let lastForest: ForestItem[] | null = null;
  let forestStream: EventSource | null = null;

  /** Subscribe to forest pushes. The daemon sends the current snapshot on connect,
   *  then again whenever it changes; each push re-renders the roster. EventSource
   *  reconnects on its own if the daemon restarts. */
  function openForestStream() {
    if (forestStream) return;
    forestStream = new EventSource("/api/watch/forest");
    forestStream.onmessage = (ev: MessageEvent<string>) => {
      try {
        lastForest = JSON.parse(ev.data) as ForestItem[];
      } catch {
        return;
      }
      void refreshRoster();
    };
  }

  function closeForestStream() {
    forestStream?.close();
    forestStream = null;
  }

  async function fetchRosterData(): Promise<{ ptys: PtyInfo[]; forest: ForestItem[] }> {
    const ptys = fetch("/api/pty").then((r) => r.json() as Promise<PtyInfo[]>);
    // Before the first push (boot), fetch the snapshot once so tab restore can
    // reconcile against it; after that the stream keeps lastForest current.
    const forest =
      lastForest !== null
        ? Promise.resolve(lastForest)
        : fetch("/api/forest").then((r) => r.json() as Promise<ForestItem[]>);
    const [p, f] = await Promise.all([ptys, forest]);
    lastForest ??= f;
    return { ptys: p, forest: f };
  }

  async function refreshRoster() {
    try {
      const { ptys, forest } = await fetchRosterData();
      railView.setRows(buildRoster(ptys, forest, railView.overrides()));

      // Update tab state badges + cwd + uuid + title from live pty / forest data.
      for (const t of tabs) {
        if (t.dead) continue;
        let changed = false;
        const live = t.descriptor.ptyId
          ? ptys.find((p) => p.id === t.descriptor.ptyId)
          : undefined;
        if (live) {
          t.state = live.state;
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
        }
        // Prefer a strong title over the tab's bare cwd basename: once the JSONL
        // yields an AI title (or the user renamed the session in the rail), adopt
        // it so tabs sharing one cwd stop all reading as "src". We only ever
        // upgrade to a user override or an AI title — never clobber a good cwd
        // label with a weaker fallback ("new session"). Override wins over aiTitle,
        // mirroring deriveLabel's precedence.
        const uuid = t.descriptor.uuid;
        const ptyId = t.descriptor.ptyId;
        const overrides = railView.overrides();
        const override =
          (uuid ? overrides[uuid] : undefined) ??
          (ptyId ? overrides[ptyId] : undefined) ??
          null;
        const aiTitle = uuid
          ? forest.find((f) => f.uuid === uuid)?.title ?? null
          : null;
        const strongLabel = override ?? aiTitle;
        if (strongLabel && strongLabel !== t.descriptor.label) {
          t.descriptor = { ...t.descriptor, label: strongLabel };
          changed = true;
        }
        if (changed) saveTabs();
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
  // Snooze — close a tab until a wake time. The daemon stores the snooze
  // (/api/snoozes, persisted); this page polls, and when one is due claims it
  // (DELETE — only one window wins) and reopens the tab in the background with
  // a flash and a ⏰ in the page title until it's looked at.
  // ------------------------------------------------------------------

  const SNOOZE_POLL_MS = 15_000;

  async function snoozeTab(id: string, until: number) {
    const t = tabs.find((t) => t.id === id);
    if (!t) return;
    try {
      const res = await fetch("/api/snoozes", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ until, tab: snoozableTab(t.descriptor) }),
      });
      if (!res.ok) throw new Error(await res.text());
      snoozes = [...snoozes, (await res.json()) as Snooze].sort((a, b) => a.until - b.until);
    } catch (e) {
      alert(`Couldn't snooze this tab: ${e instanceof Error ? e.message : e}`);
      return;
    }
    closeTab(id);
    renderControls();
  }

  async function cancelSnooze(s: Snooze) {
    snoozes = snoozes.filter((x) => x.id !== s.id);
    renderControls();
    try {
      await fetch(`/api/snoozes/${encodeURIComponent(s.id)}`, { method: "DELETE" });
    } catch {
      // Best-effort; the next poll re-syncs.
    }
  }

  /** Claim snooze `s` and reopen its tab. `focus` = woken by hand (switch to it). */
  async function wakeSnooze(s: Snooze, focus: boolean) {
    snoozes = snoozes.filter((x) => x.id !== s.id);
    renderControls();
    let ptys: PtyInfo[] = [];
    try {
      const res = await fetch(`/api/snoozes/${encodeURIComponent(s.id)}`, { method: "DELETE" });
      if (!res.ok) return; // another window already woke it
      ptys = (await (await fetch("/api/pty")).json()) as PtyInfo[];
    } catch {
      return;
    }
    const [act] = reconcileTabs([s.tab], ptys);
    let entry: TabEntry | null = null;
    if (act?.action === "attach") {
      entry = openTabWithQuery(`?attach=${act.descriptor.ptyId}`, act.descriptor, { background: !focus });
    } else if (act?.action === "resume") {
      entry = openTabWithQuery(`?session=${act.descriptor.uuid}`, act.descriptor, { background: !focus });
    } else {
      // A plain terminal whose pty has since exited: nothing left to reopen.
      alert(`Snoozed tab “${s.tab.label}” woke, but its terminal has exited.`);
      return;
    }
    if (!focus && entry.id !== activeTabId) {
      entry.woke = true;
      renderTabStrip();
    }
    syncWakeTitle();
  }

  async function refreshSnoozes() {
    try {
      snoozes = (await (await fetch("/api/snoozes")).json()) as Snooze[];
    } catch {
      return; // daemon unreachable — keep the last list
    }
    renderControls();
    for (const s of dueSnoozes(snoozes, Date.now())) await wakeSnooze(s, false);
  }

  /** ⏰ in the page title while any woken tab is still unseen. */
  function syncWakeTitle() {
    const n = tabs.filter((t) => t.woke).length;
    document.title = n > 0 ? `⏰ ${n} woke · ${baseTitle}` : baseTitle;
  }

  // ------------------------------------------------------------------
  // Boot: restore persisted tabs, initial roster, start poll.
  // ------------------------------------------------------------------

  async function boot() {
    railView.render();
    renderTabStrip();
    renderControls();
    renderTermHeader();

    let ptys: PtyInfo[] = [];
    try {
      const data = await fetchRosterData();
      ptys = data.ptys;
      railView.setRows(buildRoster(data.ptys, data.forest, railView.overrides()));
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
    // After restore, so a snooze that came due while the page was closed wakes
    // into the restored strip (and finds its tab if it was reopened by hand).
    void refreshSnoozes();
  }

  void boot();
  // Snoozes keep polling while the page is hidden: a wake is exactly when the
  // ⏰ title matters (browsers throttle hidden timers, but still fire them).
  setInterval(() => void refreshSnoozes(), SNOOZE_POLL_MS);

  // Forest arrives by push; only the cheap pty list is polled. Both stop while the
  // page is hidden and resume (with an immediate refresh) when it's shown again.
  openForestStream();
  let pollInterval = setInterval(() => void refreshRoster(), 3000);
  // Plan reviews are in-memory on the daemon (cheap) and Claude is blocked on them:
  // poll faster than the roster.
  let reviewInterval = setInterval(() => void refreshReviews(), 1500);
  void refreshReviews();
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") {
      clearInterval(pollInterval);
      clearInterval(reviewInterval);
      closeForestStream();
    } else {
      openForestStream();
      void refreshRoster();
      void refreshReviews();
      void refreshSnoozes();
      pollInterval = setInterval(() => void refreshRoster(), 3000);
      reviewInterval = setInterval(() => void refreshReviews(), 1500);
    }
  });
}
