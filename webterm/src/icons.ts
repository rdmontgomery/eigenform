/**
 * icons.ts — inline stroke-icon set from the eigenform design system.
 *
 * Each icon is a 24×24 viewBox of stroke paths (round caps/joins), rendered at
 * a given pixel size and colored via currentColor. All path data is static —
 * innerHTML here never carries user content.
 *
 * "mark" is the logo: George Spencer-Brown's mark of distinction in its
 * eigenform variant (the mark applied to itself — a fixed point), per the
 * design handoff. The second, nested stroke renders at reduced opacity.
 */

const PATHS: Record<string, string> = {
  // eigenform — nested marks of distinction
  mark: '<path d="M3 6h18V19"/><path d="M8 10h9V16" opacity="0.55"/>',
  panel: '<path d="M4 5h16v14H4z"/><path d="M15 5v14"/>',
  sun: '<path d="M12 4v2M12 18v2M4 12H2M22 12h-2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4l1.4-1.4M17 7l1.4-1.4"/><circle cx="12" cy="12" r="3.6"/>',
  moon: '<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  search: '<path d="M11 19a8 8 0 1 1 0-16 8 8 0 0 1 0 16zM21 21l-4.3-4.3"/>',
  chevron: '<path d="M9 6l6 6-6 6"/>',
  copy: '<path d="M9 9h10v10H9zM5 15V5h10"/>',
  fork: '<path d="M6 4v8M18 4v3a4 4 0 0 1-4 4H6"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="4" r="2"/><circle cx="6" cy="4" r="2"/>',
  stop: '<path d="M7 7h10v10H7z"/>',
  // tool-call type icons + outliner chevrons (design pass 2026-06-12)
  terminal: '<path d="M5 7l5 5-5 5M13 17h6"/>',
  doc: '<path d="M7 3h7l4 4v14H7zM14 3v4h4"/>',
  pencil: '<path d="M4 20l4-1L18 7l-3-3L5 15l-1 5zM14 6l3 3"/>',
  globe: '<path d="M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18M3.5 12h17M12 3c2.6 2.6 2.6 15.4 0 18M12 3c-2.6 2.6-2.6 15.4 0 18"/>',
  check: '<path d="M5 13l4 4L19 7"/>',
  list: '<path d="M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01"/>',
  skill: '<path d="M12 3l2.5 5.5L20 9l-4 4 1 6-5-3-5 3 1-6-4-4 5.5-.5z"/>',
  bolt: '<path d="M13 3L5 13h6l-1 8 8-10h-6z"/>',
  chevrons: '<path d="M6 5l6 6 6-6M6 12l6 6 6-6"/>',
  type: '<path d="M5 7V5h14v2M12 5v14M9 19h6"/>',
  palette: '<path d="M12 3a9 9 0 1 0 0 18c1 0 1.7-.8 1.7-1.7 0-.5-.2-.9-.5-1.2-.3-.3-.5-.7-.5-1.1 0-.9.8-1.7 1.7-1.7H17a4 4 0 0 0 4-4c0-4.4-4-8.3-9-8.3z"/><circle cx="7.5" cy="10.5" r="1"/><circle cx="12" cy="7.5" r="1"/><circle cx="16.5" cy="10.5" r="1"/>',
};

/** Build an <svg> element for a named icon. Unknown names render empty. */
export function icon(name: string, size = 16, strokeWidth = 1.7): SVGSVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", String(strokeWidth));
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  svg.innerHTML = PATHS[name] ?? "";
  return svg;
}
