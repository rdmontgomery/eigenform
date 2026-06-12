/**
 * drawer.ts — Transcript drawer: grouped turns, SSE live updates, auto-scroll.
 *
 * The drawer is an absolute overlay panel anchored to the right side of .term-host.
 * It slides in/out via CSS transform — the pty is NOT resized when the drawer opens
 * (the terminal stays primary; the drawer overlays it).
 *
 * Data flow:
 *   1. On open: fetchSession(uuid) → initial render
 *   2. EventSource /api/watch/:uuid → re-fetch + re-render on "change" events
 *   3. Auto-scroll to bottom unless the user has scrolled up (stick-to-bottom rule)
 *
 * Lifecycle:
 *   - mountDrawer() returns a DrawerHandle with a close() method
 *   - close() tears down the EventSource and removes the DOM node
 *   - The caller (shell.ts) must call close() on both drawer-close and tab-close
 *
 * Toggle-placement rationale:
 *   The drawer toggle button lives inside the tab strip entry (rendered by
 *   renderTabStrip in shell.ts). Because renderTabStrip does a full innerHTML
 *   rebuild every 3 s, the button's appearance is re-derived from TabEntry.drawerOpen
 *   on each rebuild — stateless from the DOM's perspective, stateful in the model.
 *   This avoids both the "stale button after rebuild" problem AND the need to move
 *   the toggle outside the strip. See shell.ts: TabEntry gains a `drawerOpen` flag
 *   and a `drawerHandle` field.
 */

import { groupTurns } from "./turns.ts";
import type { TurnGroup, Exchange } from "./turns.ts";

// ---------------------------------------------------------------------------
// Minimal session fetch (mirrors web/src/data.ts fetchSession — do not import
// from web/ to keep the two apps independent at the module boundary)
// ---------------------------------------------------------------------------

interface SessionPayload {
  id: string;
  total: number;
  branches: number;
  windowStart: number;
  exchanges: Exchange[];
}

