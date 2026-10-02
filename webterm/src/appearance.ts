// Appearance: the color scheme and terminal typography, both persisted, plus the
// two top-bar popovers that edit them and the ⌘/Ctrl +/−/0 zoom keys.
//
// The shell owns the terminals, so it supplies the side effects: repaint every
// terminal on a scheme change, re-apply + refit on a font change, and re-render
// the top bar whenever a popover opens/closes or the scheme name changes.

import { DEFAULT_FONT } from "./pty.ts";
import type { FontSettings } from "./pty.ts";
import { SCHEMES, DEFAULT_SCHEME_ID, schemeById } from "./themes/schemes.ts";
import type { Scheme } from "./themes/schemes.ts";
import { deriveChrome } from "./themes/derive.ts";
import { el } from "./dom.ts";

const LS_THEME = "eigenform:term:theme:v1"; // legacy light/dark — migrated to v2
const LS_SCHEME = "eigenform:term:theme:v2"; // scheme id
const LS_FONT = "eigenform:term:font:v1";

/** Resolve the active scheme: stored v2 id → migrated v1 light/dark → default. */
function loadSchemeId(): string {
  const v2 = localStorage.getItem(LS_SCHEME);
  if (v2 && schemeById(v2)) return v2;
  const v1 = localStorage.getItem(LS_THEME);
  if (v1 === "light") return "warm-ink-light";
  if (v1 === "dark") return "warm-ink-dark";
  return DEFAULT_SCHEME_ID;
}

/** Terminal typefaces offered in the font popover. macOS-first: "System Mono"
 *  resolves to SF Mono / Menlo with no webfont round-trip. */
const TERM_FACES: { label: string; stack: string }[] = [
  { label: "Plex Mono", stack: DEFAULT_FONT.family },
  {
    label: "System Mono",
    stack: 'ui-monospace, "SF Mono", Menlo, "Cascadia Mono", Consolas, monospace',
  },
];

/** Bounds for the font controls (and ⌘ +/− zoom). */
const FONT_BOUNDS = {
  size: { min: 9, max: 24, step: 0.5 },
  lineHeight: { min: 1.0, max: 2.0, step: 0.05 },
  letterSpacing: { min: -2, max: 4, step: 0.5 },
};

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Validate one persisted numeric field, falling back to a default. */
function numField(v: unknown, fallback: number, lo: number, hi: number): number {
  return typeof v === "number" && Number.isFinite(v) ? clamp(v, lo, hi) : fallback;
}

function loadFont(): FontSettings {
  try {
    const raw = JSON.parse(localStorage.getItem(LS_FONT) ?? "{}") as Partial<FontSettings>;
    return {
      family: typeof raw.family === "string" && raw.family ? raw.family : DEFAULT_FONT.family,
      size: numField(raw.size, DEFAULT_FONT.size, FONT_BOUNDS.size.min, FONT_BOUNDS.size.max),
      lineHeight: numField(raw.lineHeight, DEFAULT_FONT.lineHeight, FONT_BOUNDS.lineHeight.min, FONT_BOUNDS.lineHeight.max),
      letterSpacing: numField(raw.letterSpacing, DEFAULT_FONT.letterSpacing, FONT_BOUNDS.letterSpacing.min, FONT_BOUNDS.letterSpacing.max),
    };
  } catch {
    return { ...DEFAULT_FONT };
  }
}

function applyChrome(s: Scheme) {
  const root = document.documentElement;
  for (const [k, v] of Object.entries(deriveChrome(s.theme))) {
    root.style.setProperty(k, v);
  }
  root.style.colorScheme = s.dark ? "dark" : "light";
}

interface Popover {
  isOpen(): boolean;
  toggle(anchor: HTMLElement): void;
  /** Rebuild the body from current state (no-op while closed). */
  rerender(): void;
}

/**
 * A top-bar popover anchored under its button: closes on Escape or on a pointer
 * down outside both itself and its button (capture phase, so a click on a
 * tab/terminal closes it before that target acts).
 */
function createPopover(
  className: string,
  buttonSelector: string,
  render: (pop: HTMLElement) => void,
  onOpenChange: () => void,
): Popover {
  let pop: HTMLElement | null = null;

  const onOutside = (e: PointerEvent) => {
    const t = e.target as HTMLElement;
    if (pop && !pop.contains(t) && !t.closest(buttonSelector)) close();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") { e.preventDefault(); close(); }
  };

  function open(anchor: HTMLElement) {
    pop = el("div", className);
    document.body.append(pop);
    const r = anchor.getBoundingClientRect();
    pop.style.top = `${Math.round(r.bottom + 6)}px`;
    pop.style.right = `${Math.round(window.innerWidth - r.right)}px`;
    rerender();
    onOpenChange();
    document.addEventListener("pointerdown", onOutside, true);
    document.addEventListener("keydown", onKey, true);
  }

  function close() {
    pop?.remove();
    pop = null;
    document.removeEventListener("pointerdown", onOutside, true);
    document.removeEventListener("keydown", onKey, true);
    onOpenChange();
  }

  function rerender() {
    if (!pop) return;
    pop.innerHTML = "";
    render(pop);
  }

  return {
    isOpen: () => pop !== null,
    toggle: (anchor) => (pop ? close() : open(anchor)),
    rerender,
  };
}

// The 6 ANSI hues shown in a swatch strip — a quick read of a scheme's palette.
const SWATCH_KEYS = ["red", "yellow", "green", "cyan", "blue", "magenta"] as const;

