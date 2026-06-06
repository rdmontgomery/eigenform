// dom.ts — a minimal hyperscript so the view code reads like the React prototype
// without a framework. `el`/`svg` build real nodes; there is no vdom and no diffing.

type Child = Node | string | number | null | undefined | false | Child[];

interface Attrs {
  class?: string;
  text?: string;
  html?: string;
  title?: string;
  type?: string;
  id?: string;
  list?: string;
  rows?: number;
  placeholder?: string;
  value?: string;
  disabled?: boolean;
  dataset?: Record<string, string>;
  style?: string;
  [event: `on${string}`]: ((e: Event) => void) | undefined;
}

function append(node: Node, children: Child[]): void {
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    if (Array.isArray(c)) append(node, c);
    else if (c instanceof Node) node.appendChild(c);
    else node.appendChild(document.createTextNode(String(c)));
  }
}

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null) continue;
    if (k === "class") node.className = v as string;
    else if (k === "text") node.textContent = v as string;
    else if (k === "html") node.innerHTML = v as string;
    else if (k === "dataset") Object.assign((node as HTMLElement).dataset, v);
    else if (k === "style") node.setAttribute("style", v as string);
    else if (k.startsWith("on") && typeof v === "function") {
      node.addEventListener(k.slice(2).toLowerCase(), v as EventListener);
    } else if (k === "disabled") {
      (node as HTMLInputElement).disabled = Boolean(v);
    } else if (k === "value") {
      (node as HTMLInputElement).value = v as string;
    } else {
      node.setAttribute(k, String(v));
    }
  }
  append(node, children);
  return node;
}

const SVGNS = "http://www.w3.org/2000/svg";

export function svg(tag: string, attrs: Record<string, string | number> = {}, ...children: Node[]): SVGElement {
  const node = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  for (const c of children) node.appendChild(c);
  return node;
}
