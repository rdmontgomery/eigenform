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
