/**
 * shell.ts — Sidebar roster + tab strip + terminal host.
 *
 * Layout: CSS grid — left rail (sidebar) | main area (tab strip + terminal host).
 * One Terminal per open tab, kept alive while the tab exists, hidden via
 * display:none when inactive. Tab switch calls fit.fit() to correct dimensions.
 *
 * Pure helpers (relativeRecency, reconcileTabs, TabDescriptor, TabReconcileAction)
 * live in shell-helpers.ts — no xterm dependency, directly testable with node --test.
 *
 * LOCALSTORAGE SCHEMA (key "eigen:term:tabs:v1"):
 *   JSON array of TabDescriptor. Versioned key — bump suffix if schema changes.
 *
 * POLL: sidebar polls GET /api/pty + GET /api/forest every 3s to update badges.
 * Interval is cleared on visibilitychange → hidden to avoid background fan-out.
 */

import { newTerminal, connectPty } from "./pty.ts";
import { buildRoster } from "./roster.ts";
import type { RosterRow } from "./roster.ts";
import type { PtyInfo, ForestItem } from "./types.ts";
import {
  relativeRecency,
  reconcileTabs,
  type TabDescriptor,
  type TabReconcileAction,
} from "./shell-helpers.ts";
import { mountPicker } from "./picker.ts";

// Re-export so callers can reach pure helpers via either module.
export { relativeRecency, reconcileTabs };
export type { TabDescriptor, TabReconcileAction };

// ---------------------------------------------------------------------------
// Internal constants
// ---------------------------------------------------------------------------

const LS_KEY = "eigen:term:tabs:v1";
const LS_OVERRIDES = "eigen:term:overrides:v1";

