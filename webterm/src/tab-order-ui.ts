/**
 * tab-order-ui.ts — the overstory's sort menu. Each choice reorders the tabs once;
 * nothing keeps them sorted afterwards, so a drag still moves a tab where you put it.
 */

import { el } from "./dom.ts";
import { closeSnoozePopover, openPopover } from "./snooze-ui.ts";

export type TabOrder = "recent" | "stale" | "directory";

const CHOICES: { order: TabOrder; label: string; hint: string }[] = [
  { order: "recent", label: "Last messaged", hint: "newest first" },
  { order: "stale", label: "Last messaged", hint: "oldest first" },
  { order: "directory", label: "Group by directory", hint: "src/<repo> together" },
];

export function openTabOrderMenu(anchor: HTMLElement, onPick: (order: TabOrder) => void): void {
  const pop = openPopover(anchor, "tab-order-menu");
  if (!pop) return;
  const head = el("div", "snooze-head");
  head.textContent = "Reorder tabs once";
  pop.append(head);
  for (const c of CHOICES) {
    const b = el("button", "snooze-opt");
    const name = el("span");
    name.textContent = c.label;
    const hint = el("span", "snooze-opt-when");
    hint.textContent = c.hint;
    b.append(name, hint);
    b.addEventListener("click", () => {
      closeSnoozePopover();
      onPick(c.order);
    });
    pop.append(b);
  }
}
