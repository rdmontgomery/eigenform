// Shimmer: a slow band of the scheme's own ANSI hues rolling across the whole app.
//
// One fixed, click-through layer over everything below the popovers, blended with
// `soft-light` so it tints rather than covers. The band is twice as wide as one hue
// cycle and slides by exactly one cycle per loop, so the loop is seamless. The motion
// is a compositor-only transform animation: no per-frame JS, no style recalc. Reduced
// motion keeps the tint and drops the motion (style.css).

import type { TermTheme } from "./derive.ts";
import { el } from "../dom.ts";

export type ShimmerLevel = "off" | "subtle" | "vivid";
export const SHIMMER_LEVELS: ShimmerLevel[] = ["off", "subtle", "vivid"];

/** Per level: layer opacity and seconds per cycle. */
const LEVEL: Record<Exclude<ShimmerLevel, "off">, { opacity: number; period: number }> = {
  subtle: { opacity: 0.22, period: 90 },
  vivid: { opacity: 0.5, period: 36 },
};

/** The band's hue stops, closing the loop on the first one. Brights read better on a
 *  dark scheme, normals on a light one. */
export function shimmerStops(t: TermTheme, dark: boolean): string[] {
  const keys = dark
    ? (["brightRed", "brightYellow", "brightGreen", "brightCyan", "brightBlue", "brightMagenta"] as const)
    : (["red", "yellow", "green", "cyan", "blue", "magenta"] as const);
  const stops = keys.map((k) => t[k]);
  return [...stops, stops[0]!];
}

export interface Shimmer {
  set(level: ShimmerLevel, theme: TermTheme, dark: boolean): void;
}

export function mountShimmer(): Shimmer {
  const layer = el("div", "shimmer");
  layer.setAttribute("aria-hidden", "true");
  const band = el("div", "shimmer-band");
  layer.append(band);
  document.body.append(layer);

  return {
    set(level, theme, dark) {
      if (level === "off") {
        layer.hidden = true;
        return;
      }
      const { opacity, period } = LEVEL[level];
      layer.hidden = false;
      layer.style.setProperty("--shimmer-opacity", String(opacity));
      layer.style.setProperty("--shimmer-period", `${period}s`);
      band.style.backgroundImage = `linear-gradient(90deg, ${shimmerStops(theme, dark).join(", ")})`;
    },
  };
}