export interface Appearance {
  scheme(): Scheme;
  font(): FontSettings;
  themePopover: Popover;
  fontPopover: Popover;
}

/**
 * Load the persisted scheme + font and paint the chrome immediately (call before
 * any layout so first paint is correct). Registers the global zoom keys.
 */
export function createAppearance(hooks: {
  /** Repaint every open terminal with the new scheme. */
  onScheme: (s: Scheme) => void;
  /** Re-apply the font to every open terminal (and refit). */
  onFont: (f: FontSettings) => void;
  /** Re-render the top bar (popover open state, theme title). */
  onChromeChange: () => void;
}): Appearance {
  let scheme: Scheme = schemeById(loadSchemeId()) ?? SCHEMES[0]!;
  applyChrome(scheme);
  let font = loadFont();

  /** Paint the whole surface from a scheme: chrome tokens on :root + every
   *  open terminal's colors. Persisted so it survives reload. */
  function applyScheme(next: Scheme) {
    scheme = next;
    localStorage.setItem(LS_SCHEME, next.id);
    applyChrome(next);
    hooks.onScheme(next);
    hooks.onChromeChange();
    themePopover.rerender();
  }

  /** Merge a patch into the live font settings, persist, and re-lay every grid. */
  function setFont(patch: Partial<FontSettings>) {
    font = { ...font, ...patch };
    localStorage.setItem(LS_FONT, JSON.stringify(font));
    hooks.onFont(font);
  }

  function bumpFontSize(delta: number) {
    const b = FONT_BOUNDS.size;
    setFont({ size: clamp(Math.round((font.size + delta) * 4) / 4, b.min, b.max) });
    fontPopover.rerender();
  }

  // ⌘/Ctrl +/−/0 — terminal zoom (overrides browser page zoom, which is the
  // wrong granularity for a grid we own).
  window.addEventListener("keydown", (e) => {
    if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
    if (e.key === "=" || e.key === "+") { e.preventDefault(); bumpFontSize(0.5); }
    else if (e.key === "-" || e.key === "_") { e.preventDefault(); bumpFontSize(-0.5); }
    else if (e.key === "0") { e.preventDefault(); setFont({ size: DEFAULT_FONT.size }); fontPopover.rerender(); }
  });

  /** Font popover — typeface + size + line-height + letter-spacing. */
  const fontPopover = createPopover(
    "font-pop",
    ".font-btn",
    (pop) => {
      // Typeface — a row of pills; the active stack is highlighted.
      const faceRow = el("div", "font-faces");
      for (const f of TERM_FACES) {
        const pill = el("button", `font-face${font.family === f.stack ? " font-face--on" : ""}`);
        pill.textContent = f.label;
        pill.style.fontFamily = f.stack;
        pill.addEventListener("click", () => { setFont({ family: f.stack }); fontPopover.rerender(); });
        faceRow.append(pill);
      }
      pop.append(faceRow);

      const stepper = (
        label: string,
        value: number,
        fmt: (n: number) => string,
        key: keyof FontSettings,
        bounds: { min: number; max: number; step: number },
      ) => {
        const row = el("div", "font-pop-row");
        const name = el("span", "font-pop-label");
        name.textContent = label;
        const ctl = el("div", "font-step");
        const dec = el("button");
        dec.textContent = "−";
        const val = el("span", "font-step-val");
        val.textContent = fmt(value);
        const inc = el("button");
        inc.textContent = "+";
        const set = (n: number) => {
          setFont({ [key]: clamp(Math.round(n / bounds.step) * bounds.step, bounds.min, bounds.max) } as Partial<FontSettings>);
          fontPopover.rerender();
        };
        dec.addEventListener("click", () => set(value - bounds.step));
        inc.addEventListener("click", () => set(value + bounds.step));
        ctl.append(dec, val, inc);
        row.append(name, ctl);
        return row;
      };

      pop.append(
        stepper("Size", font.size, (n) => `${n}px`, "size", FONT_BOUNDS.size),
        stepper("Line height", font.lineHeight, (n) => n.toFixed(2), "lineHeight", FONT_BOUNDS.lineHeight),
        stepper("Tracking", font.letterSpacing, (n) => `${n}px`, "letterSpacing", FONT_BOUNDS.letterSpacing),
      );

      const reset = el("button", "font-pop-reset");
      reset.textContent = "Reset to defaults";
      reset.addEventListener("click", () => { setFont({ ...DEFAULT_FONT }); fontPopover.rerender(); });
      pop.append(reset);
    },
    hooks.onChromeChange,
  );

  /** Theme popover — pick a scheme by sight (live swatch strips). */
  const themePopover = createPopover(
    "theme-pop",
    ".theme-btn",
    (pop) => {
      for (const s of SCHEMES) {
        const row = el("button", `theme-row${s.id === scheme.id ? " theme-row--on" : ""}`);

        const swatch = el("div", "theme-swatch");
        swatch.style.background = s.theme.background;
        for (const k of SWATCH_KEYS) {
          const dot = el("span", "theme-dot");
          dot.style.background = s.theme[k];
          swatch.append(dot);
        }
        const fg = el("span", "theme-dot theme-dot--fg");
        fg.style.background = s.theme.foreground;
        swatch.append(fg);

        const name = el("span", "theme-row-name");
        name.textContent = s.name;

        row.append(swatch, name);
        row.addEventListener("click", () => applyScheme(s));
        pop.append(row);
      }
    },
    hooks.onChromeChange,
  );

  return {
    scheme: () => scheme,
    font: () => font,
    themePopover,
    fontPopover,
  };
}
