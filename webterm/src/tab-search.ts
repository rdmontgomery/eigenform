/**
 * tab-search.ts — find a tab in the overstory (the session tabs along the top) by typing.
 *
 * ⌘/Ctrl+Shift+F (or the magnifier at the overstory's left edge) opens a filter box.
 * Every tab whose title, launch dir, session uuid or pty id contains all the
 * query's plain words, and whose session satisfies its facet terms (`pr:56`,
 * `repo:app`, `~#56`… — see facets.ts), is highlighted; the rest dim. The selected
 * match carries a stronger ring and is scrolled into view. ↓/↑ (or Tab/⇧Tab)
 * cycle the selection, Enter jumps to it and closes the box, Esc closes.
 *
 * The overstory itself still belongs to shell.ts: it rebuilds tabs from the model
 * and asks `classFor(id)` for each one, so search state survives rebuilds.
 * `matchTabs` is pure and tested in tab-search.test.ts.
 */

import { el } from "./dom.ts";
import { icon } from "./icons.ts";
import { FACET_HELP, matchesFacets, parseQuery, type Facets } from "./facets.ts";

/** What search reads from a tab. */
export interface TabSearchItem {
  id: string;
  label: string;
  cwd?: string;
  uuid?: string;
  ptyId?: string;
  /** The tab's session facets, from the forest snapshot. */
  facets?: Facets;
}

/**
 * Ids of the items matching `query`, best first. Case-insensitive; every plain
 * word must be a substring of the item's label, cwd, uuid or ptyId, and every
 * facet term must hold for its session (facets.ts). Items with more words found
 * in the label rank first (the title is what you're scanning for); ties keep
 * overstory order. A blank query matches nothing — "not searching".
 */
export function matchTabs(query: string, items: readonly TabSearchItem[]): string[] {
  const { words, facets } = parseQuery(query);
  if (words.length === 0 && facets.length === 0) return [];
  const hits: Array<{ id: string; labelHits: number; idx: number }> = [];
  items.forEach((it, idx) => {
    const label = it.label.toLowerCase();
    const hay = [label, it.cwd, it.uuid, it.ptyId].filter(Boolean).join("\n").toLowerCase();
    if (!words.every((t) => hay.includes(t))) return;
    if (!matchesFacets(facets, it.facets)) return;
    hits.push({ id: it.id, labelHits: words.filter((t) => label.includes(t)).length, idx });
  });
  hits.sort((a, b) => b.labelHits - a.labelHits || a.idx - b.idx);
  return hits.map((h) => h.id);
}

export interface TabSearch {
  /** Toggle button + filter box; mount at the overstory's left edge. */
  el: HTMLElement;
  open(): void;
  close(): void;
  isOpen(): boolean;
  /** Extra classes for a tab while searching, or [] when idle. */
  classFor(id: string): string[];
  /** Recompute matches against the current tabs (call before rebuilding the overstory). */
  sync(): void;
}

export function mountTabSearch(opts: {
  items: () => TabSearchItem[];
  /** Jump to a tab. */
  activate: (id: string) => void;
  /** Search state changed — rebuild the overstory. */
  onChange: () => void;
  /** Box closed — hand focus back (the terminal). */
  onClose: () => void;
}): TabSearch {
  let isOpen = false;
  let matches: string[] = [];
  let sel = 0;

  const root = el("div", "tab-search");
  const btn = el("button", "icon-btn tab-search-btn");
  btn.title = "Find tab (⌘/Ctrl+Shift+F)";
  btn.append(icon("search", 15));
  const box = el("div", "tab-search-box");
  const input = el("input", "tab-search-input");
  input.placeholder = "Find tab";
  input.title = FACET_HELP;
  input.spellcheck = false;
  const count = el("span", "tab-search-count");
  box.append(input, count);
  root.append(btn, box);

  function render() {
    root.classList.toggle("tab-search--open", isOpen);
    btn.classList.toggle("icon-btn--active", isOpen);
    const q = input.value.trim();
    count.textContent = q ? (matches.length ? `${sel + 1}/${matches.length}` : "0") : "";
    root.classList.toggle("tab-search--miss", q !== "" && matches.length === 0);
  }

  function sync() {
    const prev = matches[sel];
    matches = isOpen ? matchTabs(input.value, opts.items()) : [];
    // Keep the same tab selected across rebuilds when it still matches.
    const keep = prev ? matches.indexOf(prev) : -1;
    sel = keep >= 0 ? keep : 0;
    render();
  }

  function changed() {
    opts.onChange();
    const id = matches[sel];
    if (id) {
      const tab = document.querySelector<HTMLElement>(`.tab[data-tab-id="${CSS.escape(id)}"]`);
      tab?.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
  }

  function open() {
    if (!isOpen) {
      isOpen = true;
      sync();
      changed();
    }
    input.focus();
    input.select();
  }

  function close() {
    if (!isOpen) return;
    isOpen = false;
    input.value = "";
    sync();
    opts.onChange();
    opts.onClose();
  }

  function step(delta: number) {
    if (matches.length === 0) return;
    sel = (sel + delta + matches.length) % matches.length;
    render();
    changed();
  }

  // Keep focus in the input so the blur handler below doesn't race the toggle.
  btn.addEventListener("mousedown", (e) => e.preventDefault());
  btn.addEventListener("click", () => (isOpen ? close() : open()));
  input.addEventListener("input", () => {
    sel = 0;
    matches = [];
    sync();
    changed();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Enter") {
      e.preventDefault();
      const id = matches[sel];
      if (!id) return;
      close();
      opts.activate(id);
    } else if (e.key === "ArrowDown" || (e.key === "Tab" && !e.shiftKey)) {
      e.preventDefault();
      step(1);
    } else if (e.key === "ArrowUp" || (e.key === "Tab" && e.shiftKey)) {
      e.preventDefault();
      step(-1);
    }
  });
  // Clicking away with nothing typed folds the box back to the button.
  input.addEventListener("blur", () => {
    if (isOpen && input.value.trim() === "") close();
  });

  // Capture phase so the shortcut wins over xterm, which owns focus most of the time.
  window.addEventListener(
    "keydown",
    (e) => {
      if (!(e.metaKey || e.ctrlKey) || !e.shiftKey || e.altKey || e.code !== "KeyF") return;
      e.preventDefault();
      e.stopPropagation();
      open();
    },
    true,
  );

  render();
  return {
    el: root,
    open,
    close,
    isOpen: () => isOpen,
    classFor(id) {
      if (!isOpen || input.value.trim() === "") return [];
      const i = matches.indexOf(id);
      if (i < 0) return ["tab--nomatch"];
      return i === sel ? ["tab--match", "tab--match-sel"] : ["tab--match"];
    },
    sync,
  };
}
