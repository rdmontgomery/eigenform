// The inspect dock — a GLOBAL panel (one toggle, top-right) that follows the
// active tab. Right-docked and pushing the terminal narrower (not a floating
// overlay): reach map on top, transcript below, split by a draggable divider,
// and the global events pane at the foot. Reach + transcript follow the active
// tab's session uuid; a tab with no uuid yet shows a placeholder instead.
// Open state and geometry (width, reach height) persist across reloads.

import { el } from "./dom.ts";
import { mountDrawer } from "./drawer.ts";
import type { DrawerHandle } from "./drawer.ts";
import { mountReachMap } from "./reachmap.ts";
import type { ReachHandle } from "./reachmap.ts";
import { mountEvents } from "./events.ts";
import type { EventsHandle } from "./events.ts";
import {
  drawerWidthFromPointer,
  DRAWER_DEFAULT_W,
  splitHeightFromPointer,
  REACH_DEFAULT_H,
  type TabDescriptor,
} from "./shell-helpers.ts";

const LS_DRAWER = "eigenform:term:drawer:v1";
const LS_DOCK_W = "eigenform:term:drawer-w:v1";
const LS_REACH_H = "eigenform:term:reach-h:v1";

function readNum(key: string, fallback: number): number {
  const v = Number(localStorage.getItem(key));
  return Number.isFinite(v) && v > 0 ? v : fallback;
}

export interface DockHandle {
  /** The width splitter + the dock itself, to place beside the term stack. */
  resizer: HTMLElement;
  el: HTMLElement;
  isOpen(): boolean;
  /** Persist the open state. The caller re-syncs (and re-fits) afterwards. */
  setOpen(open: boolean): void;
  /** Reconcile the mounted panes against (open, the active tab). */
  sync(active: TabDescriptor | null, hasTabs: boolean): void;
}

