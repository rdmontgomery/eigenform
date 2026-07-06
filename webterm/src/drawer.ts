/**
 * drawer.ts — Transcript drawer: turn cards, per-type tool calls, SSE live
 * updates, auto-scroll, outliner-style expand/collapse.
 *
 * The drawer is an absolute overlay panel anchored to the right side of .term-host.
 * The pty is NOT resized when the drawer opens (the terminal stays primary; the
 * drawer overlays it). It is mounted/unmounted by the shell's GLOBAL top-right
 * toggle and follows the active tab.
 *
 * Card model (eigenform design, tool-call pass 2026-06-12):
 *   - Each turn group renders as a user card followed by an assistant card.
 *   - USER turns default expanded (the navigation spine) but are collapsible.
 *   - ASSISTANT turns default to a single muted ellipsis line — the prose's
 *     first line, or a tool-verb summary ("Skill · Bash") when there is no
 *     prose. Click the text or the chevron to expand.
 *   - Tool calls render as scannable per-type rows (toolview.ts):
 *     [tinted icon] Verb · headline … [+N −M | count] [⌄⌄] [▸], expanding to
 *     structured I/O (bash command/output, edit mini-diff, read info line,
 *     grep matches, todo checklist). Unknown kinds keep the raw input/output
 *     drill-down so nothing is hidden.
 *
 * Outliner controls — "expand/collapse all below" at three levels:
 *   - Header: "⌄⌄ N turns" toggles the whole transcript (turns + tool I/O).
 *   - Turn (hover, double chevron): that turn and every turn below it.
 *   - Tool (hover, double chevron): that tool and the tools below it in the turn.
 *   The double chevron's direction is computed live: if everything below is
 *   already open it collapses (icon rotates 180°), otherwise it expands.
 *
 * Collapse state PERSISTS per session: { turnCol, toolMap } in localStorage
 * under "eigenform:term:drawerstate:<uuid>". turnCol is keyed "u:<n>" / "a:<n>"
 * (true = collapsed); toolMap is keyed toolExpandKey(n, i) (true = open).
 * Defaults (absent key): user expanded, assistant collapsed, tool closed.
 * The drawer remounts per session switch (shell's syncDrawer), so state loads
 * once in mountDrawer — no cross-session swap logic needed here.
 *
 * Quick actions (header row):
 *   - interrupt: sends ^C to the active tab's pty via a shell-injected callback.
 *   - copy: copies a plain-text rendering of the transcript to the clipboard.
 *
 * Data flow:
 *   1. On open: fetchSession(uuid) → initial render
 *   2. subscribeWatch(uuid) → re-fetch + re-render on each write, via the shared
 *      watch hub (one EventSource per uuid across the reach map + drawer).
 *   3. Auto-scroll to bottom unless the user has scrolled up (stick-to-bottom).
 *
 * Per-turn edit-and-fork:
 *   Each user card with a uuid shows a fork affordance in its head. Clicking
 *   swaps the card text for a textarea (prefilled) plus confirm/cancel.
 *   While an edit is open, SSE re-renders and batch collapse ops are
 *   suppressed (editingTurnNumber guard) so the textarea is never clobbered.
 *
 * XSS safety: all user/tool content is set via textContent, never innerHTML.
 */

import { groupTurns, toolExpandKey } from "./turns.ts";
import type { TurnGroup, Exchange, Tool } from "./turns.ts";
import { toolView, toolsSummary } from "./toolview.ts";
import type { ToolView } from "./toolview.ts";
import { icon } from "./icons.ts";
import { subscribeWatch } from "./watch.ts";

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
  /** Unsubscribe from the watch hub + remove DOM. Safe to call multiple times. */
  close(): void;
}

/** Shell-injected quick actions. Buttons render only for provided callbacks. */
export interface DrawerActions {
  /** Send an interrupt (^C) to the session's pty. */
  interrupt?: () => void;
  /** Subscribe to /api/watch for live updates. Defaults to true. Set false for a
   *  static snapshot (e.g. the forest preview float, which re-fetches on focus
   *  rather than tailing). */
  live?: boolean;
}

/**
 * Mount the transcript drawer for `uuid` into `hostEl` (.term-host).
 *
 * @param hostEl   The .term-host element — the drawer overlays it absolutely.
 * @param uuid     Session uuid for /api/session/:uuid/json + /api/watch/:uuid
 * @param onFork   Called with the new session uuid and the edited prompt text when a
 *                 fork succeeds. The daemon never writes `text` into the branch file
 *                 (the fork must end on a completed turn to be resumable) — the caller
 *                 stages it into the resumed branch's input via seedInput, unsent.
 *                 Kept decoupled from shell internals — drawer doesn't know what
 *                 openTabWithQuery or refreshRoster are.
 * @param actions  Optional quick-action callbacks (see DrawerActions).
 * @returns        DrawerHandle — caller must call .close() on tab-close or toggle-off
 */
