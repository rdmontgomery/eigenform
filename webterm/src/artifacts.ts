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
 *
 * Annotate (markdown only): swaps the iframe for the annotator (annotator.ts), which
 * renders the source as plain text in eigenform's origin, collects marks, and stages
 * the compiled critique into the terminal via `opts.stage`, never sending it.
 * Entering annotate pins the artifact so a new write can't switch it away mid-review.
 */

import { subscribeWatch } from "./watch.ts";
import { icon } from "./icons.ts";
import { el } from "./dom.ts";
import { mountAnnotator } from "./annotator.ts";
import type { AnnotatorHandle } from "./annotator.ts";

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

/** One row from GET /api/plan-reviews: a plan Claude is waiting on (plan gate). */
export interface PlanReview {
  id: string;
  sessionId: string | null;
  cwd: string | null;
  toolUseId: string | null;
  /** The plan file's path, or "tool_input.plan" when the plan came inline. */
  source: string;
  createdAt: string;
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
  /** Plans awaiting review (plan gate). One for this pane's session takes over the pane. */
  setReviews(reviews: PlanReview[]): void;
  close(): void;
}

const FOLLOW = "__follow__";

export interface ArtifactPaneOpts {
  /** Type text into the active tab's terminal input, unsent. False without a live pty. */
  stage(text: string): boolean;
}

export function mountArtifactPane(host: HTMLElement, opts: ArtifactPaneOpts): ArtifactPaneHandle {
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
  const annotateBtn = el("button", "icon-btn artifact-btn");
  annotateBtn.append(icon("pencil", 13));
  head.append(picker, meta, annotateBtn, reload, openTab);

  const frame = el("iframe", "artifact-frame");
  // Opaque origin: scripts run, but the document can't touch the daemon. See header.
  frame.setAttribute("sandbox", "allow-scripts allow-forms allow-popups allow-modals allow-downloads");
  frame.setAttribute("referrerpolicy", "no-referrer");
  const empty = el("div", "artifact-empty");
  const annHost = el("div", "artifact-annotate");
  const reviewBanner = el("div", "artifact-review-banner");
  reviewBanner.style.display = "none";
  root.append(reviewBanner, head, frame, annHost, empty);
  host.append(root);

  let uuid: string | null = null;
  let rows: ArtifactRow[] = [];
  let sel: Selection = { path: null, pinned: false };
  let shownSrc: string | null = null;
  let unsubscribe: (() => void) | null = null;
  let seq = 0;
  let annotating = false;
  let reviews: PlanReview[] = [];
  let reviewView: { id: string; handle: AnnotatorHandle } | null = null;
  function closeReview() {
    reviewView?.handle.close();
    reviewView = null;
  }

  async function decide(id: string, kind: string, message?: string): Promise<boolean> {
    try {
      const res = await fetch(`/api/plan-reviews/${encodeURIComponent(id)}/decision`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ decision: kind, message: message ?? "" }),
      });
      return res.ok;
    } catch {
      return false;
    }
  }

  /** Show the pending plan review for this session, if any. True when it owns the pane. */
  function renderReview(): boolean {
    const review = uuid ? reviews.find((r) => r.sessionId === uuid) : undefined;
    if (!review) {
      closeReview();
      reviewBanner.style.display = "none";
      return false;
    }
    closeAnnotator();
    head.style.display = "none";
    frame.style.display = "none";
    empty.style.display = "none";
    annHost.style.display = "";
    reviewBanner.style.display = "";
    reviewBanner.textContent =
      "Plan review: Claude is waiting on this plan. Mark it up, then approve or send it back.";
    if (reviewView?.id !== review.id) {
      closeReview();
      const inline = review.source === "tool_input.plan" || review.source === "none";
      reviewView = {
        id: review.id,
        handle: mountAnnotator(annHost, {
          uuid: uuid!,
          path: inline ? "plan" : review.source,
          rawUrl: `/api/plan-reviews/${encodeURIComponent(review.id)}/plan`,
          stage: opts.stage,
          storageKey: `eigenform:annot:v1:review:${uuid}:${review.toolUseId ?? review.id}`,
          header: "Plan feedback. Revise the plan to address each point, then present it again:",
          review: { decide: (kind, message) => decide(review.id, kind, message) },
        }),
      };
    }
    return true;
  }
  let annotator: { handle: AnnotatorHandle; path: string; mtime: string | null } | null = null;

  function closeAnnotator() {
    annotator?.handle.close();
    annotator = null;
  }

  function render() {
    if (renderReview()) return;
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
    const canAnnotate = row?.kind === "markdown" && row.mtime !== null;
    if (!canAnnotate) annotating = false;
    annotateBtn.style.display = canAnnotate ? "" : "none";
    annotateBtn.classList.toggle("icon-btn--active", annotating);
    annotateBtn.title = annotating ? "Back to the rendered view" : "Annotate: mark up this plan and stage the critique in the terminal";
    annHost.style.display = annotating ? "" : "none";
    if (!annotating || !row || !uuid) closeAnnotator();
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
    meta.textContent = row.mtime ? `turn ${row.turn}` : "deleted";
    meta.title = row.path;
    const src = artifactSrc(row);
    openTab.href = src;
    if (annotating && uuid) {
      frame.style.display = "none";
      if (annotator && annotator.path !== row.path) closeAnnotator();
      if (!annotator) {
        annotator = {
          handle: mountAnnotator(annHost, {
            uuid,
            path: row.path,
            rawUrl: `${row.url}?raw=1`,
            stage: opts.stage,
          }),
          path: row.path,
          mtime: row.mtime,
        };
      } else if (annotator.mtime !== row.mtime) {
        // The agent revised the file under review: re-fetch and re-anchor the marks.
        annotator.mtime = row.mtime;
        annotator.handle.refresh();
      }
      return;
    }
    frame.style.display = "";
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
    if (annotator) annotator.handle.refresh();
    else if (shownSrc) frame.src = shownSrc;
  });
  annotateBtn.addEventListener("click", () => {
    annotating = !annotating;
    // Pin what's under review so a newer write can't switch it away.
    if (annotating && sel.path !== null) sel = { path: sel.path, pinned: true };
    render();
  });

  render();
  return {
    setSession(next) {
      if (next === uuid) return;
      unsubscribe?.();
      unsubscribe = null;
      uuid = next;
      sel = { path: null, pinned: false };
      annotating = false;
      closeAnnotator();
      closeReview();
      rows = [];
      render();
      if (next) {
        unsubscribe = subscribeWatch(next, () => void refresh());
        void refresh();
      }
    },
    setReviews(next) {
      const key = (l: PlanReview[]) => l.map((r) => r.id).join(",");
      if (key(next) === key(reviews)) return;
      reviews = next;
      render();
    },
    close() {
      unsubscribe?.();
      unsubscribe = null;
      closeAnnotator();
      closeReview();
      root.remove();
    },
  };
}
