/**
 * drawer.ts — Transcript drawer: turn cards, SSE live updates, auto-scroll.
 *
 * The drawer is an absolute overlay panel anchored to the right side of .term-host.
 * The pty is NOT resized when the drawer opens (the terminal stays primary; the
 * drawer overlays it). It is mounted/unmounted by the shell's GLOBAL top-right
 * toggle and follows the active tab.
 *
 * Card model (eigen design, Claude Design handoff 2026-06-12):
 *   - Each turn group renders as a user card followed by an assistant card.
 *   - USER turns stay full — they are the navigation spine of the session.
 *   - ASSISTANT turns collapse to a single muted ellipsis line by default;
 *     click the text or the chevron to expand. Tool drill-down rows and system
 *     timing live inside the expanded assistant card; a muted "· N tools" count
 *     in the card head keeps the signal visible while collapsed.
 *   - The leaf group renders as a pulsing "awaiting input…" row.
 *
 * Quick actions (header row):
 *   - interrupt: sends ^C to the active tab's pty via a shell-injected callback
 *     (the drawer knows nothing about sockets).
 *   - copy: copies a plain-text rendering of the transcript to the clipboard.
 *
 * Data flow:
 *   1. On open: fetchSession(uuid) → initial render
 *   2. EventSource /api/watch/:uuid → re-fetch + re-render on unnamed message
 *      events (named "change" listener kept as a safety net, mirroring woland).
 *   3. Auto-scroll to bottom unless the user has scrolled up (stick-to-bottom).
 *
 * Lifecycle:
 *   - mountDrawer() returns a DrawerHandle with a close() method
 *   - close() tears down the EventSource and removes the DOM node
 *   - The caller (shell.ts) must call close() on drawer-close / tab switch
 *
 * Tool drill-down:
 *   Each tool one-liner is clickable; clicking toggles expansion to the
 *   pretty-printed `tool.input` JSON + `tool.output` in a <pre>, plus
 *   `detail.lines` colored spans when present. Expansion state is keyed by
 *   toolExpandKey(group.turnNumber, toolIndex) — stable across SSE re-renders
 *   (the daemon never renumbers exchanges). All content is set via textContent
 *   (never innerHTML) — XSS-safe.
 *
 * Per-turn edit-and-fork:
 *   Each user card with a uuid shows a fork affordance in its head. Clicking
 *   swaps the card text for a textarea (prefilled) plus confirm/cancel.
 *   Esc or cancel restores; confirm POSTs {turn, text} to /api/session/:uuid/fork
 *   and calls onFork(newUuid). While an edit is open, SSE re-renders are
 *   suppressed (editingTurnNumber guard) so the textarea is never clobbered,
 *   and the confirm button is disabled while the fetch is in flight.
 */

import { groupTurns, toolExpandKey } from "./turns.ts";
import type { TurnGroup, Exchange, Tool } from "./turns.ts";
import { icon } from "./icons.ts";

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

/** Shell-injected quick actions. Buttons render only for provided callbacks. */
export interface DrawerActions {
  /** Send an interrupt (^C) to the session's pty. */
  interrupt?: () => void;
}

/**
 * Mount the transcript drawer for `uuid` into `hostEl` (.term-host).
 *
 * @param hostEl   The .term-host element — the drawer overlays it absolutely.
 * @param uuid     Session uuid for /api/session/:uuid/json + /api/watch/:uuid
 * @param onFork   Called with the new session uuid when a fork succeeds.
 *                 Kept decoupled from shell internals — drawer doesn't know what
 *                 openTabWithQuery or refreshRoster are.
 * @param actions  Optional quick-action callbacks (see DrawerActions).
 * @returns        DrawerHandle — caller must call .close() on tab-close or toggle-off
 */
