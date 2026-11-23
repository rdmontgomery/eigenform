// The rail's "Links" section: URLs mentioned in the active tab's chat. A
// permanent sidebar fixture (unlike the dock, it's always mounted). The shell
// calls sync() with the active tab's uuid; live updates tail through the same
// watch hub the drawer uses.
//
// A long-running session collects hundreds of links, so they come grouped (by
// site, or by who sent them; see links.ts groupLinks), each group folding on its
// own, with a filter box over the lot.

import { el } from "./dom.ts";
import { icon } from "./icons.ts";
import { extractUrls, groupLinks, linkLabel, pullRequest } from "./links.ts";
import type { LinkEntry, LinkGrouping } from "./links.ts";
import type { Exchange } from "./turns.ts";
import { subscribeWatch } from "./watch.ts";

const LS_LINKS_FOLD = "eigenform:term:rail-links-fold:v1";
const LS_LINKS_VIEW = "eigenform:term:rail-links-view:v1";

/** Below this many links the panel shows a flat list: groups only help a crowd. */
const GROUP_FROM = 8;

interface LinksView {
  by: LinkGrouping;
  /** Folded group ids. */
  folded: string[];
}

function loadView(): LinksView {
  try {
    const v = JSON.parse(localStorage.getItem(LS_LINKS_VIEW) ?? "{}") as Partial<LinksView>;
    return {
      by: v.by === "source" ? "source" : "site",
      folded: Array.isArray(v.folded) ? v.folded.filter((k) => typeof k === "string") : [],
    };
  } catch {
    return { by: "site", folded: [] };
  }
}

export interface RailLinksHandle {
  /** Point the section at a session (null = no session → empty). Idempotent. */
  sync(uuid: string | null): void;
}

export function mountRailLinks(container: HTMLElement): RailLinksHandle {
  let current: { uuid: string; unsubscribe: () => void } | null = null;
  let entries: LinkEntry[] = [];
  let folded = localStorage.getItem(LS_LINKS_FOLD) === "1";
  const view = loadView();
  let query = "";

  const saveView = () => {
    try {
      localStorage.setItem(LS_LINKS_VIEW, JSON.stringify(view));
    } catch {
      // A view preference that doesn't persist is no loss.
    }
  };

  async function load(uuid: string) {
    try {
      const res = await fetch(`/api/session/${encodeURIComponent(uuid)}/json`);
      if (!res.ok) return;
      const payload = (await res.json()) as { exchanges: Exchange[] };
      if (current?.uuid !== uuid) return; // stale — tab switched mid-fetch
      entries = extractUrls(payload.exchanges);
      render();
    } catch {
      // Best-effort — the rail just shows no links.
    }
  }

  function sync(uuid: string | null) {
    if (current?.uuid === uuid) return;
    current?.unsubscribe();
    current = null;
    entries = [];
    query = "";
    if (uuid) {
      const unsubscribe = subscribeWatch(uuid, () => void load(uuid));
      current = { uuid, unsubscribe };
      void load(uuid);
    }
    render();
  }

  // The toolbar outlives renders, so typing in the filter keeps focus and caret.
  const toolbar = el("div", "rail-links-tools");
  const filter = document.createElement("input");
  filter.type = "search";
  filter.className = "rail-links-filter";
  filter.placeholder = "Filter links";
  filter.spellcheck = false;
  filter.addEventListener("input", () => {
    query = filter.value.trim();
    renderList();
  });
  filter.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && filter.value) {
      e.stopPropagation();
      filter.value = "";
      query = "";
      renderList();
    }
  });
  const modes = el("div", "rail-links-modes");
  modes.setAttribute("role", "group");
  modes.setAttribute("aria-label", "Group links by");
  const modeButtons = (["site", "source"] as const).map((by) => {
    const b = el("button", "rail-links-mode");
    b.type = "button";
    b.textContent = by === "site" ? "Site" : "Source";
    b.title = by === "site" ? "Group by website, pull requests first" : "Group by who sent the link";
    b.addEventListener("click", () => {
      view.by = by;
      saveView();
      renderList();
    });
    modes.append(b);
    return [by, b] as const;
  });
  toolbar.append(filter, modes);
  const list = el("div", "rail-links-list");

  function render() {
    container.innerHTML = "";
    container.classList.toggle("rail-links--empty", entries.length === 0);
    if (entries.length === 0) return;

    const header = el("button", `rail-links-header${folded ? " rail-links-header--folded" : ""}`);
    header.title = folded ? "Show links" : "Hide links";
    const caret = el("span", "rail-group-caret");
    caret.append(icon("chevron", 11));
    const label = el("span", "rail-group-label");
    label.textContent = "LINKS";
    const rule = el("span", "rail-group-rule");
    const count = el("span", "rail-group-count");
    count.textContent = String(entries.length);
    header.append(caret, label, rule, count);
    header.addEventListener("click", () => {
      folded = !folded;
      localStorage.setItem(LS_LINKS_FOLD, folded ? "1" : "0");
      render();
    });
    container.append(header);
    if (folded) return;

    const grouped = entries.length >= GROUP_FROM;
    if (grouped) {
      if (filter.value.trim() !== query) filter.value = query;
      container.append(toolbar);
    }
    container.append(list);
    renderList();
  }

  function linkRow(entry: LinkEntry, withHost = true): HTMLElement {
    const row = el("a", `rail-link-row rail-link-row--${entry.source}`);
    row.href = entry.url;
    row.target = "_blank";
    row.rel = "noopener noreferrer";
    const from = entry.source === "user" ? "you" : entry.source === "claude" ? "Claude" : "a subagent";
    row.title = `${entry.url}\nfrom ${from}, turn ${entry.turnNumber}`;
    const pr = pullRequest(entry.url);
    row.append(icon(pr ? "fork" : "globe", 12));
    const text = el("span", "rail-link-label");
    text.textContent = linkLabel(entry.url, withHost);
    row.append(text);
    return row;
  }

  function renderList() {
    list.innerHTML = "";
    for (const [by, b] of modeButtons) {
      b.classList.toggle("rail-links-mode--on", view.by === by);
      b.setAttribute("aria-pressed", String(view.by === by));
    }
    // Few links: newest first, flat.
    if (entries.length < GROUP_FROM) {
      for (const entry of [...entries].reverse()) list.append(linkRow(entry));
      return;
    }
    const groups = groupLinks(entries, view.by, query);
    if (groups.length === 0) {
      const none = el("div", "rail-links-none");
      none.textContent = "no matching links";
      list.append(none);
      return;
    }
    for (const g of groups) {
      // A filter opens every group: a hit hidden in a folded one reads as no hit.
      const isFolded = !query && view.folded.includes(g.key);
      const head = el("button", `rail-links-group${isFolded ? " rail-links-group--folded" : ""}`);
      head.type = "button";
      head.title = isFolded ? "Show group" : "Hide group";
      const caret = el("span", "rail-group-caret");
      caret.append(icon("chevron", 10));
      const name = el("span", "rail-links-group-name");
      name.textContent = g.label;
      const n = el("span", "rail-group-count");
      n.textContent = String(g.entries.length);
      head.append(caret, name, n);
      head.addEventListener("click", () => {
        view.folded = isFolded ? view.folded.filter((k) => k !== g.key) : [...view.folded, g.key];
        saveView();
        renderList();
      });
      list.append(head);
      if (isFolded) continue;
      const body = el("div", "rail-links-group-body");
      // A site group's header already names the host.
      const withHost = !g.key.startsWith("site:");
      for (const entry of g.entries) body.append(linkRow(entry, withHost));
      list.append(body);
    }
  }

  return { sync };
}
