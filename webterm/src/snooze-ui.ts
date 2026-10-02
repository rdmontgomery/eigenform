/**
 * snooze-ui.ts — the two snooze popovers: the per-tab "snooze until…" menu
 * (presets + a typed duration) and the top-bar shelf of snoozed tabs (wake now /
 * cancel). Pure presentation; the shell owns the API calls and what waking does.
 */

import { el } from "./dom.ts";
import { icon } from "./icons.ts";
import {
  SNOOZE_PRESETS,
  formatWake,
  parseDuration,
  type Snooze,
} from "./snooze.ts";

let current: { close: () => void; anchor: HTMLElement } | null = null;

/**
 * Open a popover under `anchor`, left-aligned and clamped to the viewport. Closes
 * on Escape, on a pointer down outside it and its anchor, or when another opens.
 * Clicking the same anchor again toggles it shut. Returns the popover element.
 */
function openPopover(anchor: HTMLElement, cls: string): HTMLElement | null {
  if (current) {
    const same = current.anchor === anchor;
    current.close();
    if (same) return null;
  }
  const pop = el("div", `snooze-pop ${cls}`);
  document.body.append(pop);

  const onOutside = (e: PointerEvent) => {
    const t = e.target as Node;
    if (!pop.contains(t) && !anchor.contains(t)) close();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") { e.preventDefault(); close(); }
  };
  function close() {
    pop.remove();
    document.removeEventListener("pointerdown", onOutside, true);
    document.removeEventListener("keydown", onKey, true);
    if (current?.anchor === anchor) current = null;
  }
  document.addEventListener("pointerdown", onOutside, true);
  document.addEventListener("keydown", onKey, true);
  current = { close, anchor };

  requestAnimationFrame(() => {
    const r = anchor.getBoundingClientRect();
    const w = pop.offsetWidth;
    pop.style.top = `${Math.round(r.bottom + 6)}px`;
    pop.style.left = `${Math.round(Math.max(8, Math.min(r.left, window.innerWidth - w - 8)))}px`;
  });
  return pop;
}

/** Close whichever snooze popover is open. */
export function closeSnoozePopover(): void {
  current?.close();
}

/**
 * Per-tab menu: pick a preset or type a duration ("45m", "2h", "3d"). `onPick`
 * gets the absolute wake time in epoch ms.
 */
export function openSnoozeMenu(
  anchor: HTMLElement,
  label: string,
  onPick: (until: number) => void,
): void {
  const pop = openPopover(anchor, "snooze-menu");
  if (!pop) return;
  const pick = (until: number) => {
    closeSnoozePopover();
    onPick(until);
  };

  const head = el("div", "snooze-head");
  head.textContent = `Snooze “${label}” until…`;
  pop.append(head);

  for (const p of SNOOZE_PRESETS) {
    const b = el("button", "snooze-opt");
    const name = el("span");
    name.textContent = p.label;
    const when = el("span", "snooze-opt-when");
    when.textContent = new Date(p.until(Date.now())).toLocaleString(undefined, {
      weekday: "short", hour: "numeric", minute: "2-digit",
    });
    b.append(name, when);
    b.addEventListener("click", () => pick(p.until(Date.now())));
    pop.append(b);
  }

  const form = el("form", "snooze-custom");
  const input = el("input", "snooze-input");
  input.placeholder = "or type: 45m · 2h · 3d";
  input.setAttribute("aria-label", "Custom snooze duration");
  const go = el("button", "snooze-go");
  go.type = "submit";
  go.textContent = "Snooze";
  const hint = el("div", "snooze-hint");
  const update = () => {
    const ms = parseDuration(input.value);
    go.disabled = ms === null;
    hint.textContent = ms === null
      ? (input.value.trim() ? "try 30m, 2h, 1h30m, 3 days" : "")
      : `wakes ${new Date(Date.now() + ms).toLocaleString(undefined, {
          weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
        })}`;
  };
  input.addEventListener("input", update);
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const ms = parseDuration(input.value);
    if (ms !== null) pick(Date.now() + ms);
  });
  update();
  form.append(input, go);
  pop.append(form, hint);
  requestAnimationFrame(() => input.focus());
}

/** Top-bar shelf: every snoozed tab with when it wakes, plus wake-now and cancel. */
export function openSnoozeShelf(
  anchor: HTMLElement,
  snoozes: Snooze[],
  actions: { onWake: (s: Snooze) => void; onCancel: (s: Snooze) => void },
): void {
  const pop = openPopover(anchor, "snooze-shelf");
  if (!pop) return;
  const head = el("div", "snooze-head");
  head.textContent = snoozes.length ? "Snoozed tabs" : "Nothing snoozed";
  pop.append(head);
  const now = Date.now();
  for (const s of snoozes) {
    const row = el("div", "snooze-row");
    const text = el("div", "snooze-row-text");
    const name = el("span", "snooze-row-label");
    name.textContent = s.tab.label;
    name.title = s.tab.cwd ?? "";
    const when = el("span", "snooze-row-when");
    when.textContent = `wakes ${formatWake(s.until, now)}`;
    when.title = new Date(s.until).toLocaleString();
    text.append(name, when);

    const wake = el("button", "snooze-row-btn");
    wake.title = "Wake now — reopen the tab";
    wake.append(icon("play", 11, 2));
    wake.addEventListener("click", () => { closeSnoozePopover(); actions.onWake(s); });
    const cancel = el("button", "snooze-row-btn");
    cancel.title = "Cancel snooze (don't reopen)";
    cancel.append(icon("x", 11, 2));
    cancel.addEventListener("click", () => { row.remove(); actions.onCancel(s); });
    row.append(text, wake, cancel);
    pop.append(row);
  }
}