export function mountDrawer(
  hostEl: HTMLElement,
  uuid: string,
  onFork: (newUuid: string) => void = () => {},
  actions: DrawerActions = {},
): DrawerHandle {
  // ------------------------------------------------------------------
  // DOM structure
  // ------------------------------------------------------------------
  const panel = el("div", "drawer");

  const header = el("div", "drawer-header");
  const headerTitle = el("span", "drawer-title");
  headerTitle.textContent = "Transcript";
  const headerCount = el("span", "chip");
  headerCount.textContent = "0 turns";
  header.append(headerTitle, headerCount);

  const actionsRow = el("div", "drawer-actions");
  if (actions.interrupt) {
    const stopBtn = el("button", "drawer-action drawer-action--danger");
    stopBtn.title = "Interrupt (send ^C)";
    stopBtn.append(icon("stop", 13));
    stopBtn.addEventListener("click", () => actions.interrupt!());
    actionsRow.append(stopBtn);
  }
  const copyBtn = el("button", "drawer-action");
  copyBtn.title = "Copy transcript";
  copyBtn.append(icon("copy", 13));
  copyBtn.addEventListener("click", () => void copyTranscript(copyBtn));
  actionsRow.append(copyBtn);

  const body = el("div", "drawer-body scroll");
  panel.append(header, actionsRow, body);
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

  /** Assistant cards expanded by the user, keyed by group turnNumber.
   *  DEFAULT IS COLLAPSED — user turns are the spine, assistant turns ellipse. */
  const expandedAsst = new Set<number>();

  /**
   * Tool expansion state keyed by toolExpandKey(group.turnNumber, toolIndex).
   * Stable across SSE re-renders because group.turnNumber == exchange.n of the
   * opening user turn, which the daemon never renumbers.
   */
  const toolExpanded = new Set<string>();

  /** The most recently rendered groups — used by click handlers and the
   *  transcript copier so an SSE tick between render and click can't serve
   *  stale group data. */
  let currentGroups: TurnGroup[] = [];

  /**
   * When non-null, the turnNumber of the group currently being edited in-place.
   * An SSE re-render that would clobber the textarea is suppressed.
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

    const turnCount = groups.filter((g) => !g.isLeaf).length;
    headerCount.textContent = turnCount === 1 ? "1 turn" : `${turnCount} turns`;

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

    if (group.isLeaf) {
      const row = el("div", "turn-input");
      row.append(el("span", "turn-input-dot"));
      const label = el("span", "turn-input-label");
      label.textContent = "awaiting input…";
      row.append(label);
      wrap.append(row);
      return wrap;
    }

    if (group.userText) {
      wrap.append(renderUserCard(group));
    }
    if (group.assistantText || group.toolExchanges.length > 0 || group.systemText) {
      wrap.append(renderAssistantCard(group));
    }
    return wrap;
  }

  /** USER card — full text, always. The navigation spine. */
  function renderUserCard(group: TurnGroup): HTMLElement {
    const card = el("div", "turn-card turn-card--user");

    const head = el("div", "turn-card-head");
    head.append(el("span", "turn-role-dot"));
    const role = el("span", "turn-role");
    role.textContent = "you";
    head.append(role);

    const num = el("span", "turn-num");
    num.textContent = `${group.turnNumber}`;
    head.append(num);

    const forkBtn = el("button", "drawer-fork-btn");
    forkBtn.append(icon("fork", 13));
    if (group.uuid) {
      forkBtn.title = "Edit and fork from this turn";
      forkBtn.addEventListener("click", (e) => {
        e.stopPropagation();
        openForkEdit(group, textEl, forkBtn);
      });
    } else {
      forkBtn.title = "Fork unavailable — no uuid on this turn";
      forkBtn.disabled = true;
      forkBtn.classList.add("drawer-fork-btn--disabled");
    }
    head.append(forkBtn);

    const textEl = el("p", "turn-text");
    textEl.textContent = group.userText;

    card.append(head, textEl);
    return card;
  }

  /** ASSISTANT card — one-line ellipsis until expanded. */
  function renderAssistantCard(group: TurnGroup): HTMLElement {
    const isExpanded = expandedAsst.has(group.turnNumber);
    const card = el(
      "div",
      `turn-card turn-card--assistant${isExpanded ? "" : " turn-card--collapsed"}`,
    );

    const head = el("div", "turn-card-head");
    head.append(el("span", "turn-role-dot"));
    const role = el("span", "turn-role");
    role.textContent = "assistant";
    head.append(role);

    if (group.toolExchanges.length > 0) {
      const toolCount = el("span", "turn-num");
      toolCount.textContent =
        group.toolExchanges.length === 1 ? "· 1 tool" : `· ${group.toolExchanges.length} tools`;
      toolCount.style.marginLeft = "0";
      head.append(toolCount);
    }

    const spacer = el("span");
    spacer.style.flex = "1";
    head.append(spacer);

    const toggle = el("button", "turn-toggle");
    toggle.title = isExpanded ? "Collapse" : "Expand";
    toggle.append(icon("chevron", 13));
    toggle.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleAsst(group.turnNumber);
    });
    head.append(toggle);

    const textEl = el("p", "turn-text");
    textEl.textContent = group.assistantText || "(tool calls only)";
    if (!isExpanded) {
      textEl.addEventListener("click", () => toggleAsst(group.turnNumber));
    }

    card.append(head, textEl);

    if (isExpanded) {
      group.toolExchanges.forEach((ex, idx) => {
        card.append(renderToolRow(ex.tool!, group.turnNumber, idx));
      });
      if (group.systemText) {
        const sys = el("div", "drawer-system");
        sys.textContent = group.systemText;
        card.append(sys);
      }
    }

    return card;
  }

  function toggleAsst(turnNumber: number) {
    if (editingTurnNumber !== null) return;
    if (expandedAsst.has(turnNumber)) {
      expandedAsst.delete(turnNumber);
    } else {
      expandedAsst.add(turnNumber);
    }
    render(currentGroups);
  }

  /**
   * Open the inline fork-edit UI for a user card.
   *
   * Replaces the card's text with a textarea (prefilled) + confirm/cancel.
   * Sets `editingTurnNumber` to suppress SSE re-renders while editing.
   * On cancel or confirm (success or failure): clears `editingTurnNumber` and
   * calls render(currentGroups) to restore the normal view.
   *
   * @param group    The TurnGroup being forked (must have group.uuid set — caller guards).
   * @param textEl   The card's text node to swap out for the edit widget.
   * @param forkBtn  The fork button — hidden while editing to avoid confusion.
   */
  function openForkEdit(
    group: TurnGroup,
    textEl: HTMLElement,
    forkBtn: HTMLButtonElement,
  ) {
    // Only one edit open at a time.
    if (editingTurnNumber !== null) return;
    editingTurnNumber = group.turnNumber;

    // Hide the fork button while editing.
    forkBtn.style.display = "none";

    // Build the edit widget in place of the card text.
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

    textEl.replaceWith(editWrap);

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
  // Copy transcript — plain-text rendering of the current groups
  // ------------------------------------------------------------------

  async function copyTranscript(btn: HTMLButtonElement) {
    const parts: string[] = [];
    for (const g of currentGroups) {
      if (g.isLeaf) continue;
      if (g.userText) parts.push(`## turn ${g.turnNumber} — user\n${g.userText}`);
      if (g.assistantText) parts.push(`assistant:\n${g.assistantText}`);
      for (const ex of g.toolExchanges) {
        if (ex.tool) parts.push(`[tool] ${ex.tool.kind} ${ex.tool.arg}`);
      }
    }
    try {
      await navigator.clipboard.writeText(parts.join("\n\n"));
      btn.classList.add("drawer-action--flash");
      setTimeout(() => btn.classList.remove("drawer-action--flash"), 900);
    } catch {
      // Clipboard unavailable (permissions / non-secure context) — no-op.
    }
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
