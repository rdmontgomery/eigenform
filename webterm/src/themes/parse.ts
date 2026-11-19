// Read a pasted terminal color scheme, whatever the format, into ITheme keys.
//
// One forgiving pass rather than a parser per format: every `key <sep> value` line
// (kitty, Ghostty, Xresources, CSS variables, TOML, YAML) and every string leaf of a
// JSON object (Windows Terminal, VS Code, xterm ITheme, iTerm2-Color-Schemes JSON)
// goes through the same key mapper, with the enclosing section (`[colors.bright]`,
// `bright:`, `"colors": {…}`) as context. WezTerm-style `ansi`/`brights` arrays and a
// bare list of 16 colors are understood too. Pure; tested in parse.test.ts.

import type { TermTheme } from "./derive.ts";

export type ThemeKey = keyof TermTheme;

const ANSI = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;
const bright = (n: string) => `bright${n[0]!.toUpperCase()}${n.slice(1)}` as ThemeKey;
/** ANSI index 0–15 → ITheme key. */
export const ANSI_KEYS: ThemeKey[] = [...ANSI, ...ANSI.map(bright)];

/** Every key a theme can carry, in editor order. */
export const THEME_KEYS: ThemeKey[] = [
  "background", "foreground", "cursor", "cursorAccent", "selectionBackground", ...ANSI_KEYS,
];

/** `#rgb`, `#rrggbb`, `#rrggbbaa`, `0xrrggbb`, bare `rrggbb`, `rgb(r, g, b)` →
 *  lowercase `#rrggbb`; null for anything else. */