async function fetchSession(uuid: string): Promise<SessionPayload | null> {
  try {
    const res = await fetch(`/api/session/${encodeURIComponent(uuid)}/json`);
    if (!res.ok) return null;
    return (await res.json()) as SessionPayload;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface DrawerHandle {
  /** Tear down EventSource + remove DOM. Safe to call multiple times. */
  close(): void;
}

/**
 * Mount the transcript drawer for `uuid` into `hostEl` (.term-host).
 *
 * @param hostEl   The .term-host element — the drawer overlays it absolutely.
 * @param uuid     Session uuid for /api/session/:uuid/json + /api/watch/:uuid
 * @returns        DrawerHandle — caller must call .close() on tab-close or toggle-off
 */
export function mountDrawer(hostEl: HTMLElement, uuid: string): DrawerHandle {
  // ------------------------------------------------------------------
  // DOM structure
  // ------------------------------------------------------------------
  const panel = el("div", "drawer");
  const header = el("div", "drawer-header");
  const headerTitle = el("span", "drawer-title");
  headerTitle.textContent = "transcript";
  header.append(headerTitle);

  const body = el("div", "drawer-body");
  panel.append(header, body);
  hostEl.append(panel);

  // ------------------------------------------------------------------
  // Scroll-stick logic
  // ------------------------------------------------------------------
  const STICK_THRESHOLD = 60; // px from bottom — if closer, re-stick after update

  /** True when the user is NOT scrolled up (i.e. auto-scroll should re-stick). */
  function isNearBottom(): boolean {
    return body.scrollHeight - body.scrollTop - body.clientHeight < STICK_THRESHOLD;
  }

  function scrollToBottom() {
    body.scrollTop = body.scrollHeight;
  }

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------

  /** Fold state keyed by group turnNumber. Default: all open. */
  const folded = new Set<number>();

  function render(groups: TurnGroup[]) {
    const wasNearBottom = isNearBottom();
    const prevScrollTop = body.scrollTop;

    body.innerHTML = "";

    if (groups.length === 0) {
      const empty = el("div", "drawer-empty");
      empty.textContent = "no exchanges yet";
      body.append(empty);
    } else {
      for (const group of groups) {
        body.append(renderGroup(group));
      }
    }

    // Restore scroll position: stick if near bottom, preserve if scrolled up.
    if (wasNearBottom) {
      scrollToBottom();
    } else {
      body.scrollTop = prevScrollTop;
    }
  }

  function renderGroup(group: TurnGroup): HTMLElement {
    const wrap = el("div", "drawer-group");
    if (group.isLeaf) wrap.classList.add("drawer-group--leaf");

    // Header row: turn number + first line of user text (or "assistant" if none)
    const headerRow = el("div", "drawer-group-header");
    const numEl = el("span", "drawer-group-num");
    numEl.textContent = `${group.turnNumber}`;

    const summaryEl = el("span", "drawer-group-summary");
    if (group.isLeaf) {
      summaryEl.textContent = "[ input ]";
    } else {
      const firstLine = group.userText.split("\n")[0]?.trim() ?? "";
      summaryEl.textContent = firstLine || "(assistant)";
    }

    const isFolded = folded.has(group.turnNumber);
    const foldBtn = el("button", "drawer-fold-btn");
    foldBtn.textContent = isFolded ? "▶" : "▼";
    foldBtn.title = isFolded ? "expand" : "collapse";
    foldBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      if (folded.has(group.turnNumber)) {
        folded.delete(group.turnNumber);
      } else {
        folded.add(group.turnNumber);
      }
      // Re-render just this group in-place.
      const fresh = renderGroup(group);
      wrap.replaceWith(fresh);
    });

    headerRow.append(foldBtn, numEl, summaryEl);

    // Click on header row (not the button) also toggles fold.
    headerRow.addEventListener("click", (e) => {
      if (e.target === foldBtn) return;
      foldBtn.click();
    });
    headerRow.style.cursor = "pointer";

    wrap.append(headerRow);

    if (!isFolded && !group.isLeaf) {
      const contentEl = el("div", "drawer-group-content");

      // User text (left-border accent)
      if (group.userText) {
        const userEl = el("div", "drawer-user");
        userEl.textContent = group.userText;
        contentEl.append(userEl);
      }

      // Assistant text (plain text v1; markdown is backlog)
      if (group.assistantText) {
        const asst = el("div", "drawer-assistant");
        asst.textContent = group.assistantText;
        contentEl.append(asst);
      }

      // Tool one-liners (Task 4.3 will expand these — leave a clean seam)
      for (const ex of group.toolExchanges) {
        contentEl.append(renderToolRow(ex.tool!));
      }

      // System timing
      if (group.systemText) {
        const sys = el("div", "drawer-system");
        sys.textContent = group.systemText;
        contentEl.append(sys);
      }

      wrap.append(contentEl);
    }

    return wrap;
  }

  /**
   * Render a tool exchange as a one-liner.
   * Clean seam for Task 4.3: a future drill-down replaces this function in-place.
   */
  function renderToolRow(tool: NonNullable<Exchange["tool"]>): HTMLElement {
    const row = el("div", "drawer-tool-row");
    const kindEl = el("span", "drawer-tool-kind");
    kindEl.textContent = tool.kind;
    const argEl = el("span", "drawer-tool-arg");
    argEl.textContent = tool.arg;
    row.append(kindEl, argEl);
    return row;
  }

  // ------------------------------------------------------------------
  // Data: initial fetch + SSE
  // ------------------------------------------------------------------

  let es: EventSource | null = null;
  let closed = false;

  async function load() {
    if (closed) return;
    const session = await fetchSession(uuid);
    if (closed) return; // may have been closed while fetching
    if (session) {
      render(groupTurns(session.exchanges));
    }
  }

  void load();

  es = new EventSource(`/api/watch/${encodeURIComponent(uuid)}`);
  es.onmessage = () => void load();
  es.onerror = () => {
    // EventSource will auto-reconnect; no action needed here.
  };

  // ------------------------------------------------------------------
  // Handle (teardown)
  // ------------------------------------------------------------------

  return {
    close() {
      if (closed) return;
      closed = true;
      es?.close();
      es = null;
      panel.remove();
    },
  };
}

// ---------------------------------------------------------------------------
// DOM utility (local — same pattern as shell.ts)
// ---------------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}
