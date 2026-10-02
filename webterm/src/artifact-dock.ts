// The artifact pane's dock — a GLOBAL toggle (top bar) that follows the active tab,
// sitting between the terminal stack and the inspect dock. It hosts the artifact pane
// (artifacts.ts): what the session made, sandboxed, plus annotate and the plan gate's
// reviews. Open state and width persist across reloads.
//
// Plan reviews: a review for the active session opens the pane, once per review —
// whether the review or the tab arrived first — so closing it mid-review sticks.

import { makeDragHandle, el } from "./dom.ts";
import { mountArtifactPane } from "./artifacts.ts";
import type { ArtifactPaneHandle, PlanReview } from "./artifacts.ts";
import { artifactWidthFromPointer, ARTIFACT_DEFAULT_W, type TabDescriptor } from "./shell-helpers.ts";

const LS_ARTIFACT = "eigenform:term:artifact:v1"; // "1" = artifact pane open
const LS_ARTIFACT_W = "eigenform:term:artifact-w:v1";

function readNum(key: string, fallback: number): number {
  const v = Number(localStorage.getItem(key));
  return Number.isFinite(v) && v > 0 ? v : fallback;
}

export interface ArtifactDockHandle {
  /** The width splitter + the pane host, to place between the term stack and the dock. */
  resizer: HTMLElement;
  el: HTMLElement;
  isOpen(): boolean;
  /** Persist the open state. The caller re-syncs (and re-fits) afterwards. */
  setOpen(open: boolean): void;
  /** Reconcile against (open, the active tab). */
  sync(active: TabDescriptor | null, hasTabs: boolean): void;
  /**
   * New plan-review list. Returns true when it opened the pane for the active
   * session's review (the caller then re-syncs and re-fits).
   */
  setReviews(reviews: PlanReview[], activeUuid: string | null): boolean;
  /** Reviews as last set (for the rail's "plan review" chip). */
  reviews(): PlanReview[];
}

export function mountArtifactDock(deps: {
  /** Type text into the active tab's terminal input, unsent. False without a live pty. */
  stage: (text: string) => boolean;
  /** The pane's width changed → the terminal column resized; re-fit. */
  onResize: () => void;
}): ArtifactDockHandle {
  const resizer = el("div", "drawer-resizer artifact-resizer");
  resizer.title = "drag to resize the artifact pane";
  const host = el("div", "artifact-host");

  let open = localStorage.getItem(LS_ARTIFACT) === "1";
  let pane: ArtifactPaneHandle | null = null;
  let reviews: PlanReview[] = [];
  const autoOpened = new Set<string>();

  let width = artifactWidthFromPointer(0, readNum(LS_ARTIFACT_W, ARTIFACT_DEFAULT_W));
  const applyWidth = () => document.documentElement.style.setProperty("--artifact-w", `${width}px`);
  applyWidth();

  function setOpen(next: boolean) {
    open = next;
    localStorage.setItem(LS_ARTIFACT, next ? "1" : "0");
  }

  /** Open (state only) when the active session has a review that hasn't opened it yet. */
  function autoOpen(activeUuid: string | null): boolean {
    const review = reviews.find((r) => r.sessionId === activeUuid && !autoOpened.has(r.id));
    if (!review) return false;
    autoOpened.add(review.id);
    if (open) return false;
    setOpen(true);
    return true;
  }

  function sync(active: TabDescriptor | null, hasTabs: boolean) {
    autoOpen(active?.uuid ?? null);
    const shown = open && hasTabs;
    host.style.display = shown ? "flex" : "none";
    resizer.style.display = shown ? "" : "none";
    if (!shown) {
      pane?.close();
      pane = null;
      return;
    }
    pane ??= mountArtifactPane(host, { stage: deps.stage });
    pane.setSession(active?.uuid ?? null);
    pane.setReviews(reviews);
  }

  makeDragHandle(
    resizer,
    "drawer-resizer--dragging",
    "col-resize",
    (e) => {
      width = artifactWidthFromPointer(e.clientX, host.getBoundingClientRect().right);
      applyWidth();
    },
    () => {
      localStorage.setItem(LS_ARTIFACT_W, String(width));
      deps.onResize();
    },
  );

  return {
    resizer,
    el: host,
    isOpen: () => open,
    setOpen,
    sync,
    setReviews(next, activeUuid) {
      reviews = next;
      const opened = autoOpen(activeUuid);
      pane?.setReviews(reviews);
      return opened;
    },
    reviews: () => reviews,
  };
}
