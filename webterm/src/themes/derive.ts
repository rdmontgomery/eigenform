// Derive the app's chrome tokens from a terminal color scheme.
//
// A scheme gives ~18 sRGB colors (16 ANSI + fg/bg/cursor/selection). The
// terminal half feeds xterm directly; this module computes the *chrome* half —
// every semantic CSS custom property the UI uses — in OKLCH, so the whole
// surface follows the chosen scheme. Pure and dependency-free.

/** The colors a scheme carries — a superset-compatible xterm ITheme. */
export interface TermTheme {
  foreground: string;
  background: string;
  cursor: string;
  cursorAccent?: string;
  selectionBackground?: string;
  black: string; red: string; green: string; yellow: string;
  blue: string; magenta: string; cyan: string; white: string;
  brightBlack: string; brightRed: string; brightGreen: string; brightYellow: string;
  brightBlue: string; brightMagenta: string; brightCyan: string; brightWhite: string;
}

export interface Oklch {
  /** Perceptual lightness, 0..1. */
  L: number;
  /** Chroma, 0..~0.4. */
  C: number;
  /** Hue in degrees, 0..360. */
  H: number;
}

interface Oklab { L: number; a: number; b: number }

/** Parse `#rgb`, `#rrggbb` (with or without `#`) into sRGB 0..1 components. */
function parseHex(hex: string): [number, number, number] {
  let h = hex.trim().replace(/^#/, "");
  if (h.length === 3) h = h[0]! + h[0]! + h[1]! + h[1]! + h[2]! + h[2]!;
  const n = parseInt(h, 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff].map((v) => v / 255) as [
    number,
    number,
    number,
  ];
}

const cbrt = Math.cbrt;

/** Convert an sRGB hex color to OKLab. */
function hexToOklab(hex: string): Oklab {
  const [r, g, b] = parseHex(hex).map((c) =>
    c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4,
  ) as [number, number, number];

  const l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
  const m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
  const s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
  const l_ = cbrt(l), m_ = cbrt(m), s_ = cbrt(s);

  return {
    L: 0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_,
    a: 1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_,
    b: 0.0259040371 * l_ + 0.7827717662 * m_ - 0.808675766 * s_,
  };
}

function labToLch({ L, a, b }: Oklab): Oklch {
  let H = (Math.atan2(b, a) * 180) / Math.PI;
  if (H < 0) H += 360;
  return { L, C: Math.hypot(a, b), H };
}

/** Convert an sRGB hex color to OKLCH. */
export function hexToOklch(hex: string): Oklch {
  return labToLch(hexToOklab(hex));
}

/** Format OKLCH as a CSS `oklch()` string, optionally with alpha. */
function lchToCss({ L, C, H }: Oklch, alpha = 1): string {
  const base = `oklch(${L.toFixed(4)} ${C.toFixed(4)} ${H.toFixed(2)}`;
  return alpha >= 1 ? `${base})` : `${base} / ${alpha})`;
}

const labToCss = (lab: Oklab, alpha?: number) => lchToCss(labToLch(lab), alpha);

/** Linear blend of two OKLab colors (t=0 → x, t=1 → y). */
function mixLab(x: Oklab, y: Oklab, t: number): Oklab {
  return { L: x.L + (y.L - x.L) * t, a: x.a + (y.a - x.a) * t, b: x.b + (y.b - x.b) * t };
}

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

/** The scheme's signature accent: its cursor, unless that's near-greyscale, in
 *  which case the most chromatic ANSI color stands in. */
function pickAccent(s: TermTheme): Oklch {
  const cursor = hexToOklch(s.cursor);
  if (cursor.C >= 0.04) return cursor;
  const candidates = [s.red, s.yellow, s.green, s.cyan, s.blue, s.magenta,
    s.brightRed, s.brightBlue, s.brightMagenta].map(hexToOklch);
  return candidates.reduce((best, c) => (c.C > best.C ? c : best));
}

/**
 * Derive every chrome CSS custom property from a scheme. Keys include the
 * leading `--`, so the caller can `setProperty(k, v)` directly.
 */
export function deriveChrome(s: TermTheme): Record<string, string> {
  const bg = hexToOklab(s.background);
  const fg = hexToOklab(s.foreground);
  const dark = bg.L < 0.5;

  // Surfaces climb from bg toward fg; text fades from fg toward bg.
  const surf = (t: number) => labToCss(mixLab(bg, fg, t));
  const text = (t: number) => labToCss(mixLab(fg, bg, t));

  const accent = pickAccent(s);

  // Functional colors: keep the ANSI hue, but pin lightness/chroma into a band
  // so they read consistently on whatever surface the scheme produced.
  const fnL = dark ? 0.72 : 0.52;
  const functional = (hex: string, c: number) =>
    lchToCss({ L: fnL, C: c, H: hexToOklch(hex).H });
  const inkL = dark ? 0.70 : 0.54;
  const ink = (hex: string) => lchToCss({ L: inkL, C: 0.105, H: hexToOklch(hex).H });

  return {
    "--bg": labToCss(bg),
    "--surface": surf(0.045),
    "--raised": surf(0.085),
    "--raised-2": surf(0.135),
    "--line": surf(0.22),
    "--line-soft": surf(0.13),
    "--term-bg": labToCss(bg),
    "--term-line": surf(0.20),

    "--tx": labToCss(fg),
    "--tx-2": text(0.28),
    "--tx-3": text(0.48),
    "--tx-faint": text(0.62),

    "--accent": lchToCss(accent),
    "--accent-2": lchToCss({ L: clamp(accent.L, 0.5, 0.8), C: accent.C * 0.55, H: accent.H }),
    "--accent-wash": lchToCss(accent, dark ? 0.16 : 0.12),

    "--st-running": functional(s.brightYellow, 0.13),
    "--st-idle": text(0.42),
    "--st-error": functional(s.brightRed, 0.15),
    "--st-success": functional(s.brightGreen, 0.11),

    "--ink-clay": ink(s.red),
    "--ink-ochre": ink(s.yellow),
    "--ink-olive": ink(s.green),
    "--ink-teal": ink(s.cyan),
    "--ink-slate": ink(s.blue),
    "--ink-plum": ink(s.magenta),
  };
}
