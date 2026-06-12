/**
 * drawer.ts — Transcript drawer: grouped turns, SSE live updates, auto-scroll.
 *
 * The drawer is an absolute overlay panel anchored to the right side of .term-host.
 * It slides in/out via CSS transform — the pty is NOT resized when the drawer opens
 * (the terminal stays primary; the drawer overlays it).
 *
 * Data flow:
 *   1. On open: fetchSession(uuid) → initial render
 *   2. EventSource /api/watch/:uuid → re-fetch + re-render on unnamed message events
 *      (daemon emits unnamed SSE events; onmessage handles them).
 *      A named "change" listener is also registered as a safety-net mirror of
 *      woland's followManuscript, in case the daemon is updated to name its events.
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
 *
 * Tool drill-down (Task 4.3):
 *   Each tool one-liner is clickable; clicking toggles expansion. Expanded state
 *   shows pretty-printed `tool.input` JSON + `tool.output` in a <pre>, plus
 *   `detail.lines` colored spans when present. Truncation notices are shown when
 *   `truncated` or `inputTruncated` are set.
 *
 *   Expansion state is keyed by `${group.turnNumber}:${toolIndex}`.
 *   `group.turnNumber` is the exchange `n` of the group's opening user turn —
 *   stable for the lifetime of a session (the daemon never renumbers exchanges).
 *   A new group appearing above shifts no existing turnNumbers, so the key is
 *   stable across SSE re-renders.
 *
 *   Input is rendered via JSON.stringify(input, null, 2) inside a <pre> with
 *   CSS max-height + overflow-y: auto — no JS cap beyond the server-side 50 KB.
 *   Output is set via textContent (not innerHTML) — XSS-safe for both input and
 *   output paths.
 *
 * Per-turn edit-and-fork (Task 4.4):
 *   Each non-leaf user turn with a uuid shows a fork button (✂) in the header.
 *   Clicking it swaps the summary span for a textarea (prefilled with group.userText)
 *   plus confirm/cancel buttons. Esc or cancel restores the summary. Confirm POSTs
 *   `{turn: group.uuid, text}` to /api/session/:uuid/fork and calls onFork(newUuid).
 *
 *   SSE re-render guard: an active edit is tracked by `editingTurnNumber`. When a
 *   re-render fires while an edit is in progress, the full render() call is
 *   suppressed — the entire drawer body is left intact to avoid clobbering the
 *   textarea. (Same concept as shell.ts's roster-rename-input guard, which also
 *   freezes the whole sidebar while a rename is active.)
 *
 *   Double-submit guard: the confirm button is disabled while a fetch is in flight.
 *   Failure shows an inline error message; the source session is never mutated.
 */

import { groupTurns, toolExpandKey } from "./turns.ts";
import type { TurnGroup, Exchange, Tool } from "./turns.ts";

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

/**
 * POST /api/session/:uuid/fork with {turn, text}.
 * Returns the new session uuid on success, null on failure.
 * Mirrors woland's forkSession in web/src/data.ts exactly.
 */
async function forkSession(
  srcUuid: string,
  turn: string,
  text: string,
): Promise<string | null> {
  try {
    const res = await fetch(`/api/session/${encodeURIComponent(srcUuid)}/fork`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ turn, text }),
    });
    if (!res.ok) return null;
    const j = (await res.json()) as { uuid?: string };
    return typeof j.uuid === "string" ? j.uuid : null;
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
 * @param onFork   Called with the new session uuid when a fork succeeds.
 *                 Kept decoupled from shell internals — drawer doesn't know what
 *                 openTabWithQuery or refreshRoster are.
 * @returns        DrawerHandle — caller must call .close() on tab-close or toggle-off
 */
