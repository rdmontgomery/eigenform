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
 * resolvePick decides only the target path and whether it's a *known* candidate
 * (`known: true` → it exists for sure, open it). For anything typed (`known:
 * false`) the picker can't tell from the browser whether the dir exists, so it
 * asks the daemon (`GET /api/path`) and then either opens it (exists) or shows a
 * "Create <path>?" confirmation before opening with `&create=1`. The daemon
 * allows creation anywhere; the confirmation is the only gate.
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
   * true  → send `&create=1`; the daemon mkdir_all's it (allowed anywhere).
   * false → the directory already exists; no mkdir needed.
   */
  create: boolean;
}

export interface PickDecision {
  /** The absolute path to open. */
  path: string;
  /**
   * True when `path` matched a known candidate — it exists, so open it directly
   * with no existence probe and no create prompt. False for any typed/derived
   * path, whose existence the caller must check via `GET /api/path` before
   * deciding to open (exists) or confirm-then-create (missing).
   */
  known: boolean;
}

/**
 * Decide the target path given the current picker state.
 *
 * @param typed       The raw text in the input field (may be empty or whitespace).
 * @param highlighted The currently highlighted candidate (null if none).
 * @param candidates  The full candidate list (for root-proxy derivation + match check).
 * @returns A `PickDecision`, or `null` if the state represents no valid action (e.g. empty
 *          input with no highlight, or a bare name with no non-recent candidate to derive
 *          the workspace root from).
 */
export function resolvePick(
  typed: string,
  highlighted: Candidate | null,
  candidates: Candidate[],
): PickDecision | null {
  // 1. Highlighted candidate takes absolute priority — it exists.
  if (highlighted !== null) {
    return { path: highlighted.path, known: true };
  }

  const text = typed.trim();
  if (text.length === 0) return null;

  // 2. Absolute path ("/…") or a home path ("~" / "~/…"; the daemon expands ~).
  //    Known iff it matches a candidate exactly.
  if (text.startsWith("/") || text === "~" || text.startsWith("~/")) {
    const known = candidates.some((c) => c.path === text);
    return { path: text, known };
  }

  // 3. Relative path: contains "/" but doesn't start with "/" or "~/" — unsupported.
  if (text.includes("/")) {
    return null;
  }

  // 4. Bare name (no "/" anywhere). Resolve against workspace_root proxy.
  //    Proxy = parent of the first non-recent candidate. Existence is unknown
  //    (the dir may or may not already be there) → known: false.
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
  return { path: resolved, known: false };
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
  // When set, the picker is showing a "Create <path>?" confirmation; Enter creates,
  // Esc returns to picking, and typing cancels back to the list.
  let pendingCreate: string | null = null;

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

  function finish(result: PickResult) {
    callbacks.onPick(result);
    teardown();
  }

  function confirmCurrent() {
    // In confirmation mode, confirming means "yes, create it".
    if (pendingCreate !== null) {
      finish({ path: pendingCreate, create: true });
      return;
    }
    const highlighted = highlightIdx >= 0 ? (filtered[highlightIdx] ?? null) : null;
    const decision = resolvePick(input.value, highlighted, candidates);
    if (!decision) return;
    if (decision.known) {
      // A known candidate exists — open it directly.
      finish({ path: decision.path, create: false });
      return;
    }
    // Typed/derived path: existence unknown. Ask the daemon, then open or confirm.
    void probeAndAct(decision.path);
  }

  /** Stat the path via the daemon, then open it (exists) or offer to create it (missing). */
  async function probeAndAct(path: string) {
    const info = await probePath(path);
    if (info && info.isDir) {
      finish({ path, create: false });
    } else if (info && info.exists) {
      showMessage(`${path} is a file, not a directory`);
    } else {
      // Missing — or the probe failed; either way, confirm before creating.
      showConfirm(path);
    }
  }

  async function probePath(path: string): Promise<{ exists: boolean; isDir: boolean } | null> {
    try {
      const r = await fetch(`/api/path?path=${encodeURIComponent(path)}`);
      if (!r.ok) return null;
      return (await r.json()) as { exists: boolean; isDir: boolean };
    } catch {
      return null;
    }
  }

  /** Swap the list for a "Create <path>?" confirmation panel (Enter creates, Esc cancels). */
  function showConfirm(path: string) {
    pendingCreate = path;
    highlightIdx = -1;
    list.innerHTML = "";
    const panel = document.createElement("div");
    panel.className = "picker-confirm";
    const q = document.createElement("div");
    q.className = "picker-confirm-q";
    q.textContent = `Create ${path}?`;
    const hint = document.createElement("div");
    hint.className = "picker-confirm-hint";
    hint.textContent = "⏎ create · esc cancel";
    panel.append(q, hint);
    panel.addEventListener("click", () => confirmCurrent());
    list.append(panel);
  }

  /** Show a transient message (e.g. "is a file") in place of the list; stays in pick mode. */
  function showMessage(msg: string) {
    pendingCreate = null;
    highlightIdx = -1;
    list.innerHTML = "";
    const m = document.createElement("div");
    m.className = "picker-message";
    m.textContent = msg;
    list.append(m);
  }

  // ------------------------------------------------------------------
  // Keyboard handling
  // ------------------------------------------------------------------
  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (pendingCreate !== null) {
        // Cancel the create confirmation — back to picking, don't dismiss.
        e.preventDefault();
        pendingCreate = null;
        renderList();
        return;
      }
      callbacks.onDismiss();
      teardown();
      return;
    }
    if (pendingCreate !== null) {
      // While confirming, Enter creates; swallow nav keys.
      if (e.key === "Enter") {
        e.preventDefault();
        confirmCurrent();
      }
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
    // Typing abandons any in-progress create confirmation.
    pendingCreate = null;
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