export function mountDrawer(
  hostEl: HTMLElement,
  uuid: string,
  onFork: (newUuid: string, text: string) => void = () => {},
  actions: DrawerActions = {},
): DrawerHandle {
  // ------------------------------------------------------------------
  // DOM structure
  // ------------------------------------------------------------------
  const panel = el("div", "drawer");

  const header = el("div", "drawer-header");
  const headerTitle = el("span", "drawer-title");
  headerTitle.textContent = "Transcript";
  const expandAllBtn = el("button", "drawer-expandall");
  expandAllBtn.append(icon("chevrons", 13));
  const expandAllCount = el("span", "drawer-expandall-count");
  expandAllBtn.append(expandAllCount);
  header.append(headerTitle, expandAllBtn);

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
  // Collapse state — persisted per session
  // ------------------------------------------------------------------

  const STORE_KEY = `eigenform:term:drawerstate:${uuid}`;

  /** true = collapsed. Keys "u:<n>" (user card) / "a:<n>" (assistant card). */
  let turnCol: Record<string, boolean> = {};
  /** true = open. Keys toolExpandKey(n, i). */
  let toolMap: Record<string, boolean> = {};
  try {
    const raw = localStorage.getItem(STORE_KEY);
    if (raw) {
      const s = JSON.parse(raw) as { turnCol?: Record<string, boolean>; toolMap?: Record<string, boolean> };
      turnCol = s.turnCol ?? {};
      toolMap = s.toolMap ?? {};
    }
  } catch {
    // Corrupt entry — start from defaults.
  }

  function persist() {
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify({ turnCol, toolMap }));
    } catch {
      // Quota/private-mode — collapse state just won't survive reloads.
    }
  }

  // Defaults when the key is absent: user expanded, assistant collapsed, tool closed.
  const userCollapsed = (n: number) => turnCol[`u:${n}`] ?? false;
  const asstCollapsed = (n: number) => turnCol[`a:${n}`] ?? true;
  const toolOpen = (key: string) => toolMap[key] ?? false;

  /** The most recently rendered groups — click handlers and the copier read
   *  this so an SSE tick between render and click can't serve stale data. */
  let currentGroups: TurnGroup[] = [];

  /**
   * When non-null, the turnNumber of the group currently being edited in-place.
   * SSE re-renders and batch collapse ops are suppressed while editing.
   */
  let editingTurnNumber: number | null = null;

  // --- range ops (the "all below" semantics) --------------------------------

  /** Has the group an assistant card to collapse? */
  function hasAsstCard(g: TurnGroup): boolean {
    return !!g.assistantText || g.toolExchanges.length > 0 || !!g.systemText;
  }

  /** True when every card and tool from group index `fromIdx` onward is open. */
  function rangeExpanded(fromIdx: number): boolean {
    return currentGroups.slice(fromIdx).every((g) => {
      if (g.isLeaf) return true;
      if (g.userText && userCollapsed(g.turnNumber)) return false;
      if (hasAsstCard(g) && asstCollapsed(g.turnNumber)) return false;
      return g.toolExchanges.every((_, i) => toolOpen(toolExpandKey(g.turnNumber, i)));
    });
  }

  /** Expand/collapse every card and tool from group index `fromIdx` onward. */
  function setRange(fromIdx: number, expand: boolean) {
    if (editingTurnNumber !== null) return;
    for (const g of currentGroups.slice(fromIdx)) {
      if (g.isLeaf) continue;
      if (g.userText) turnCol[`u:${g.turnNumber}`] = !expand;
      if (hasAsstCard(g)) turnCol[`a:${g.turnNumber}`] = !expand;
      g.toolExchanges.forEach((_, i) => {
        toolMap[toolExpandKey(g.turnNumber, i)] = expand;
      });
    }
    persist();
    render(currentGroups);
  }

  /** True when every tool from index `fromI` onward in `g` is open. */
  function toolsRangeOpen(g: TurnGroup, fromI: number): boolean {
    return g.toolExchanges.slice(fromI).every((_, k) => toolOpen(toolExpandKey(g.turnNumber, fromI + k)));
  }

  function setToolsRange(g: TurnGroup, fromI: number, open: boolean) {
    if (editingTurnNumber !== null) return;
    g.toolExchanges.forEach((_, i) => {
      if (i >= fromI) toolMap[toolExpandKey(g.turnNumber, i)] = open;
    });
    persist();
    render(currentGroups);
  }

  expandAllBtn.addEventListener("click", () => setRange(0, !rangeExpanded(0)));

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------

  function render(groups: TurnGroup[]) {
    currentGroups = groups;

    // Guard: if an edit is in progress, skip the full re-render to avoid clobbering
    // the textarea. The edited group will re-render after cancel or confirm resolves.
    if (editingTurnNumber !== null) return;

    const wasNearBottom = isNearBottom();
    const prevScrollTop = body.scrollTop;

    const turnCount = groups.filter((g) => !g.isLeaf).length;
    expandAllCount.textContent = turnCount === 1 ? "1 turn" : `${turnCount} turns`;
    const allOpen = rangeExpanded(0);
    expandAllBtn.title = allOpen ? "Collapse all" : "Expand all";
    expandAllBtn.classList.toggle("drawer-expandall--open", allOpen);

    body.innerHTML = "";

    if (groups.length === 0) {
      const empty = el("div", "drawer-empty");
      empty.textContent = "no exchanges yet";
      body.append(empty);
    } else {
      groups.forEach((group, idx) => {
        body.append(renderGroup(group, idx));
      });
    }

    // Restore scroll position: stick if near bottom, preserve if scrolled up.
    if (wasNearBottom) {
      scrollToBottom();
    } else {
      body.scrollTop = prevScrollTop;
    }
  }

  function renderGroup(group: TurnGroup, idx: number): HTMLElement {
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
      wrap.append(renderUserCard(group, idx));
    }
    if (hasAsstCard(group)) {
      wrap.append(renderAssistantCard(group, idx));
    }
    return wrap;
  }

  /** Hover-revealed double chevron: expand/collapse this node + all below. */
  function belowBtn(open: boolean, title: string, onClick: () => void): HTMLButtonElement {
    const btn = el("button", `below-btn${open ? " below-btn--open" : ""}`);
    btn.title = title;
    btn.append(icon("chevrons", 13));
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      onClick();
    });
    return btn;
  }

  /** Single chevron toggling one card. */
  function toggleBtn(collapsed: boolean, onClick: () => void): HTMLButtonElement {
    const btn = el("button", "turn-toggle");
    btn.title = collapsed ? "Expand" : "Collapse";
    btn.append(icon("chevron", 13));
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      onClick();
    });
    return btn;
  }

  function toggleCard(key: string, collapsedNow: boolean) {
    if (editingTurnNumber !== null) return;
    turnCol[key] = !collapsedNow;
    persist();
    render(currentGroups);
  }

  /** USER card — full text by default; collapsible to a one-line clamp. */
  function renderUserCard(group: TurnGroup, idx: number): HTMLElement {
    const collapsed = userCollapsed(group.turnNumber);
    const card = el("div", `turn-card turn-card--user${collapsed ? " turn-card--collapsed" : ""}`);

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

    head.append(
      belowBtn(rangeExpanded(idx), rangeExpanded(idx) ? "Collapse all below" : "Expand all below",
        () => setRange(idx, !rangeExpanded(idx))),
      toggleBtn(collapsed, () => toggleCard(`u:${group.turnNumber}`, collapsed)),
    );

    const textEl = el("p", "turn-text");
    textEl.textContent = group.userText;
    if (collapsed) {
      textEl.addEventListener("click", () => toggleCard(`u:${group.turnNumber}`, true));
    }

    card.append(head, textEl);
    return card;
  }

  /** ASSISTANT card — one-line summary until expanded. */
  function renderAssistantCard(group: TurnGroup, idx: number): HTMLElement {
    const collapsed = asstCollapsed(group.turnNumber);
    const card = el(
      "div",
      `turn-card turn-card--assistant${collapsed ? " turn-card--collapsed" : ""}`,
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

    // The user card (when present) carries the group's "below" control; a
    // prose-only assistant reply still gets one so every turn has the affordance.
    if (!group.userText) {
      head.append(
        belowBtn(rangeExpanded(idx), rangeExpanded(idx) ? "Collapse all below" : "Expand all below",
          () => setRange(idx, !rangeExpanded(idx))),
      );
    }
    head.append(toggleBtn(collapsed, () => toggleCard(`a:${group.turnNumber}`, collapsed)));

    const tools = group.toolExchanges.map((ex) => ex.tool!).filter(Boolean);

    const textEl = el("p", "turn-text");
    textEl.textContent = collapsed
      ? (group.assistantText || toolsSummary(tools))
      : (group.assistantText || "");
    if (collapsed) {
      textEl.addEventListener("click", () => toggleCard(`a:${group.turnNumber}`, true));
    }

    card.append(head);
    if (collapsed) {
      card.append(textEl);
      return card;
    }

    if (group.assistantText) card.append(textEl);

    if (group.toolExchanges.length > 0) {
      const list = el("div");
      list.style.marginTop = group.assistantText ? "9px" : "0";
      group.toolExchanges.forEach((ex, i) => {
        list.append(renderToolCall(ex.tool!, group, i));
      });
      card.append(list);
    }

    if (group.systemText) {
      const sys = el("div", "drawer-system");
      sys.textContent = group.systemText;
      card.append(sys);
    }

    return card;
  }

  // ------------------------------------------------------------------
  // Tool calls — per-type rows + bodies (toolview.ts)
  // ------------------------------------------------------------------

  function renderToolCall(tool: Tool, group: TurnGroup, toolIndex: number): HTMLElement {
    const key = toolExpandKey(group.turnNumber, toolIndex);
    const isOpen = toolOpen(key);
    const view = toolView(tool);

    const wrap = el("div", `tool-call${isOpen ? " tool-call--open" : ""}`);

    const row = el("div", "tool-row");
    const ico = el("span", "tool-ico");
    ico.style.setProperty("--tool-tint", `var(--ink-${view.tint})`);
    ico.append(icon(view.icon, 12.5, 1.8));

    const main = el("span", "tool-main");
    const verb = el("span", "tool-verb");
    verb.textContent = view.verb;
    const headline = el("span", `tool-headline${view.mono ? " tool-headline--mono" : ""}`);
    headline.textContent = view.headline;
    main.append(verb, headline);

    const side = el("span", "tool-side");
    if (view.accessory?.kind === "stat") {
      const acc = el("span", "tool-acc");
      const add = el("span", "tool-acc-add");
      add.textContent = `+${view.accessory.add}`;
      const del = el("span", "tool-acc-del");
      del.textContent = ` −${view.accessory.del}`;
      acc.append(add, del);
      side.append(acc);
    } else if (view.accessory?.kind === "count") {
      const acc = el("span", "tool-acc tool-acc--count");
      acc.textContent = String(view.accessory.n);
      side.append(acc);
    }
    if (group.toolExchanges.length > 1) {
      side.append(
        belowBtn(toolsRangeOpen(group, toolIndex),
          toolsRangeOpen(group, toolIndex) ? "Collapse this + below" : "Expand this + below",
          () => setToolsRange(group, toolIndex, !toolsRangeOpen(group, toolIndex))),
      );
    }
    const chev = el("span", "tool-chevron");
    chev.append(icon("chevron", 12));
    side.append(chev);

    row.append(ico, main, side);
    row.addEventListener("click", () => {
      toolMap[key] = !isOpen;
      persist();
      render(currentGroups);
    });
    wrap.append(row);

    if (isOpen) {
      const bodyEl = el("div", "tool-body");
      bodyEl.append(...renderToolBody(view, tool));
      wrap.append(bodyEl);
    }

    return wrap;
  }

  /** Labeled inset box (command/output/skill/url). */
  function inset(label: string, error = false): { wrap: HTMLElement; box: HTMLElement } {
    const wrap = el("div", "tool-inset");
    const lab = el("div", `tool-inset-label${error ? " tool-inset-label--error" : ""}`);
    lab.textContent = label;
    const box = el("div", "tool-inset-box");
    wrap.append(lab, box);
    return { wrap, box };
  }

  function renderToolBody(view: ToolView, tool: Tool): HTMLElement[] {
    const b = view.body;

    if (b.kind === "command") {
      const out: HTMLElement[] = [];
      const cmd = inset("command");
      const prompt = el("span", "cmd-prompt");
      prompt.textContent = "$ ";
      cmd.box.append(prompt, document.createTextNode(b.command));
      out.push(cmd.wrap);
      if (b.output !== undefined && b.output !== "") {
        const o = inset(tool.truncated ? "output (truncated at 50 KB)" : "output");
        o.box.textContent = b.output;
        out.push(o.wrap);
      }
      return out;
    }

    if (b.kind === "readinfo") {
      const info = el("div", "tool-readinfo");
      info.append(`Read ${b.lines} lines${b.range ? ` (${b.range})` : ""} from `);
      const file = document.createElement("b");
      file.textContent = b.file;
      info.append(file);
      return [info];
    }

    if (b.kind === "diff") {
      const box = el("div", "tool-diff");
      for (const line of b.lines) {
        const ln = el("div", `tool-diff-ln tool-diff-ln--${line.sign === "+" ? "add" : "rem"}`);
        const sign = el("span", "tool-diff-sign");
        sign.textContent = line.sign;
        const text = el("span", "tool-diff-text");
        text.textContent = line.text;
        ln.append(sign, text);
        box.append(ln);
      }
      if (b.truncated > 0) {
        const more = el("div", "tool-diff-more");
        more.textContent = `… ${b.truncated} more line${b.truncated > 1 ? "s" : ""}`;
        box.append(more);
      }
      return [box];
    }

    if (b.kind === "matches") {
      const m = inset(`${b.lines.length} matches`);
      for (const line of b.lines) {
        const row = document.createElement("div");
        const match = /^([^:]+):(\d+):(.*)$/.exec(line);
        if (match) {
          const f = el("span", "tool-match-file");
          f.textContent = match[1]!;
          const n = el("span", "tool-match-line");
          n.textContent = `:${match[2]}:`;
          const t = el("span", "tool-match-text");
          t.textContent = match[3]!;
          row.append(f, n, t);
        } else {
          row.textContent = line;
        }
        m.box.append(row);
      }
      return [m.wrap];
    }

    if (b.kind === "todos") {
      const list = el("div", "tool-todos");
      for (const item of b.items) {
        const row = el("div", `tool-todo tool-todo--${item.s}`);
        if (item.s === "done") {
          const check = icon("check", 13, 2.4);
          check.style.color = "var(--st-success)";
          row.append(check);
        } else {
          row.append(el("span", "tool-todo-ring"));
        }
        const text = el("span", "tool-todo-text");
        text.textContent = item.text;
        row.append(text);
        list.append(row);
      }
      return [list];
    }

    if (b.kind === "inset") {
      const i = inset(b.label);
      i.box.textContent = b.text;
      return [i.wrap];
    }

    if (b.kind === "subagent") {
      const label = b.description ?? b.agentType ?? "subagent";
      const wrap = el("div", "tool-subagent");
      const header = el("div", "tool-subagent-label");
      header.textContent = b.agentType ? `${label} (${b.agentType})` : label;
      wrap.append(header);
      for (const ex of b.exchanges) {
        if (ex.user) {
          const row = el("div", "tool-subagent-turn tool-subagent-turn--user");
          row.textContent = ex.user;
          wrap.append(row);
        }
        if (ex.assistant) {
          const row = el("div", "tool-subagent-turn tool-subagent-turn--assistant");
          row.textContent = ex.assistant;
          wrap.append(row);
        }
        if (ex.tool) {
          const row = el("div", "tool-subagent-turn tool-subagent-turn--tool");
          row.textContent = `${ex.tool.kind} ${ex.tool.arg}`.trim();
          wrap.append(row);
        }
      }
      return [wrap];
    }

    // raw — the honest fallback: full input JSON + output/detail.
    return [renderRawDetail(tool)];
  }

  /**
   * Raw drill-down for unknown tool kinds: pretty-printed input JSON +
   * output (or detail.lines colored spans when present), with truncation
   * notices. Content wraps — no horizontal scroll.
   */
  function renderRawDetail(tool: Tool): HTMLElement {
    const detail = el("div", "drawer-tool-detail");

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

    // Non-empty detail.lines takes precedence over raw output when present,
    // as it carries color annotations.
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
      const notice = el("div", "drawer-tool-detail-label");
      notice.textContent = "output (truncated at 50 KB)";
      detail.append(notice);
    }

    return detail;
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
      onFork(newUuid, text);
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
        if (ex.tool) {
          const v = toolView(ex.tool);
          parts.push(`[${v.verb}] ${v.headline}`);
        }
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

  let unsubscribe: (() => void) | null = null;
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

  // Static mode (live === false): one fetch, no subscription. Used by the forest
  // preview float, which re-fetches on focus instead of tailing the file.
  //
  // Live mode: follow the session through the shared watch hub, which keeps a
  // single EventSource per uuid across the reach map + drawer and reconnects on
  // the pre-flush 404 (see watch.ts).
  if (actions.live !== false) {
    unsubscribe = subscribeWatch(uuid, () => void load());
  }

  // ------------------------------------------------------------------
  // Handle (teardown)
  // ------------------------------------------------------------------

  return {
    close() {
      if (closed) return;
      closed = true;
      unsubscribe?.();
      unsubscribe = null;
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
