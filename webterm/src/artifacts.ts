/**
 * artifacts.ts — the split pane that renders what the active session MADE:
 * HTML, SVG, markdown and images it wrote or edited (GET /api/session/:uuid/artifacts),
 * including files a nested subagent or Codex worker wrote.
 *
 * Isolation: each artifact loads in an <iframe sandbox> WITHOUT allow-same-origin,
 * and the daemon serves it under a matching `Content-Security-Policy: sandbox` header
 * — so the document has an opaque origin even if opened in its own tab, and can't
 * reach the daemon's API or pty socket. Never add allow-same-origin here.
 *
 * Follow vs pin: by default the pane follows the newest write. Picking an artifact
 * from the menu pins it; picking "follow newest" (or a session switch) unpins.
 * A pinned artifact that disappears from the list falls back to following.
 *
 * Live: subscribes to the session's watch; when the shown file's mtime changes the
 * iframe reloads (cache-busted on mtime), so an agent iterating on a page is visible
 * as it iterates.
 */

import { subscribeWatch } from "./watch.ts";
import { icon } from "./icons.ts";

/** One row from GET /api/session/:uuid/artifacts (mirrors the daemon's JSON). */
export interface ArtifactRow {
  path: string;
  name: string | null;
  kind: "html" | "svg" | "markdown" | "image";
  /** Exchange number of the last write. */
  turn: number;
  /** Who wrote it last: "session", or a nested agent's type ("codex", …). */
  by: string;
  /** ISO-8601 mtime; null when the file no longer exists. */
  mtime: string | null;
  /** Sandboxed daemon URL for the file. */
  url: string;
}

export interface Selection {
  path: string | null;
  pinned: boolean;
}

/**
 * Which artifact to show. Rows arrive newest-write first. Pinned and still present →
 * keep it; otherwise follow the newest row that still exists on disk (falling back
 * to the newest row at all, so a deleted file still explains itself).
 */
export function pickArtifact(rows: ArtifactRow[], sel: Selection): Selection {
  if (sel.pinned && sel.path !== null && rows.some((r) => r.path === sel.path)) return sel;
  const live = rows.find((r) => r.mtime !== null) ?? rows[0];
  return { path: live ? live.path : null, pinned: false };
}

/** The iframe src for a row: cache-busted on mtime so an edit reloads the frame. */
export function artifactSrc(row: ArtifactRow): string {
  const v = row.mtime ? encodeURIComponent(row.mtime) : "gone";
  return `${row.url}${row.url.includes("?") ? "&" : "?"}v=${v}`;
}

/** Short menu label: file name, plus who wrote it when it wasn't the session. */
export function artifactLabel(row: ArtifactRow): string {
  const name = row.name ?? row.path.split("/").pop() ?? row.path;
  return row.by === "session" ? name : `${name} · ${row.by}`;
}

// ---------------------------------------------------------------------------
// DOM
// ---------------------------------------------------------------------------

export interface ArtifactPaneHandle {
  /** Follow a session (null = no session: empty state). */
  setSession(uuid: string | null): void;
  close(): void;
}

const FOLLOW = "__follow__";

export function mountArtifactPane(host: HTMLElement): ArtifactPaneHandle {
  const root = el("div", "artifact-pane");
  const head = el("div", "artifact-head");
  const picker = el("select", "artifact-picker");
  picker.title = "Artifacts this session wrote (newest first)";
  const meta = el("span", "artifact-meta");
  const reload = el("button", "icon-btn artifact-btn");
  reload.title = "Reload";
  reload.append(icon("refresh", 13));
  const openTab = el("a", "icon-btn artifact-btn");
  openTab.title = "Open in a new tab (still sandboxed)";
  openTab.target = "_blank";
  openTab.rel = "noopener noreferrer";
  openTab.append(icon("external", 13));
  head.append(picker, meta, reload, openTab);

  const frame = el("iframe", "artifact-frame");
  // Opaque origin: scripts run, but the document can't touch the daemon. See header.
  frame.setAttribute("sandbox", "allow-scripts allow-forms allow-popups allow-modals allow-downloads");
  frame.setAttribute("referrerpolicy", "no-referrer");
  const empty = el("div", "artifact-empty");
  root.append(head, frame, empty);
  host.append(root);

  let uuid: string | null = null;
  let rows: ArtifactRow[] = [];
  let sel: Selection = { path: null, pinned: false };
  let shownSrc: string | null = null;
  let unsubscribe: (() => void) | null = null;
  let seq = 0;

  function render() {
    sel = pickArtifact(rows, sel);
    const row = rows.find((r) => r.path === sel.path) ?? null;

    picker.innerHTML = "";
    const follow = el("option");
    follow.value = FOLLOW;
    follow.textContent = "follow newest";
    picker.append(follow);
    for (const r of rows) {
      const o = el("option");
      o.value = r.path;
      o.textContent = artifactLabel(r);
      o.title = r.path;
      picker.append(o);
    }
    picker.value = sel.pinned && row ? row.path : FOLLOW;
    picker.disabled = rows.length === 0;

    head.style.display = rows.length === 0 ? "none" : "";
    if (!row) {
      frame.style.display = "none";
      empty.style.display = "";
      empty.textContent = uuid
        ? "Nothing to show yet. HTML, SVG, markdown or images this session writes appear here, and update as they change."
        : "No session in this tab.";
      if (shownSrc !== null) {
        frame.removeAttribute("src");
        shownSrc = null;
      }
      return;
    }
    empty.style.display = "none";
    frame.style.display = "";
    meta.textContent = row.mtime ? `turn ${row.turn}` : "deleted";
    meta.title = row.path;
    const src = artifactSrc(row);
    openTab.href = src;
    if (src !== shownSrc) {
      frame.src = src;
      shownSrc = src;
    }
  }

  async function refresh() {
    const want = uuid;
    const mine = ++seq;
    if (!want) {
      rows = [];
      render();
      return;
    }
    try {
      const res = await fetch(`/api/session/${encodeURIComponent(want)}/artifacts`);
      const next = res.ok ? ((await res.json()) as ArtifactRow[]) : [];
      if (mine !== seq || want !== uuid) return; // stale
      rows = next;
    } catch {
      if (mine !== seq) return;
      rows = [];
    }
    render();
  }

  picker.addEventListener("change", () => {
    sel = picker.value === FOLLOW ? { path: null, pinned: false } : { path: picker.value, pinned: true };
    render();
  });
  reload.addEventListener("click", () => {
    if (shownSrc) frame.src = shownSrc;
  });

  render();
  return {
    setSession(next) {
      if (next === uuid) return;
      unsubscribe?.();
      unsubscribe = null;
      uuid = next;
      sel = { path: null, pinned: false };
      rows = [];
      render();
      if (next) {
        unsubscribe = subscribeWatch(next, () => void refresh());
        void refresh();
      }
    },
    close() {
      unsubscribe?.();
      unsubscribe = null;
      root.remove();
    },
  };
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}
