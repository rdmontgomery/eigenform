// The rail: search box + age-grouped session roster + footer. Owns the rows'
// presentation state — search query, folded groups, keyboard selection and the
// preview float, inline-rename label overrides. What a row *does* (launch,
// fork, which one backs the active tab) is the shell's, passed in as callbacks.

import { el } from "./dom.ts";
import { icon } from "./icons.ts";
import { createForestPreview } from "./forest-preview.ts";
import type { ForestPreviewHandle } from "./forest-preview.ts";
import type { RosterRow } from "./roster.ts";
import { ageGroup, relativeRecency, type AgeGroup } from "./shell-helpers.ts";
import { dotClasses, dotTitle, inkVar, livenessTag } from "./status.ts";

const LS_OVERRIDES = "eigenform:term:overrides:v1";
const LS_GROUPS = "eigenform:term:rail-groups:v1";

/** A rail group: an age bucket for interactive sessions, or the one "headless"
 *  bucket that gathers every `claude -p` / SDK run regardless of age. */
type RailGroup = AgeGroup | "headless";

const GROUP_LABELS: Record<RailGroup, string> = {
  today: "Today",
  week: "This week",
  earlier: "Earlier",
  headless: "Headless",
};
/** Groups that start folded until the user opens them. Headless runs are
 *  scripted noise next to the sessions you're driving by hand. */
const FOLDED_BY_DEFAULT: ReadonlySet<RailGroup> = new Set(["headless"]);
const GROUP_ORDER: AgeGroup[] = ["today", "week", "earlier"];

export interface RailElements {
  scroll: HTMLElement;
  foot: HTMLElement;
  searchInput: HTMLInputElement;
  searchClear: HTMLButtonElement;
}

export interface RailDeps {
  /** True when this row backs the active tab. */
  isActive: (row: RosterRow) => boolean;
  /** Launch/attach the row's session (double-click, Enter, preview Launch). */
  onLaunch: (row: RosterRow) => void;
  /** The preview forked the session at a turn: open the branch with `text` staged. */
  onFork: (newUuid: string, text: string) => void;
  /** Re-fetch the roster (after a rename commits or is cancelled). */
  onRefresh: () => void;
  /** True when Claude is waiting on this row's plan (plan gate). */
  hasPlanReview?: (row: RosterRow) => boolean;
}

export interface RailHandle {
  /** Replace the roster and re-render. */
  setRows(rows: RosterRow[]): void;
  /** The latest roster (unfiltered). */
  rows(): RosterRow[];
  /** Re-render from current state (e.g. the active tab changed). */
  render(): void;
  /** User label overrides (uuid or ptyId → label), for buildRoster + tab titles. */
  overrides(): Readonly<Record<string, string>>;
}

export function mountRail(els: RailElements, deps: RailDeps): RailHandle {
  const { scroll: railScroll, foot: railFoot, searchInput, searchClear } = els;


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

  /** Folded rail groups (true = collapsed). Persisted across reloads; a group
   *  never toggled falls back to FOLDED_BY_DEFAULT. */
  let foldedGroups: Partial<Record<RailGroup, boolean>> = {};
  try {
    foldedGroups = JSON.parse(localStorage.getItem(LS_GROUPS) ?? "{}") as Partial<
      Record<RailGroup, boolean>
    >;
  } catch {
    foldedGroups = {};
  }

  function isFolded(group: RailGroup): boolean {
    return foldedGroups[group] ?? FOLDED_BY_DEFAULT.has(group);
  }

  function toggleGroup(group: RailGroup) {
    foldedGroups[group] = !isFolded(group);
    localStorage.setItem(LS_GROUPS, JSON.stringify(foldedGroups));
    renderRail();
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
      deps.onLaunch(row);
      preview.hide();
    },
    onFork: (newUuid, text) => deps.onFork(newUuid, text),
  });

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
        deps.onLaunch(row);
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
    searchClear.hidden = searchInput.value === "";
    renderRail();
  });

  searchClear.addEventListener("click", () => {
    searchInput.value = "";
    searchInput.dispatchEvent(new Event("input"));
    searchInput.focus();
  });

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

    // Interactive sessions bucket by age; headless runs share one group at the
    // bottom so a batch of `claude -p` jobs never buries the sessions you drive.
    const buckets: [RailGroup, RosterRow[]][] = GROUP_ORDER.map((g) => [
      g,
      rows.filter((r) => !r.headless && ageGroup(r.recency, now) === g),
    ]);
    buckets.push(["headless", rows.filter((r) => r.headless)]);

    for (const [group, groupRows] of buckets) {
      if (groupRows.length === 0) continue;

      // A fold hides the group's rows (the count still says how many). While a
      // search is active, folds are ignored — hiding matches inside a folded
      // group would make the search read as "no results" for no visible reason.
      const folded = !searchQuery && isFolded(group);

      const header = el("button", `rail-group-header${folded ? " rail-group-header--folded" : ""}`);
      header.title = folded ? "Show group" : "Hide group";
      const caret = el("span", "rail-group-caret");
      caret.append(icon("chevron", 11));
      const label = el("span", "rail-group-label");
      label.textContent = GROUP_LABELS[group];
      const rule = el("span", "rail-group-rule");
      const count = el("span", "rail-group-count");
      // A folded headless group still says how many of its runs are live.
      const liveCount = groupRows.filter((r) => r.liveness !== "none").length;
      count.textContent =
        group === "headless" && liveCount > 0
          ? `${liveCount} live · ${groupRows.length}`
          : String(groupRows.length);
      header.append(caret, label, rule, count);
      header.addEventListener("click", () => toggleGroup(group));
      railScroll.append(header);

      if (folded) continue;

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
    if (deps.isActive(row)) item.classList.add("rail-row--active");

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
    if (deps.hasPlanReview?.(row)) {
      const pending = el("span", "rail-row-review");
      pending.textContent = "plan review";
      pending.title = "Claude is waiting on plan approval: open the session to review it";
      meta.append(pending);
    }
    if (row.engine) {
      const engine = el("span", "rail-row-engine");
      engine.textContent = row.engine;
      engine.title = "OpenAI Codex CLI thread — opens with `codex resume`";
      meta.append(engine);
    }
    if (row.msgCount !== undefined) {
      const count = el("span", "rail-row-count");
      count.textContent = `~${row.msgCount}`;
      meta.append(count);
    }
    const tag = livenessTag(row.activity, row.liveness);
    if (tag) {
      const live = el("span", "rail-row-live");
      if (row.liveness === "external") live.classList.add("rail-row-live--external");
      if (row.activity !== "idle") live.classList.add(`rail-row-live--${row.activity}`);
      live.textContent = tag;
      live.title = dotTitle(row.activity, row.liveness);
      meta.append(live);
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
      deps.onLaunch(row);
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
        deps.onRefresh();
      };

      input.addEventListener("blur", commit, { once: true });
      input.addEventListener("keydown", (ev) => {
        if (ev.key === "Enter") {
          input.blur();
        } else if (ev.key === "Escape") {
          input.removeEventListener("blur", commit);
          deps.onRefresh();
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
    railFoot.title = "Show active sessions";
  }

  return {
    setRows(rows) {
      lastRows = rows;
      renderRail();
    },
    rows: () => lastRows,
    render: renderRail,
    overrides: () => overrides,
  };
}
