// Custom themes: the user's edited copies of a scheme, persisted by the daemon
// (`/api/themes`). Pure: validation, the JSON export format, and import from that
// format, a bare palette, or an iTerm2 `.itermcolors` plist.
import { hexToOklch, OVERRIDE_KEYS } from "./derive.ts";
import type { TermTheme, ChromeOverrides } from "./derive.ts";
import type { Scheme } from "./schemes.ts";

/** Every palette color a theme carries, in editor order. */
export const PALETTE_KEYS = [
  "background", "foreground", "cursor", "selectionBackground",
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow",
  "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
] as const;
export type PaletteKey = (typeof PALETTE_KEYS)[number];

/** What the daemon stores and what an export file holds. */
export interface CustomTheme {
  id: string;
  name: string;
  base?: string;
  theme: Record<PaletteKey, string>;
  overrides?: ChromeOverrides;
}

export const EXPORT_FORMAT = "eigenform-theme/1";

/** `#RGB`, `#RRGGBB`, `RRGGBB` or `0xRRGGBB` → lowercase `#rrggbb`; null otherwise. */
export function normHex(v: unknown): string | null {
  if (typeof v !== "string") return null;
  let h = v.trim().toLowerCase().replace(/^(#|0x)/, "");
  if (/^[0-9a-f]{3}$/.test(h)) h = h.replace(/./g, (c) => c + c);
  return /^[0-9a-f]{6}$/.test(h) ? `#${h}` : null;
}

/** A url-safe id for a theme name, prefixed so it can never shadow a built-in. */
export function themeIdFor(name: string, taken: Iterable<string> = []): string {
  const slug = name.toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "").slice(0, 48) || "theme";
  const used = new Set(taken);
  let id = `custom-${slug}`;
  for (let n = 2; used.has(id); n++) id = `custom-${slug}-${n}`;
  return id;
}

/** A complete, normalized palette from a loose object; null when a color is missing or malformed. */
export function readPalette(raw: unknown): Record<PaletteKey, string> | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const out = {} as Record<PaletteKey, string>;
  for (const k of PALETTE_KEYS) {
    // Selection is optional on import: fall back to a fg/bg blend later.
    const hex = normHex(r[k]);
    if (!hex) {
      if (k === "selectionBackground") continue;
      return null;
    }
    out[k] = hex;
  }
  out.selectionBackground ??= blendHex(out.background, out.foreground, 0.2);
  return out;
}

function readOverrides(raw: unknown): ChromeOverrides | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const out: ChromeOverrides = {};
  for (const k of OVERRIDE_KEYS) {
    const hex = normHex((raw as Record<string, unknown>)[k]);
    if (hex) out[k] = hex;
  }
  return Object.keys(out).length ? out : undefined;
}

/** Validate a stored or imported custom theme; null when it isn't one. */
export function readCustomTheme(raw: unknown): CustomTheme | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const theme = readPalette(r.theme);
  if (!theme || typeof r.id !== "string" || typeof r.name !== "string") return null;
  const name = r.name.trim().slice(0, 80);
  if (!name || !/^[a-z0-9][a-z0-9-]{0,63}$/.test(r.id)) return null;
  const out: CustomTheme = { id: r.id, name, theme };
  if (typeof r.base === "string" && r.base) out.base = r.base;
  const overrides = readOverrides(r.overrides);
  if (overrides) out.overrides = overrides;
  return out;
}

/** A custom theme as a Scheme the picker and appearance can apply. */
export function customToScheme(c: CustomTheme): Scheme {
  const s: Scheme = {
    id: c.id,
    name: c.name,
    dark: hexToOklch(c.theme.background).L < 0.5,
    theme: { ...c.theme, cursorAccent: c.theme.background },
    custom: true,
  };
  if (c.base) s.base = c.base;
  if (c.overrides) s.overrides = { ...c.overrides };
  return s;
}

/** The palette of any scheme in custom-theme shape. */
export function paletteOf(t: TermTheme): Record<PaletteKey, string> {
  const out = {} as Record<PaletteKey, string>;
  for (const k of PALETTE_KEYS) {
    out[k] = normHex(t[k]) ?? (k === "selectionBackground" ? blendHex(t.background, t.foreground, 0.2) : "#000000");
  }
  return out;
}

