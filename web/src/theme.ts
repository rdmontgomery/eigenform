// theme.ts — the two grounds. Palettes live as CSS custom properties in woland.css
// under [data-theme="furnace"|"paper"]; toggling is one attribute write, so nothing
// restyles in JS. The hex values are mirrored here only for consumers that can't read
// CSS vars — notably the xterm terminal, which wants concrete colors.

export type ThemeName = "furnace" | "paper";

export interface Palette {
  bg: string;
  panel: string;
  furnaceBg: string;
  ink: string;
  agent: string;
  dim: string;
  faint: string;
  amber: string;
  cold: string;
  cool: string;
}

export const PALETTES: Record<ThemeName, Palette> = {
  furnace: {
    bg: "#15120d", panel: "#100d09", furnaceBg: "#0c0a07",
    ink: "#ebe4d4", agent: "#aeb3a8", dim: "#8b8474", faint: "#5a5345",
    amber: "#e0902e", cold: "#5f8bb0", cool: "#6f9b94",
  },
  paper: {
    bg: "#efe9dc", panel: "#e9e2d2", furnaceBg: "#23201a",
    ink: "#27231c", agent: "#6a6051", dim: "#938878", faint: "#b3a890",
    amber: "#b35f29", cold: "#3f6f8e", cool: "#4d756a",
  },
};

export function applyTheme(name: ThemeName): void {
  document.documentElement.dataset.theme = name;
}

export function currentTheme(): ThemeName {
  return (document.documentElement.dataset.theme as ThemeName) || "furnace";
}