const STATE_COLORS: Record<string, string> = {
  working: "var(--state-working)",
  waiting: "var(--state-waiting)",
  idle: "var(--state-idle)",
  exited: "var(--state-exited)",
};

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
  // Skeleton DOM
  // ------------------------------------------------------------------
  appEl.innerHTML = "";
  appEl.className = "shell";

  const sidebar = el("aside", "sidebar");
  const main = el("div", "main");
  const tabStrip = el("div", "tab-strip");
  const termHost = el("div", "term-host");
  main.append(tabStrip, termHost);
  appEl.append(sidebar, main);

  // ------------------------------------------------------------------
  // Tab registry
  // ------------------------------------------------------------------
  const tabs: TabEntry[] = [];
  let activeTabId: string | null = null;

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
      if (next) activateTab(next.id);
      else renderTabStrip();
    } else {
      renderTabStrip();
    }
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

  // Phase 4 note: full-rebuilds the strip on every call — fine for stateless buttons; switch to patch-in-place if stateful controls like the drawer toggle are added.
  function renderTabStrip() {
    tabStrip.innerHTML = "";
    for (const t of tabs) {
      const tab = el("div", "tab");
      if (t.id === activeTabId) tab.classList.add("tab--active");
      if (t.dead) tab.classList.add("tab--dead");

      const badge = el("span", "tab-badge");
      badge.style.color = STATE_COLORS[t.state] ?? "var(--state-idle)";
      badge.textContent = "●";

      const labelEl = el("span", "tab-label");
      labelEl.textContent = t.descriptor.label;

      const kill = el("button", "tab-kill");
      kill.title = "Kill pty (process terminated)";
      kill.textContent = "☠";
      kill.addEventListener("click", (e) => {
        e.stopPropagation();
        void killTab(t.id);
      });

      const close = el("button", "tab-close");
      close.title = "Detach — close tab, pty stays alive";
      close.textContent = "✕";
      close.addEventListener("click", (e) => {
        e.stopPropagation();
        closeTab(t.id);
      });

      tab.append(badge, labelEl, kill, close);
      tab.addEventListener("click", () => activateTab(t.id));
      tabStrip.append(tab);
    }

    const newBtn = el("button", "tab-new");
    newBtn.textContent = "+";
    newBtn.title = "Open new session (fuzzy launcher)";
    newBtn.addEventListener("click", () => openPicker(newBtn));
    tabStrip.append(newBtn);
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
        // If we opened without a ptyId, update the tab's identity.
        if (entry.id !== id && !desc.ptyId) entry.id = id;
        saveTabs();
        renderTabStrip();
      },
      onSessionUuid(uuid) {
        entry.descriptor = { ...entry.descriptor, uuid };
        saveTabs();
        renderTabStrip();
      },
      onExit() {
        entry.state = "exited";
        entry.dead = true;
        renderTabStrip();
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
        } else {
          // Any other close: mark dead so user can see + manually close.
          entry.dead = true;
          renderTabStrip();
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
  // Picker — overlay triggered by the "+" tab-strip button
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
          openTabWithQuery(query, { label: basename(path) });
        },
        onDismiss() {
          pickerTeardown = null;
        },
      },
    );
  }

  /** Path basename (everything after the last "/"). */
  function basename(p: string): string {
    const i = p.lastIndexOf("/");
    return i >= 0 ? p.slice(i + 1) : p;
  }

  // ------------------------------------------------------------------
  // Sidebar + roster
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

  function renderSidebar(rows: RosterRow[], now: number) {
    // Guard: don't clobber an active inline-rename input.
    if (sidebar.contains(document.activeElement) &&
        document.activeElement instanceof HTMLInputElement &&
        document.activeElement.classList.contains("roster-rename-input")) {
      return;
    }
    sidebar.innerHTML = "";
    const header = el("div", "sidebar-header");
    header.textContent = "sessions";
    sidebar.append(header);

    if (rows.length === 0) {
      const empty = el("div", "sidebar-empty");
      empty.textContent = "no sessions";
      sidebar.append(empty);
      return;
    }

    for (const row of rows) {
      const item = el("div", "roster-row");
      if (!row.live) item.classList.add("roster-row--disk");

      const badge = el("span", "roster-badge");
      badge.style.color = STATE_COLORS[row.state] ?? "var(--state-idle)";
      badge.textContent = "●";
      if (row.state === "waiting") badge.classList.add("roster-badge--waiting");

      const labelEl = el("span", "roster-label");
      labelEl.textContent = row.label;

      const cwdEl = el("span", "roster-cwd");
      cwdEl.textContent = row.cwdChip;

      const recencyEl = el("span", "roster-recency");
      recencyEl.textContent = relativeRecency(row.recency, now);

      item.append(badge, labelEl, cwdEl, recencyEl);

      // Click: open/attach to this session.
      item.addEventListener("click", () => {
        if (row.ptyId) {
          openTabWithQuery(`?attach=${row.ptyId}`, {
            ptyId: row.ptyId,
            uuid: row.uuid,
            label: row.label,
          });
        } else if (row.uuid) {
          openTabWithQuery(`?session=${row.uuid}`, {
            uuid: row.uuid,
            label: row.label,
          });
        }
      });

      // Double-click label → inline rename → localStorage override.
      labelEl.addEventListener("dblclick", (e) => {
        e.stopPropagation();
        const input = document.createElement("input");
        input.className = "roster-rename-input";
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

      sidebar.append(item);
    }
  }

  async function refreshRoster() {
    try {
      const { ptys, forest } = await fetchRosterData();
      const rows = buildRoster(ptys, forest, overrides);
      renderSidebar(rows, Date.now());

      // Update tab state badges from live pty data.
      for (const t of tabs) {
        if (!t.descriptor.ptyId || t.dead) continue;
        const live = ptys.find((p) => p.id === t.descriptor.ptyId);
        if (live) t.state = live.state;
      }
      renderTabStrip();
    } catch {
      // Daemon not reachable — keep stale sidebar.
    }
  }

  // ------------------------------------------------------------------
  // Boot: restore persisted tabs, initial roster, start poll.
  // ------------------------------------------------------------------

  async function boot() {
    renderSidebar([], Date.now());
    renderTabStrip();

    let ptys: PtyInfo[] = [];
    try {
      const data = await fetchRosterData();
      ptys = data.ptys;
      const rows = buildRoster(data.ptys, data.forest, overrides);
      renderSidebar(rows, Date.now());
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
