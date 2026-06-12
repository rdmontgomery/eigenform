/**
 * picker.ts — Fuzzy launcher picker overlay + resolvePick decision function.
 *
 * ## Architecture
 *
 * Two distinct responsibilities live here:
 *
 * 1. **resolvePick** (pure, exported, tested): decides what `{path, create}` pair
 *    to open given the current picker state (typed text + highlighted candidate +
 *    the full candidate list). No DOM, no fetch.
 *
 * 2. **mountPicker** (DOM, not unit-tested): builds the picker overlay element,
 *    handles keyboard nav / fetch / re-rank, and calls a callback when a choice
 *    is confirmed or the picker is dismissed.
 *
 * ## Bare-name resolution (resolvePick)
 *
 * The frontend does not know workspace_root directly. We derive a proxy from the
 * candidate list: non-recent candidates are immediate subdirs of workspace_root,
 * so the parent of the first non-recent candidate IS the workspace root.
 *
 * **Limitation:** if all candidates are recent (or the list is empty), bare-name
 * resolution is unavailable and resolvePick returns null. In that case the user
 * must type an absolute path.
 *
 * Backlog note: a cleaner fix is to expose workspace_root in the GET /api/candidates
 * response as `{ root: string | null, candidates: Candidate[] }`. Deferred because
 * it changes the 3.1 route contract + CLI mirror + tests.
 *
 * ## mkdir flow
 *
 * resolvePick returns `create: true` when the path is not in the known candidates
 * (i.e. it will be a new directory). The caller then opens the tab with `&create=1`.
 * The daemon enforces workspace_root containment; the picker does not pre-validate
 * (server is the authority). A rejection surfaces as a POLICY close → tab shows dead
 * with a reason string.
 */

import type { Candidate } from "./types.ts";
import { rankCandidates } from "./fuzzy.ts";

// ---------------------------------------------------------------------------
// resolvePick — pure decision function
// ---------------------------------------------------------------------------

export interface PickResult {
  /** The absolute path to open in a new pty. */
  path: string;
  /**
   * Whether the daemon should create the directory before spawning.
   * true  → send `&create=1`; the daemon will mkdir_all (subject to root check).
   * false → path already exists (known candidate); no mkdir needed.
   */
  create: boolean;
}

/**
 * Decide what to open given the current picker state.
 *
 * @param typed       The raw text in the input field (may be empty or whitespace).
 * @param highlighted The currently highlighted candidate (null if none).
 * @param candidates  The full candidate list (for root-proxy derivation + match check).
 * @returns A `PickResult` or `null` if the state represents no valid action (e.g. empty input
 *          with no highlight, or a bare name with no non-recent candidates to derive root from).
 */
export function resolvePick(
  typed: string,
  highlighted: Candidate | null,
  candidates: Candidate[],
): PickResult | null {
  // 1. Highlighted candidate takes absolute priority.
  if (highlighted !== null) {
    return { path: highlighted.path, create: false };
  }

  const text = typed.trim();
  if (text.length === 0) return null;

  // 2. Absolute path (starts with "/").
  if (text.startsWith("/")) {
    const known = candidates.some((c) => c.path === text);
    return { path: text, create: !known };
  }

  // 3. Bare name (no "/" anywhere). Resolve against workspace_root proxy.
  //    Proxy = parent of the first non-recent candidate.
  const firstNonRecent = candidates.find((c) => !c.recent);
  if (!firstNonRecent) {
    // No workspace_root proxy available — cannot resolve bare name.
    return null;
  }
  const lastSlash = firstNonRecent.path.lastIndexOf("/");
  const workspaceRoot = lastSlash > 0
    ? firstNonRecent.path.slice(0, lastSlash)
    : "/";
  const resolved = `${workspaceRoot}/${text}`;
  return { path: resolved, create: true };
}

// ---------------------------------------------------------------------------
// mountPicker — DOM overlay (not unit-tested; house posture)
// ---------------------------------------------------------------------------

export interface PickerCallbacks {
  /** Called when the user confirms a selection. The picker should be closed after this. */
  onPick: (result: PickResult) => void;
  /** Called when the user dismisses the picker (Esc or click-outside). */
  onDismiss: () => void;
}

/**
 * Build and mount the picker overlay into `container`. The returned function
 * removes all event listeners and detaches the overlay — call it to clean up.
 *
 * The picker:
 * - Fetches GET /api/candidates on open.
 * - Re-ranks with rankCandidates as the user types.
 * - ↑/↓ to move highlight; Enter to confirm; Esc to dismiss.
 * - Rows: bold basename, dim parent path, faint "recent" tag.
 * - Positioned as an overlay near the button that opened it (CSS class `picker-overlay`).
 */