export function mountDrawer(
  hostEl: HTMLElement,
  uuid: string,
  onFork: (newUuid: string) => void = () => {},
): DrawerHandle {
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

  /**
   * Tool expansion state keyed by toolExpandKey(group.turnNumber, toolIndex).
   * Stable across SSE re-renders because group.turnNumber == exchange.n of the
   * opening user turn, which the daemon never renumbers.
   */
  const toolExpanded = new Set<string>();

  /** The most recently rendered groups — used by fold click handlers to avoid
   *  rendering stale group data when an SSE tick arrives between render and click.
   */
  let currentGroups: TurnGroup[] = [];

  /**
   * When non-null, the turnNumber of the group currently being edited in-place.
   * An SSE re-render that would clobber the textarea is suppressed for that group.
   * Set when the fork textarea opens; cleared on cancel or confirm.
   */
  let editingTurnNumber: number | null = null;

  function render(groups: TurnGroup[]) {
    currentGroups = groups;

    // Guard: if an edit is in progress, skip the full re-render to avoid clobbering
    // the textarea. The edited group will re-render after cancel or confirm resolves.
    if (editingTurnNumber !== null) return;

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
      // Re-render the full groups list so fold state is applied consistently
      // and no stale group data from a previous SSE tick is shown.
      render(currentGroups);
    });

    headerRow.append(foldBtn, numEl, summaryEl);

    // Fork button — only on non-leaf user turns with a uuid.
    // Disabled if no uuid (e.g. unsent leaf or live turn not yet committed).
    if (!group.isLeaf && group.userText !== "") {
      const forkBtn = el("button", "drawer-fork-btn");
      forkBtn.textContent = "✂";
      if (group.uuid) {
        forkBtn.title = "Edit and fork from this turn";
        forkBtn.addEventListener("click", (e) => {
          e.stopPropagation();
          openForkEdit(group, summaryEl, forkBtn, headerRow);
        });
      } else {
        forkBtn.title = "Fork unavailable — no uuid on this turn";
        forkBtn.disabled = true;
        forkBtn.classList.add("drawer-fork-btn--disabled");
      }
      headerRow.append(forkBtn);
    }

    // Click on header row (not the buttons) also toggles fold.
    headerRow.addEventListener("click", (e) => {
      if (editingTurnNumber !== null) return;
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

      // Tool rows with drill-down expansion
      group.toolExchanges.forEach((ex, idx) => {
        contentEl.append(renderToolRow(ex.tool!, group.turnNumber, idx));
      });

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
   * Open the inline fork-edit UI for a group.
   *
   * Replaces the `summaryEl` span with a textarea (prefilled) + confirm/cancel.
   * Sets `editingTurnNumber` to suppress SSE re-renders while editing.
   * On cancel or confirm (success or failure): clears `editingTurnNumber` and
   * calls render(currentGroups) to restore the normal view.
   *
   * @param group      The TurnGroup being forked (must have group.uuid set — caller guards).
   * @param summaryEl  The summary span to swap out for the edit widget.
   * @param forkBtn    The fork button — hidden while editing to avoid confusion.
   * @param headerRow  The header row — click-to-fold is inhibited while editing.
   */
  function openForkEdit(
    group: TurnGroup,
    summaryEl: HTMLElement,
    forkBtn: HTMLButtonElement,
    headerRow: HTMLElement,
  ) {
    // Only one edit open at a time (clicking fork on another group while editing
    // is blocked because SSE render suppresses the full rebuild, and the fork
    // button on other groups still works — but we guard via editingTurnNumber).
    if (editingTurnNumber !== null) return;
    editingTurnNumber = group.turnNumber;

    // Hide the fork button while editing.
    forkBtn.style.display = "none";

    // Disable the fold-toggle click while editing.
    headerRow.style.cursor = "default";

    // Build the edit widget in place of summaryEl.
    const editWrap = el("div", "drawer-fork-edit");

    const textarea = el("textarea", "drawer-fork-textarea");
    textarea.value = group.userText;
    textarea.rows = Math.min(8, group.userText.split("\n").length + 1);
    editWrap.append(textarea);

    const controls = el("div", "drawer-fork-controls");
    const confirmBtn = el("button", "drawer-fork-confirm");
    confirmBtn.textContent = "fork";
    const cancelBtn = el("button", "drawer-fork-cancel");
    cancelBtn.textContent = "cancel";
    const errorEl = el("span", "drawer-fork-error");
    controls.append(confirmBtn, cancelBtn, errorEl);
    editWrap.append(controls);

    summaryEl.replaceWith(editWrap);

    // Focus + select all for immediate editing.
    textarea.focus();
    textarea.select();

    function cancel() {
      editingTurnNumber = null;
      render(currentGroups);
    }

    async function confirm() {
      const text = textarea.value.trim();
      if (!text) {
        errorEl.textContent = "text cannot be empty";
        return;
      }
      confirmBtn.disabled = true;
      errorEl.textContent = "";
      const newUuid = await forkSession(uuid, group.uuid!, text);
      if (newUuid === null) {
        confirmBtn.disabled = false;
        errorEl.textContent = "fork failed — see daemon logs";
        return;
      }
      editingTurnNumber = null;
      onFork(newUuid);
      render(currentGroups);
    }

    cancelBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      cancel();
    });

    confirmBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      void confirm();
    });

    // Esc key cancels; Ctrl+Enter confirms.
    textarea.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        cancel();
      } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        void confirm();
      }
    });

    // Prevent header fold toggle from firing while clicking inside the edit widget.
    editWrap.addEventListener("click", (e) => {
      e.stopPropagation();
    });
  }

  /**
   * Render a tool exchange row with drill-down expansion.
   *
   * Collapsed: kind badge + arg one-liner.
   * Expanded: pretty-printed tool.input JSON + tool.output, with detail.lines
   *   colored spans when present (classes dim/add/rem/cool mirror woland's
   *   .tool .body .ln.* conventions; webterm-local equivalents in style.css).
   *   Truncation notices are shown when truncated/inputTruncated are set.
   *
   * XSS safety: all user-controlled content is set via textContent (never innerHTML).
   * Input JSON: rendered via JSON.stringify(input, null, 2) inside a <pre> with
   *   CSS max-height + overflow-y: auto — no extra JS cap needed beyond the
   *   server-side 50 KB limit.
   *
   * @param tool       The Tool object from the exchange.
   * @param turnNumber The group's turnNumber (stable key component).
   * @param toolIndex  Index within group.toolExchanges (stable for a given SSE frame).
   */
  function renderToolRow(tool: Tool, turnNumber: number, toolIndex: number): HTMLElement {
    const key = toolExpandKey(turnNumber, toolIndex);
    const isExpanded = toolExpanded.has(key);

    const wrap = el("div", "drawer-tool-wrap");

    // One-liner header (always visible)
    const row = el("div", "drawer-tool-row");
    row.style.cursor = "pointer";
    const kindEl = el("span", "drawer-tool-kind");
    kindEl.textContent = tool.kind;
    const argEl = el("span", "drawer-tool-arg");
    argEl.textContent = tool.arg;
    const chevron = el("span", "drawer-tool-chevron");
    chevron.textContent = isExpanded ? "▾" : "▸";
    row.append(chevron, kindEl, argEl);
    wrap.append(row);

    // Toggle expansion on click
    row.addEventListener("click", () => {
      if (toolExpanded.has(key)) {
        toolExpanded.delete(key);
      } else {
        toolExpanded.add(key);
      }
      render(currentGroups);
    });

    if (isExpanded) {
      const detail = el("div", "drawer-tool-detail");

      // --- Input section ---
      if (tool.input !== undefined) {
        const inputLabel = el("div", "drawer-tool-detail-label");
        inputLabel.textContent = tool.inputTruncated === true
          ? "input (truncated at 50 KB)"
          : "input";
        detail.append(inputLabel);

        const inputPre = el("pre", "drawer-tool-detail-pre");
        inputPre.textContent = JSON.stringify(tool.input, null, 2);
        detail.append(inputPre);
      }

      // --- Output section ---
      // Non-empty detail.lines takes precedence over raw output when present, as it
      // carries color annotations.
      if (tool.detail && tool.detail.lines.length > 0) {
        const outputLabel = el("div", "drawer-tool-detail-label");
        outputLabel.textContent = tool.truncated === true
          ? "output (truncated at 50 KB)"
          : "output";
        detail.append(outputLabel);

        const pre = el("pre", "drawer-tool-detail-pre");
        for (const line of tool.detail.lines) {
          const span = el("span", `drawer-tool-ln drawer-tool-ln--${line.c}`);
          span.textContent = line.t + "\n";
          pre.append(span);
        }
        detail.append(pre);
      } else if (tool.output !== undefined && tool.output !== "") {
        const outputLabel = el("div", "drawer-tool-detail-label");
        outputLabel.textContent = tool.truncated === true
          ? "output (truncated at 50 KB)"
          : "output";
        detail.append(outputLabel);

        const outputPre = el("pre", "drawer-tool-detail-pre");
        outputPre.textContent = tool.output;
        detail.append(outputPre);
      } else if (tool.truncated === true) {
        // output was present but got trimmed to empty — show notice only
        const notice = el("div", "drawer-tool-detail-label");
        notice.textContent = "output (truncated at 50 KB)";
        detail.append(notice);
      }

      wrap.append(detail);
    }

    return wrap;
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
  // The daemon sends unnamed SSE events; onmessage handles them.
  // The named "change" listener is a safety net in case the daemon is updated
  // to emit named events (mirrors woland's followManuscript pattern).
  es.onmessage = () => void load();
  es.addEventListener("change", () => void load());
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
