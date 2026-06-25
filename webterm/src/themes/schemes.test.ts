// Validates the bundled scheme catalog: well-formed, correctly oriented
// (dark/light), legible (WCAG AA), and cleanly derivable into chrome tokens.
import { test } from "node:test";
import assert from "node:assert/strict";
import { SCHEMES, DEFAULT_SCHEME_ID, schemeById } from "./schemes.ts";
import { hexToOklch, deriveChrome } from "./derive.ts";
import type { TermTheme } from "./derive.ts";

const HEX_KEYS: (keyof TermTheme)[] = [
  "foreground", "background", "cursor",
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow",
  "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
];

const isHex = (s: string) => /^#[0-9a-fA-F]{6}$/.test(s);

// WCAG relative luminance + contrast ratio, from sRGB hex.
function luminance(hex: string): number {
  const n = parseInt(hex.replace("#", ""), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255]
    .map((v) => v / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}
function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

test("catalog: non-empty with unique ids", () => {
  assert.ok(SCHEMES.length > 0);
  const ids = SCHEMES.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length, "duplicate scheme id");
});

test("catalog: default scheme id resolves", () => {
  assert.ok(schemeById(DEFAULT_SCHEME_ID), `no scheme '${DEFAULT_SCHEME_ID}'`);
});

for (const s of SCHEMES) {
  test(`scheme[${s.id}]: every color is a 6-digit hex`, () => {
    for (const k of HEX_KEYS) {
      assert.ok(isHex(s.theme[k] as string), `${s.id}.${k} = ${s.theme[k]}`);
    }
  });

  test(`scheme[${s.id}]: dark flag matches background lightness`, () => {
    assert.equal(s.dark, hexToOklch(s.theme.background).L < 0.5);
  });

  test(`scheme[${s.id}]: foreground is legible on background (WCAG AA)`, () => {
    const ratio = contrast(s.theme.foreground, s.theme.background);
    assert.ok(ratio >= 4.5, `${s.id} contrast ${ratio.toFixed(2)} < 4.5`);
  });

  test(`scheme[${s.id}]: derives a full set of valid oklch chrome tokens`, () => {
    const tokens = deriveChrome(s.theme);
    assert.ok(Object.keys(tokens).length >= 25, "too few tokens");
    for (const [k, v] of Object.entries(tokens)) {
      assert.match(v, /^oklch\(/, `${s.id} ${k} not oklch: ${v}`);
    }
  });
}