export function mountPicker(
  container: HTMLElement,
  anchorEl: HTMLElement,
  callbacks: PickerCallbacks,
): () => void {
  let candidates: Candidate[] = [];
  let filtered: Candidate[] = [];
  let highlightIdx: number = -1;

  // ------------------------------------------------------------------
  // DOM structure
  // ------------------------------------------------------------------
  const overlay = document.createElement("div");
  overlay.className = "picker-overlay";

  const input = document.createElement("input");
  input.className = "picker-input";
  input.type = "text";
  input.placeholder = "path or name…";
  input.setAttribute("autocomplete", "off");
  input.setAttribute("spellcheck", "false");

  const list = document.createElement("div");
  list.className = "picker-list";

  overlay.append(input, list);

  // Position overlay near anchor.
  positionOverlay(overlay, anchorEl);
  container.append(overlay);
  input.focus();

  // ------------------------------------------------------------------
  // Fetch candidates
  // ------------------------------------------------------------------
  let fetchAborted = false;
  fetch("/api/candidates")
    .then((r) => r.json() as Promise<Candidate[]>)
    .then((data) => {
      if (fetchAborted) return;
      candidates = data;
      renderList();
    })
    .catch(() => {
      /* Daemon not reachable — picker shows empty list; free-type still works. */
    });

  // ------------------------------------------------------------------
  // Render helpers
  // ------------------------------------------------------------------
  function renderList() {
    list.innerHTML = "";
    highlightIdx = -1;

    const query = input.value;
    filtered = query.length > 0
      ? rankCandidates(query, candidates)
      : candidates.slice();

    for (let i = 0; i < filtered.length; i++) {
      const cand = filtered[i]!;
      const row = buildRow(cand);
      const idx = i; // capture for closure
      row.addEventListener("mouseenter", () => setHighlight(idx));
      row.addEventListener("click", () => confirmCurrent());
      list.append(row);
    }
  }

  function buildRow(cand: Candidate): HTMLDivElement {
    const row = document.createElement("div");
    row.className = "picker-row";

    const lastSlash = cand.path.lastIndexOf("/");
    const basename = cand.path.slice(lastSlash + 1);
    const parent = lastSlash > 0 ? cand.path.slice(0, lastSlash) : cand.path;

    const nameEl = document.createElement("span");
    nameEl.className = "picker-row-basename";
    nameEl.textContent = basename;

    const pathEl = document.createElement("span");
    pathEl.className = "picker-row-parent";
    pathEl.textContent = parent;

    row.append(nameEl, pathEl);

    if (cand.recent) {
      const tag = document.createElement("span");
      tag.className = "picker-row-recent";
      tag.textContent = "recent";
      row.append(tag);
    }

    return row;
  }

  function setHighlight(idx: number) {
    // Remove old highlight.
    const rows = list.querySelectorAll<HTMLDivElement>(".picker-row");
    rows.forEach((r, i) => r.classList.toggle("picker-row--highlight", i === idx));
    highlightIdx = idx;
  }

  function confirmCurrent() {
    const highlighted = highlightIdx >= 0 ? (filtered[highlightIdx] ?? null) : null;
    const result = resolvePick(input.value, highlighted, candidates);
    if (result) {
      callbacks.onPick(result);
      teardown();
    }
  }

  // ------------------------------------------------------------------
  // Keyboard handling
  // ------------------------------------------------------------------
  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      callbacks.onDismiss();
      teardown();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = Math.min(highlightIdx + 1, filtered.length - 1);
      setHighlight(next);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      const prev = Math.max(highlightIdx - 1, -1);
      setHighlight(prev);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      confirmCurrent();
      return;
    }
  }

  function onInput() {
    renderList();
  }

  input.addEventListener("keydown", onKeyDown);
  input.addEventListener("input", onInput);

  // ------------------------------------------------------------------
  // Click-outside dismiss
  // ------------------------------------------------------------------
  function onDocClick(e: MouseEvent) {
    if (!overlay.contains(e.target as Node)) {
      callbacks.onDismiss();
      teardown();
    }
  }
  // Defer so the click that opened the picker doesn't immediately close it.
  requestAnimationFrame(() => {
    document.addEventListener("click", onDocClick);
  });

  // ------------------------------------------------------------------
  // Teardown: remove listeners + overlay
  // ------------------------------------------------------------------
  function teardown() {
    fetchAborted = true;
    input.removeEventListener("keydown", onKeyDown);
    input.removeEventListener("input", onInput);
    document.removeEventListener("click", onDocClick);
    overlay.remove();
  }

  return teardown;
}

// ---------------------------------------------------------------------------
// Position overlay near anchor element
// ---------------------------------------------------------------------------

function positionOverlay(overlay: HTMLElement, anchor: HTMLElement) {
  const rect = anchor.getBoundingClientRect();
  overlay.style.position = "fixed";
  overlay.style.top = `${rect.bottom + 4}px`;
  overlay.style.left = `${rect.left}px`;
}
