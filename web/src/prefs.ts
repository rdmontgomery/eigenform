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