export function mountDock(
  termHost: HTMLElement,
  deps: {
    /** The transcript forked a session: open the branch with `text` staged. */
    onFork: (newUuid: string, text: string) => void;
    /** The dock's width changed → the terminal column resized; re-fit. */
    onResize: () => void;
    /** Open/close or content changed — re-render the top-bar toggle. */
    onChange: () => void;
  },
): DockHandle {
  const dockResizer = el("div", "drawer-resizer");
  dockResizer.title = "drag to resize the inspect panel";
  const drawerDock = el("div", "drawer-dock");
  const reachRegion = el("div", "reach-region");
  const dockVsplit = el("div", "dock-vsplit");
  dockVsplit.title = "drag to resize the reach map / transcript split";
  const transcriptRegion = el("div", "transcript-region");
  // Events pane: a collapsible accordion at the dock's foot. Unlike the reach map
  // + transcript (which are uuid-bound), it's global, so it has no vertical split —
  // it self-collapses to just its header when folded.
  const eventsRegion = el("div", "events-region");
  drawerDock.append(reachRegion, dockVsplit, transcriptRegion, eventsRegion);

  let drawerOpen = localStorage.getItem(LS_DRAWER) === "1";
  /** Mounted transcript drawer (uuid-bound), or null. */
  let drawerCurrent: { uuid: string; handle: DrawerHandle } | null = null;
  /** Mounted reach map (uuid-bound), or null. */
  let reachCurrent: { uuid: string; handle: ReachHandle } | null = null;
  /** Mounted events pane (global — not uuid-bound), or null. Lives while the dock
   *  is open, independent of the active tab. */
  let eventsCurrent: EventsHandle | null = null;
  /** Placeholder shown when the dock is open but the active tab has no uuid. */
  let dockPlaceholder: HTMLElement | null = null;

  // Persisted dock geometry: dock width + reach-region height. Re-clamped on
  // read so a stale/garbage value can't wedge the layout.
  let dockW = drawerWidthFromPointer(0, readNum(LS_DOCK_W, DRAWER_DEFAULT_W));
  let reachH = readNum(LS_REACH_H, REACH_DEFAULT_H);

  function applyDockGeometry() {
    document.documentElement.style.setProperty("--drawer-w", `${dockW}px`);
    document.documentElement.style.setProperty("--reach-h", `${reachH}px`);
  }
  applyDockGeometry();

  function saveDockGeometry() {
    localStorage.setItem(LS_DOCK_W, String(dockW));
    localStorage.setItem(LS_REACH_H, String(reachH));
  }

  function sync(active: TabDescriptor | null, hasTabs: boolean) {
    const open = drawerOpen && hasTabs;
    drawerDock.style.display = open ? "flex" : "none";
    dockResizer.style.display = open ? "" : "none";

    if (!open) {
      reachCurrent?.handle.close();
      reachCurrent = null;
      drawerCurrent?.handle.close();
      drawerCurrent = null;
      eventsCurrent?.close();
      eventsCurrent = null;
      deps.onChange();
      return;
    }

    // The events pane is global (not uuid-bound) — mount it once while the dock is
    // open, before the per-uuid reach/transcript wiring below.
    if (!eventsCurrent) eventsCurrent = mountEvents(eventsRegion);

    const uuid = active?.uuid ?? null;

    if (uuid) {
      if (dockPlaceholder) {
        dockPlaceholder.remove();
        dockPlaceholder = null;
      }
      reachRegion.style.display = "";
      dockVsplit.style.display = "";

      if (reachCurrent?.uuid !== uuid) {
        reachCurrent?.handle.close();
        // No onClose → the reach map renders without a close button and won't
        // grab Esc; it lives in the dock for as long as the dock is open.
        reachCurrent = {
          uuid,
          handle: mountReachMap(reachRegion, uuid, {
            root: active?.cwd ?? undefined,
          }),
        };
      }
      if (drawerCurrent?.uuid !== uuid) {
        drawerCurrent?.handle.close();
        // onFork: the shell opens the forked session as a new tab and refreshes
        // the roster (copy-on-fork — source tab stays open). The edited prompt is
        // staged into the resumed branch, unsent — the daemon never writes it.
        drawerCurrent = {
          uuid,
          handle: mountDrawer(transcriptRegion, uuid, deps.onFork),
        };
      }
    } else {
      // No transcript yet — collapse the split to a single placeholder.
      reachCurrent?.handle.close();
      reachCurrent = null;
      drawerCurrent?.handle.close();
      drawerCurrent = null;
      reachRegion.style.display = "none";
      dockVsplit.style.display = "none";
      // Rebuilt (not reused) each time: the message depends on the active tab's
      // kind, which can change between calls (switching from a claude tab whose
      // uuid hasn't resolved yet to a plain terminal tab, or vice versa).
      dockPlaceholder?.remove();
      dockPlaceholder = el("div", "drawer");
      const head = el("div", "drawer-header");
      const title = el("span", "drawer-title");
      title.textContent = "Transcript";
      head.append(title);
      const empty = el("div", "drawer-empty");
      empty.textContent = active?.kind === "terminal"
        ? "plain terminal — no transcript for this tab"
        : "no transcript yet — waiting for a session uuid";
      dockPlaceholder.append(head, empty);
      transcriptRegion.append(dockPlaceholder);
    }
    deps.onChange();
  }

  // ── Splitters ──────────────────────────────────────────────────────────────
  // Width (the dock's left edge) and the reach/transcript vertical split. Both
  // mirror the rail resizer: drag updates the CSS var live; state persists on
  // mouseup. The width drag re-fits the terminal once at the end — a per-pixel
  // resize would SIGWINCH the pty on every move (spike 09's repaint handles one).
  function makeDragHandle(
    handle: HTMLElement,
    cls: string,
    cursor: string,
    onMove: (e: MouseEvent) => void,
    onEnd: () => void,
  ) {
    let dragging = false;
    handle.addEventListener("mousedown", (e) => {
      dragging = true;
      e.preventDefault();
      handle.classList.add(cls);
      document.body.style.cursor = cursor;
      document.body.style.userSelect = "none";
    });
    window.addEventListener("mousemove", (e) => {
      if (!dragging) return;
      onMove(e);
    });
    window.addEventListener("mouseup", () => {
      if (!dragging) return;
      dragging = false;
      handle.classList.remove(cls);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      onEnd();
    });
  }

  makeDragHandle(
    dockResizer,
    "drawer-resizer--dragging",
    "col-resize",
    (e) => {
      dockW = drawerWidthFromPointer(e.clientX, termHost.getBoundingClientRect().right);
      applyDockGeometry();
    },
    () => {
      saveDockGeometry();
      deps.onResize();
    },
  );

  makeDragHandle(
    dockVsplit,
    "dock-vsplit--dragging",
    "row-resize",
    (e) => {
      const r = drawerDock.getBoundingClientRect();
      reachH = splitHeightFromPointer(e.clientY, r.top, r.height);
      applyDockGeometry();
    },
    saveDockGeometry,
  );

  return {
    resizer: dockResizer,
    el: drawerDock,
    isOpen: () => drawerOpen,
    setOpen(open) {
      drawerOpen = open;
      localStorage.setItem(LS_DRAWER, open ? "1" : "0");
    },
    sync,
  };
}
