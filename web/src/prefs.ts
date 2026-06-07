// prefs.ts — tiny localStorage-backed UI preferences. The first persistence in
// woland; kept behind get/set so a private-mode SecurityError never breaks boot.
export type Density = "normal" | "compact";

const DENSITY_KEY = "woland.density";

export function loadDensity(): Density {
  try {
    return localStorage.getItem(DENSITY_KEY) === "compact" ? "compact" : "normal";
  } catch {
    return "normal";
  }
}

export function saveDensity(d: Density): void {
  try {
    localStorage.setItem(DENSITY_KEY, d);
  } catch {
    /* storage unavailable (private mode) — preference just won't persist */
  }
}

const FOREST_W_KEY = "woland.forestWidth";
export const FOREST_W_DEFAULT = 300;

export function clampForestWidth(px: number): number {
  return Math.max(200, Math.min(460, Math.round(px)));
}

export function loadForestWidth(): number {
  try {
    const v = parseInt(localStorage.getItem(FOREST_W_KEY) ?? "", 10);
    return Number.isFinite(v) ? clampForestWidth(v) : FOREST_W_DEFAULT;
  } catch {
    return FOREST_W_DEFAULT;
  }
}

export function saveForestWidth(px: number): void {
  try {
    localStorage.setItem(FOREST_W_KEY, String(clampForestWidth(px)));
  } catch {
    /* storage unavailable — width just won't persist */
  }
}