export function normalizeHex(raw: string): string | null {
  const v = raw.trim().replace(/^['"]|['"]$/g, "").trim();
  const rgb = /^rgba?\(\s*(\d{1,3})\s*,?\s*(\d{1,3})\s*,?\s*(\d{1,3})/i.exec(v);
  if (rgb) {
    const parts = rgb.slice(1, 4).map(Number);
    if (parts.some((n) => n > 255)) return null;
    return `#${parts.map((n) => n.toString(16).padStart(2, "0")).join("")}`;
  }
  const m = /^(?:#|0x)?([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(v);
  if (!m) return null;
  let h = m[1]!.toLowerCase();
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  return `#${h.slice(0, 6)}`;
}

/** Sections whose keys aren't the scheme's main colors (Alacritty's dim/search/hints, …). */
const SKIP_SECTION = /dim|search|hint|vimode|footer|indexed|lineindicator|match|tab/;

/** Map one raw key, inside `section` (lowercased, alphanumeric path), to an ITheme key. */
export function mapKey(rawKey: string, section = ""): ThemeKey | null {
  if (SKIP_SECTION.test(section)) return null;
  let k = rawKey.toLowerCase().replace(/[^a-z0-9]/g, "");
  k = k.replace(/^terminalansi/, "").replace(/^terminal/, "").replace(/^ansi(?=[a-z])/, "");

  const idx = /^(?:color|colour|ansi|palette)(\d{1,2})$/.exec(k);
  if (idx) {
    const n = Number(idx[1]);
    return n < 16 ? ANSI_KEYS[n]! : null;
  }

  if (/cursor/.test(section) && !/cursor/.test(k)) {
    if (k === "cursor" || k === "color" || k === "background" || k === "bg") return "cursor";
    if (k === "text" || k === "foreground" || k === "fg") return "cursorAccent";
    return null;
  }
  if (/selection/.test(section) && !/selection/.test(k)) {
    return k === "background" || k === "bg" ? "selectionBackground" : null;
  }

  switch (k) {
    case "background": case "bg": case "primarybackground": case "backgroundcolor":
      return "background";
    case "foreground": case "fg": case "primaryforeground": case "foregroundcolor": case "text":
      return "foreground";
    case "cursor": case "cursorcolor": case "cursorbg": case "cursorbackground": case "cursorforeground":
      return "cursor";
    case "cursoraccent": case "cursortext": case "cursortextcolor": case "cursorfg":
      return "cursorAccent";
    case "selection": case "selectionbackground": case "selectionbg": case "selectionbackgroundcolor":
      return "selectionBackground";
  }

  const name = (s: string) => (s === "purple" ? "magenta" : s);
  const b = /^(?:bright|light)([a-z]+)$/.exec(k) ?? /^([a-z]+)bright$/.exec(k);
  if (b) {
    const n = name(b[1]!);
    return (ANSI as readonly string[]).includes(n) ? bright(n) : null;
  }
  const n = name(k);
  if ((ANSI as readonly string[]).includes(n)) {
    return /bright/.test(section) ? bright(n) : (n as ThemeKey);
  }
  return null;
}

export interface ParsedTheme {
  colors: Partial<TermTheme>;
  /** A name the paste carried (`name = "…"`, `"name": "…"`), if any. */
  name?: string;
}

/** Hex colors inside an array literal, in order. */
function hexList(s: string): string[] {
  return (s.match(/(?:#|0x)[0-9a-f]{3,8}\b/gi) ?? []).map((h) => normalizeHex(h)).filter((h): h is string => !!h);
}

/** Apply an array of colors under `key`: 8 → normals or brights, 16 → all. */
function applyList(out: Partial<TermTheme>, key: string, list: string[]) {
  const k = key.toLowerCase().replace(/[^a-z]/g, "");
  if (list.length >= 16 && /ansi|palette|colors?$/.test(k)) {
    list.slice(0, 16).forEach((h, i) => (out[ANSI_KEYS[i]!] = h));
  } else if (list.length === 8 && /bright/.test(k)) {
    list.forEach((h, i) => (out[bright(ANSI[i]!)] = h));
  } else if (list.length === 8 && /ansi|normal/.test(k)) {
    list.forEach((h, i) => (out[ANSI[i]!] = h));
  }
}

function walkJson(v: unknown, section: string, out: Partial<TermTheme>, meta: { name?: string }) {
  if (!v || typeof v !== "object" || Array.isArray(v)) return;
  for (const [key, val] of Object.entries(v as Record<string, unknown>)) {
    if (typeof val === "string") {
      if (/^name$/i.test(key) && !meta.name && !normalizeHex(val)) { meta.name = val; continue; }
      const t = mapKey(key, section);
      const hex = t && normalizeHex(val);
      if (t && hex) out[t] = hex;
    } else if (Array.isArray(val)) {
      applyList(out, key, val.filter((x): x is string => typeof x === "string").map((x) => normalizeHex(x)).filter((h): h is string => !!h));
    } else {
      walkJson(val, `${section}${key.toLowerCase().replace(/[^a-z0-9]/g, "")}`, out, meta);
    }
  }
}

function parseLines(text: string, out: Partial<TermTheme>, meta: { name?: string }) {
  const stack: { indent: number; name: string }[] = [];
  for (const raw of text.split(/\r?\n/)) {
    if (/^\s*(?:!|;|\/\/|#(?![0-9a-f]{3}))/i.test(raw) || !raw.trim()) continue;
    const indent = raw.length - raw.trimStart().length;

    const table = /^\s*\[+\s*([^\]]+?)\s*\]+\s*$/.exec(raw);
    if (table) {
      stack.length = 0;
      stack.push({ indent: -1, name: table[1]!.toLowerCase().replace(/[^a-z0-9]/g, "") });
      continue;
    }
    const m = /^\s*["']?([*\w.\-]+(?: [\w]+)*)["']?\s*(?:[:=]\s*|\s+)(.*?)\s*[,;]?\s*$/.exec(raw);
    if (!m) continue;
    const [, key, value] = m as unknown as [string, string, string];
    // YAML nesting: drop sections at or below this indent (TOML tables sit at -1).
    while (stack.length && stack[stack.length - 1]!.indent >= indent && stack[stack.length - 1]!.indent >= 0) stack.pop();
    if (value === "" || value === "{") {
      stack.push({ indent, name: key.toLowerCase().replace(/[^a-z0-9]/g, "") });
      continue;
    }
    const section = stack.map((s) => s.name).join("");

    if (/^name$/i.test(key) && !meta.name) {
      const n = value.replace(/^['"]|['"]$/g, "").trim();
      if (n && !normalizeHex(n)) meta.name = n;
      continue;
    }
    if (value.startsWith("[")) { applyList(out, key, hexList(value)); continue; }
    // Ghostty: `palette = 3=#c4a000`.
    const pal = /^(\d{1,2})\s*=\s*(.+)$/.exec(value);
    if (pal && /palette/i.test(key)) {
      const n = Number(pal[1]);
      const hex = normalizeHex(pal[2]!);
      if (n < 16 && hex) out[ANSI_KEYS[n]!] = hex;
      continue;
    }
    const t = mapKey(key, section);
    const hex = t && normalizeHex(value.replace(/\s+(?:#|\/\/).*$/, "").replace(/;$/, ""));
    if (t && hex) out[t] = hex;
  }
}

/** Read whatever was pasted. Unknown keys and non-colors are ignored. */
export function parseTheme(text: string): ParsedTheme {
  const out: Partial<TermTheme> = {};
  const meta: { name?: string } = {};
  const trimmed = text.trim();
  let json = false;
  if (/^[{[]/.test(trimmed)) {
    try {
      const v = JSON.parse(trimmed) as unknown;
      json = true;
      if (Array.isArray(v)) applyList(out, "ansi", v.filter((x): x is string => typeof x === "string").map((x) => normalizeHex(x)).filter((h): h is string => !!h));
      else walkJson(v, "", out, meta);
    } catch { /* not JSON: read it line by line */ }
  }
  if (!json) parseLines(text, out, meta);
  // Nothing keyed: a bare list of exactly 16 colors is the ANSI palette in order.
  if (!Object.keys(out).length) {
    const all = (text.match(/(?:#|0x)[0-9a-f]{6}\b/gi) ?? []).map((h) => normalizeHex(h)!);
    if (all.length === 16) all.forEach((h, i) => (out[ANSI_KEYS[i]!] = h));
  }
  return meta.name ? { colors: out, name: meta.name } : { colors: out };
}

/** Mix two hex colors in sRGB (`t` = share of `b`). */
export function mixHex(a: string, b: string, t: number): string {
  const p = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const [x, y] = [p(a), p(b)];
  return `#${x.map((v, i) => Math.round(v + (y[i]! - v) * t).toString(16).padStart(2, "0")).join("")}`;
}

/** Fill a partial theme into a full one: missing normals take their bright twin (and
 *  vice versa), then `base`; cursor falls back to the foreground, selection to a
 *  quarter-way mix of background toward foreground. */
export function completeTheme(p: Partial<TermTheme>, base: TermTheme): TermTheme {
  const background = p.background ?? base.background;
  const foreground = p.foreground ?? base.foreground;
  const t = { ...p, background, foreground } as TermTheme;
  for (const n of ANSI) {
    const b = bright(n);
    t[n as ThemeKey] = (p[n as ThemeKey] ?? p[b] ?? base[n as ThemeKey]) as string;
    t[b] = (p[b] ?? p[n as ThemeKey] ?? base[b]) as string;
  }
  t.cursor = p.cursor ?? foreground;
  t.cursorAccent = p.cursorAccent ?? background;
  t.selectionBackground = p.selectionBackground ?? mixHex(background, foreground, 0.25);
  return t;
}

/** Most recent first, deduped, at most `max`. */
export function pushRecent(list: readonly string[], id: string, max = 6): string[] {
  return [id, ...list.filter((x) => x !== id)].slice(0, max);
}

/** A stable id from a display name, unique against `taken`. */
export function slugFor(name: string, taken: ReadonlySet<string>): string {
  const base = name.toLowerCase().normalize("NFKD").replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "theme";
  let id = base;
  for (let i = 2; taken.has(id); i++) id = `${base}-${i}`;
  return id;
}