/** The export file body for a theme. */
export function exportTheme(c: CustomTheme): string {
  const body: Record<string, unknown> = { format: EXPORT_FORMAT, name: c.name, theme: c.theme };
  if (c.base) body.base = c.base;
  if (c.overrides) body.overrides = c.overrides;
  return JSON.stringify(body, null, 2) + "\n";
}

/** What an import yields before it is given an id: a name and colors. */
export type ImportedTheme = Omit<CustomTheme, "id">;

/**
 * Parse an imported file. Accepts this app's export format, a bare palette object
 * (xterm ITheme keys), or an iTerm2 `.itermcolors` plist. Throws with a message
 * fit to show the user when the text is none of those.
 */
export function parseThemeFile(text: string, filename = ""): ImportedTheme {
  const fallbackName = filename.replace(/\.[^.]+$/, "").trim() || "Imported theme";
  if (/<plist[\s>]/.test(text) || /\.itermcolors$/i.test(filename)) {
    return { name: fallbackName, theme: parseItermColors(text) };
  }
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    throw new Error("not JSON or an .itermcolors file");
  }
  if (!raw || typeof raw !== "object") throw new Error("expected a JSON object");
  const r = raw as Record<string, unknown>;
  const palette = readPalette(r.theme ?? r);
  if (!palette) throw new Error(`missing or malformed colors (need ${PALETTE_KEYS.filter((k) => k !== "selectionBackground").join(", ")})`);
  const name = typeof r.name === "string" && r.name.trim() ? r.name.trim().slice(0, 80) : fallbackName;
  const out: ImportedTheme = { name, theme: palette };
  if (typeof r.base === "string" && r.base) out.base = r.base;
  const overrides = readOverrides(r.overrides);
  if (overrides) out.overrides = overrides;
  return out;
}

const ITERM_KEYS: Record<string, PaletteKey> = {
  "Background Color": "background",
  "Foreground Color": "foreground",
  "Cursor Color": "cursor",
  "Selection Color": "selectionBackground",
};
const ANSI_ORDER: PaletteKey[] = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow",
  "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
];
ANSI_ORDER.forEach((k, i) => (ITERM_KEYS[`Ansi ${i} Color`] = k));

/**
 * Read an iTerm2 `.itermcolors` plist. Components are taken as sRGB whatever the
 * file's declared color space; the difference is small next to a theme's intent.
 */
export function parseItermColors(xml: string): Record<PaletteKey, string> {
  const found: Partial<Record<PaletteKey, string>> = {};
  for (const m of xml.matchAll(/<key>([^<]+)<\/key>\s*<dict>([\s\S]*?)<\/dict>/g)) {
    const key = ITERM_KEYS[m[1]!.trim()];
    if (!key) continue;
    const comp = (name: string) => {
      const c = m[2]!.match(new RegExp(`<key>${name} Component</key>\\s*<(?:real|integer)>([^<]+)</`));
      const n = c ? Number(c[1]) : NaN;
      return Number.isFinite(n) ? Math.round(Math.min(1, Math.max(0, n)) * 255) : null;
    };
    const rgb = [comp("Red"), comp("Green"), comp("Blue")];
    if (rgb.some((v) => v === null)) continue;
    found[key] = "#" + rgb.map((v) => v!.toString(16).padStart(2, "0")).join("");
  }
  const palette = readPalette(found);
  if (!palette) {
    const missing = PALETTE_KEYS.filter((k) => k !== "selectionBackground" && !found[k]);
    throw new Error(`.itermcolors is missing ${missing.join(", ")}`);
  }
  return palette;
}

/** Linear sRGB-space blend of two hex colors, used only for a fallback selection. */
function blendHex(a: string, b: string, t: number): string {
  const pa = parseInt(a.slice(1), 16), pb = parseInt(b.slice(1), 16);
  const ch = (p: number, s: number) => (p >> s) & 255;
  return "#" + [16, 8, 0]
    .map((s) => Math.round(ch(pa, s) + (ch(pb, s) - ch(pa, s)) * t).toString(16).padStart(2, "0"))
    .join("");
}
