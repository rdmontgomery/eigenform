// The rail's "Links" section: URLs mentioned in the active tab's chat. A
// permanent sidebar fixture (unlike the dock, it's always mounted). The shell
// calls sync() with the active tab's uuid; live updates tail through the same
// watch hub the drawer uses.

import { el } from "./dom.ts";
import { icon } from "./icons.ts";
import { extractUrls, linkLabel } from "./links.ts";
import type { LinkEntry } from "./links.ts";
import type { Exchange } from "./turns.ts";
import { subscribeWatch } from "./watch.ts";

const LS_LINKS_FOLD = "eigenform:term:rail-links-fold:v1";

export interface RailLinksHandle {
  /** Point the section at a session (null = no session → empty). Idempotent. */
  sync(uuid: string | null): void;
}

export function mountRailLinks(container: HTMLElement): RailLinksHandle {
  let current: { uuid: string; unsubscribe: () => void } | null = null;
  let entries: LinkEntry[] = [];
  let folded = localStorage.getItem(LS_LINKS_FOLD) === "1";

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
    if (uuid) {
      const unsubscribe = subscribeWatch(uuid, () => void load(uuid));
      current = { uuid, unsubscribe };
      void load(uuid);
    }
    render();
  }

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

    const list = el("div", "rail-links-list");
    // Most-recently-mentioned first — the newest link is the one you're likely
    // looking for.
    for (const entry of [...entries].reverse()) {
      const row = el("a", "rail-link-row");
      row.href = entry.url;
      row.target = "_blank";
      row.rel = "noopener noreferrer";
      row.title = entry.url;
      row.append(icon("globe", 12));
      const text = el("span", "rail-link-label");
      text.textContent = linkLabel(entry.url);
      row.append(text);
      list.append(row);
    }
    container.append(list);
  }

  return { sync };
}
