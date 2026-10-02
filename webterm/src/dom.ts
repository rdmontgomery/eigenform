// Shared DOM construction helper — the one `el` every view module uses.

/** Create an element, optionally with a class string. */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}

/**
 * Make `handle` a drag splitter: `onMove` per mousemove while dragging, `onEnd` on
 * release. While dragging, the body carries `is-dragging` so CSS can stop iframes
 * (the artifact pane) from swallowing mousemove.
 */
export function makeDragHandle(
  handle: HTMLElement,
  cls: string,
  cursor: string,
  onMove: (e: MouseEvent) => void,
  onEnd: () => void,
): void {
  let dragging = false;
  handle.addEventListener("mousedown", (e) => {
    dragging = true;
    e.preventDefault();
    handle.classList.add(cls);
    document.body.classList.add("is-dragging");
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
    document.body.classList.remove("is-dragging");
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    onEnd();
  });
}
