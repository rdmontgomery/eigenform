/**
 * forest-preview.ts — a single floating panel that previews a forest session's
 * transcript + meta before you commit to launching it.
 *
 * Design (docs/plans/2026-06-26-forest-preview-float-design.md):
 *   - One reusable float instance, repositioned/repopulated as focus moves.
 *   - STATIC snapshot: the transcript is rendered once per focus via the shared
 *     drawer renderer in `live: false` mode (no SSE). Re-focusing re-fetches.
 *   - Meta header: cwd (project-tinted), last-activity, ~turn count, model, and a
 *     live marker. A Launch button commits.
 *   - Separate from the docked inspect panel: the dock means "active session",
 *     the float means "browsing, not committed yet".
 */

import { mountDrawer, type DrawerHandle } from "./drawer.ts";
import { relativeRecency, inkFor } from "./shell-helpers.ts";
import type { RosterRow } from "./roster.ts";

export interface ForestPreviewOptions {
  /** Commit: launch/resume the previewed session. */
  onLaunch: (row: RosterRow) => void;
  /** Open a fork of the previewed session (newUuid) as a tab. Optional. */
  onFork?: (newUuid: string) => void;
}

export interface ForestPreviewHandle {
  /** Show the float for `row`, anchored beside `anchor`. */
  show(row: RosterRow, anchor: HTMLElement): void;
  /** Hide the float and tear down its transcript. */
  hide(): void;
  isOpen(): boolean;
  /** The key of the row currently previewed, or null. */
  currentKey(): string | null;
  destroy(): void;
}

// Client-side model cache: focusing the same session repeatedly shouldn't refetch
// just to label the model. Dropped only on full reload (the model rarely changes).
const modelCache = new Map<string, string | null>();

async function fetchModel(uuid: string): Promise<string | null> {
  if (modelCache.has(uuid)) return modelCache.get(uuid) ?? null;
  try {
    const res = await fetch(`/api/session/${encodeURIComponent(uuid)}/json`);
    if (!res.ok) return null;
    const j = (await res.json()) as { model?: string | null };
    const m = j.model ?? null;
    modelCache.set(uuid, m);
    return m;
  } catch {
    return null;
  }
}

/** "claude-opus-4-8" → "Opus 4.8"; unknown ids degrade to a tidy fallback. */
export function prettyModel(id: string | null): string {
  if (!id) return "";
  const m = id.match(/(opus|sonnet|haiku|fable)[-]?(\d+)[-.]?(\d+)?/i);
  if (m) {
    const fam = m[1]!.charAt(0).toUpperCase() + m[1]!.slice(1).toLowerCase();
    const ver = m[3] ? `${m[2]}.${m[3]}` : m[2];
    return `${fam} ${ver}`;
  }
  return id.replace(/^claude-/, "");
}

export function createForestPreview(opts: ForestPreviewOptions): ForestPreviewHandle {
  // ── DOM (built once) ──────────────────────────────────────────────────────
  const float = el("div", "forest-preview");
  float.style.display = "none";

  const head = el("div", "forest-preview-head");
  const cwdEl = el("div", "forest-preview-cwd");
  const metaEl = el("div", "forest-preview-meta");
  const recencyEl = el("span", "forest-preview-recency");
  const countEl = el("span", "forest-preview-count");
  const modelEl = el("span", "forest-preview-model");
  const liveEl = el("span", "forest-preview-live");
  metaEl.append(recencyEl, countEl, modelEl, liveEl);

  const launchBtn = el("button", "forest-preview-launch");
  launchBtn.type = "button";
  launchBtn.textContent = "Launch ⏎";

  head.append(cwdEl, metaEl, launchBtn);

  const bodyHost = el("div", "forest-preview-body");
  float.append(head, bodyHost);
  document.body.append(float);

  let drawer: DrawerHandle | null = null;
  let currentRow: RosterRow | null = null;
  let open = false;

  function clearDrawer() {
    drawer?.close();
    drawer = null;
    bodyHost.innerHTML = "";
  }

  function renderMeta(row: RosterRow) {
    cwdEl.textContent = row.cwd ?? row.cwdChip;
    // Project tint mirrors the rail/tab coloring (color = project).
    cwdEl.style.setProperty("--row-ink", `var(--ink-${inkFor(row.cwd ?? row.cwdChip)})`);
    recencyEl.textContent = relativeRecency(row.recency, Date.now());
    countEl.textContent = row.msgCount !== undefined ? `~${row.msgCount} turns` : "";
    modelEl.textContent = "";
    liveEl.textContent = row.live ? "live" : "";
  }

  function show(row: RosterRow, anchor: HTMLElement) {
    currentRow = row;
    open = true;
    renderMeta(row);
    clearDrawer();

    if (row.uuid) {
      const uuid = row.uuid;
      // Static transcript via the shared drawer renderer (no SSE).
      drawer = mountDrawer(
        bodyHost,
        uuid,
        (newUuid) => opts.onFork?.(newUuid),
        { live: false },
      );
      // Model needs the JSON payload; fetch async and fill in if still current.
      void fetchModel(uuid).then((m) => {
        if (open && currentRow?.uuid === uuid) modelEl.textContent = prettyModel(m);
      });
    } else {
      const note = el("div", "forest-preview-empty");
      note.textContent = "session still initializing — no transcript yet";
      bodyHost.append(note);
    }

    float.style.display = "flex";
    position(anchor);
  }

  function hide() {
    open = false;
    currentRow = null;
    clearDrawer();
    float.style.display = "none";
  }

  // ── Positioning ───────────────────────────────────────────────────────────
  function position(anchor: HTMLElement) {
    const gap = 8;
    const margin = 8;
    const aRect = anchor.getBoundingClientRect();
    const fW = float.offsetWidth;
    const fH = float.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    // Narrow viewport: center over the content area instead of flanking the rail.
    if (vw < 720) {
      float.style.left = `${Math.max(margin, (vw - fW) / 2)}px`;
      float.style.top = `${Math.max(margin, (vh - fH) / 2)}px`;
      return;
    }

    let left = aRect.right + gap;
    if (left + fW + margin > vw) left = Math.max(margin, aRect.left - gap - fW);
    let top = aRect.top;
    top = Math.min(top, vh - fH - margin);
    top = Math.max(margin, top);
    float.style.left = `${left}px`;
    float.style.top = `${top}px`;
  }

  // ── Wiring ────────────────────────────────────────────────────────────────
  launchBtn.addEventListener("click", () => {
    if (currentRow) opts.onLaunch(currentRow);
  });

  return {
    show,
    hide,
    isOpen: () => open,
    currentKey: () => currentRow?.key ?? null,
    destroy() {
      clearDrawer();
      float.remove();
    },
  };
}

// Local DOM helper (same pattern as shell.ts / drawer.ts — kept independent).
function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}
