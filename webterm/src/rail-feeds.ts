// The rail's "Needs you" section: what local feeds (see docs/plans/2026-11-12-workers-
// and-feeds-design.md) say is waiting on the operator. Hidden when no feed is
// configured. Everything a provider sends renders as text, never as HTML.

import { el } from "./dom.ts";
import { icon } from "./icons.ts";
import { downFeeds, feedRows, formatAge, safeLink } from "./feeds.ts";
import type { FeedRow, FeedsResponse } from "./feeds.ts";

const LS_FEEDS_FOLD = "eigenform:term:rail-feeds-fold:v1";
const POLL_MS = 30_000;

export function mountRailFeeds(container: HTMLElement): void {
  let data: FeedsResponse = { feeds: [] };
  let folded = localStorage.getItem(LS_FEEDS_FOLD) === "1";
  /** `${feed}\0${item}` → the error its last action answered. */
  const rowErrors = new Map<string, string>();
  let busy: string | null = null;
  let loading = false;

  async function load() {
    if (loading) return;
    loading = true;
    try {
      const res = await fetch("/api/feeds");
      if (res.ok) data = (await res.json()) as FeedsResponse;
    } catch {
      // Daemon unreachable: keep the last list rather than blanking the section.
    } finally {
      loading = false;
    }
    render();
  }

  async function act(row: FeedRow, action: number) {
    const key = `${row.feed}\0${row.item.id}`;
    busy = `${key}\0${action}`;
    rowErrors.delete(key);
    render();
    try {
      const res = await fetch("/api/feeds/act", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ feed: row.feed, item: row.item.id, action }),
      });
      if (!res.ok) {
        const body = (await res.json().catch(() => ({}))) as { error?: string; status?: number };
        rowErrors.set(key, body.error || (body.status ? `HTTP ${body.status}` : `HTTP ${res.status}`));
      }
    } catch (e) {
      rowErrors.set(key, String(e));
    }
    busy = null;
    await load();
  }

  function render() {
    container.replaceChildren();
    const rows = feedRows(data.feeds);
    const down = downFeeds(data.feeds);
    const configured = data.feeds.length > 0 || !!data.error;
    container.classList.toggle("rail-feeds--empty", !configured);
    if (!configured) return;

    const header = el("button", `rail-links-header${folded ? " rail-links-header--folded" : ""}`);
    header.title = folded ? "Show what needs you" : "Hide what needs you";
    const caret = el("span", "rail-group-caret");
    caret.append(icon("chevron", 11));
    const label = el("span", "rail-group-label");
    label.textContent = "NEEDS YOU";
    const rule = el("span", "rail-group-rule");
    const count = el("span", "rail-group-count");
    count.textContent = String(rows.length);
    header.append(caret, label, rule, count);
    header.addEventListener("click", () => {
      folded = !folded;
      localStorage.setItem(LS_FEEDS_FOLD, folded ? "1" : "0");
      render();
    });
    container.append(header);
    if (folded) return;

    const list = el("div", "rail-feeds-list");
    const now = Date.now();
    for (const row of rows) list.append(renderRow(row, now));
    if (data.error) list.append(dimLine(data.error));
    for (const f of down) {
      const line = dimLine(`${f.name} unreachable`);
      if (f.error) line.title = f.error;
      list.append(line);
    }
    container.append(list);
  }

  function dimLine(text: string): HTMLElement {
    const line = el("div", "rail-feed-down");
    line.textContent = text;
    return line;
  }

  function renderRow(row: FeedRow, now: number): HTMLElement {
    const { item } = row;
    const key = `${row.feed}\0${item.id}`;
    const wrap = el("div", "rail-feed-row");

    const top = el("div", "rail-feed-top");
    const chip = el("span", "rail-feed-kind");
    chip.textContent = item.kind;
    const href = safeLink(item.link);
    const title = href ? el("a", "rail-feed-title") : el("span", "rail-feed-title");
    title.textContent = item.title;
    title.title = item.title;
    if (href && title instanceof HTMLAnchorElement) {
      title.href = href;
      title.target = "_blank";
      title.rel = "noopener noreferrer";
    }
    const age = el("span", "rail-feed-age");
    age.textContent = formatAge(item.at, now);
    top.append(chip, title, age);
    wrap.append(top);

    if (item.detail) {
      const detail = el("div", "rail-feed-detail");
      detail.textContent = item.detail;
      detail.title = item.detail;
      wrap.append(detail);
    }

    if (item.actions.length > 0) {
      const actions = el("div", "rail-feed-actions");
      item.actions.forEach((a, i) => {
        const b = el("button", "rail-feed-action");
        b.textContent = a.label;
        b.disabled = busy !== null;
        if (busy === `${key}\0${i}`) b.classList.add("rail-feed-action--busy");
        b.addEventListener("click", () => void act(row, i));
        actions.append(b);
      });
      wrap.append(actions);
    }

    const err = rowErrors.get(key);
    if (err) {
      const e = el("div", "rail-feed-error");
      e.textContent = err;
      wrap.append(e);
    }
    return wrap;
  }

  render();
  void load();
  setInterval(() => void load(), POLL_MS);
  window.addEventListener("focus", () => void load());
}
