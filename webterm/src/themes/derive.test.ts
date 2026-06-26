// Tests for chrome-token derivation from a terminal color scheme.
// Run: `node --test` (native TS strip, Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { hexToOklch, deriveChrome } from "./derive.ts";
import type { TermTheme } from "./derive.ts";

// Reference OKLCH values are the well-known sRGB conversions (per the OKLab
// spec / culori). Tolerances are loose enough for float drift, tight enough to
// catch a wrong matrix.
function approx(actual: number, expected: number, tol: number, label: string) {
  assert.ok(
    Math.abs(actual - expected) <= tol,
    `${label}: expected ~${expected}, got ${actual}`,
  );
}

test("hexToOklch: pure red", () => {
  const c = hexToOklch("#ff0000");
  approx(c.L, 0.6279, 0.002, "L");
  approx(c.C, 0.2577, 0.003, "C");
  approx(c.H, 29.23, 0.5, "H");
});

test("hexToOklch: white is L≈1, near-zero chroma", () => {
  const c = hexToOklch("#ffffff");
  approx(c.L, 1.0, 0.002, "L");
  approx(c.C, 0.0, 0.002, "C");
});

test("hexToOklch: black is L≈0", () => {
  const c = hexToOklch("#000000");
  approx(c.L, 0.0, 0.002, "L");
});

test("hexToOklch: tolerates 3-digit hex and missing '#'", () => {
  const a = hexToOklch("#ff0000");
  const b = hexToOklch("f00");
  approx(b.L, a.L, 0.001, "L");
  approx(b.C, a.C, 0.001, "C");
});

// ── deriveChrome ───────────────────────────────────────────────────────────

const DARK: TermTheme = {
  background: "#1d2021", foreground: "#ebdbb2", cursor: "#fe8019",
  cursorAccent: "#1d2021", selectionBackground: "#504945",
  black: "#282828", red: "#cc241d", green: "#98971a", yellow: "#d79921",
  blue: "#458588", magenta: "#b16286", cyan: "#689d6a", white: "#a89984",
  brightBlack: "#928374", brightRed: "#fb4934", brightGreen: "#b8bb26",
  brightYellow: "#fabd2f", brightBlue: "#83a598", brightMagenta: "#d3869b",
  brightCyan: "#8ec07c", brightWhite: "#ebdbb2",
};

const LIGHT: TermTheme = {
  background: "#fdf6e3", foreground: "#657b83", cursor: "#586e75",
  cursorAccent: "#fdf6e3", selectionBackground: "#eee8d5",
  black: "#073642", red: "#dc322f", green: "#859900", yellow: "#b58900",
  blue: "#268bd2", magenta: "#d33682", cyan: "#2aa198", white: "#eee8d5",
  brightBlack: "#586e75", brightRed: "#cb4b16", brightGreen: "#586e75",
  brightYellow: "#657b83", brightBlue: "#839496", brightMagenta: "#6c71c4",
  brightCyan: "#93a1a1", brightWhite: "#fdf6e3",
};

const REQUIRED_TOKENS = [
  "--bg", "--surface", "--raised", "--raised-2", "--line", "--line-soft",
  "--term-bg", "--term-line",
  "--tx", "--tx-2", "--tx-3", "--tx-faint",
  "--accent", "--accent-2", "--accent-wash",
  "--st-running", "--st-idle", "--st-error", "--st-success",
  "--ink-clay", "--ink-ochre", "--ink-olive", "--ink-teal", "--ink-slate", "--ink-plum",
];

/** Pull L/C/H back out of an `oklch(L C H[ / a])` string for assertions. */
function parseOklch(s: string): { L: number; C: number; H: number; a: number } {
  const m = s.match(/oklch\(\s*([\d.]+)\s+([\d.]+)\s+([\d.]+)\s*(?:\/\s*([\d.]+)\s*)?\)/);
  assert.ok(m, `not an oklch() string: ${s}`);
  return { L: +m![1]!, C: +m![2]!, H: +m![3]!, a: m![4] ? +m![4]! : 1 };
}
const Lof = (tokens: Record<string, string>, k: string) => parseOklch(tokens[k]!).L;

for (const [name, scheme, dark] of [["dark", DARK, true], ["light", LIGHT, false]] as const) {
  test(`deriveChrome[${name}]: emits every chrome token as a valid oklch() string`, () => {
    const t = deriveChrome(scheme);
    for (const key of REQUIRED_TOKENS) {
      assert.ok(key in t, `missing token ${key}`);
      parseOklch(t[key]!); // throws if malformed
    }
  });

  test(`deriveChrome[${name}]: --bg tracks the scheme background lightness`, () => {
    const t = deriveChrome(scheme);
    approx(Lof(t, "--bg"), hexToOklch(scheme.background).L, 0.02, "bg L");
  });

  test(`deriveChrome[${name}]: surface ladder steps monotonically away from bg`, () => {
    const t = deriveChrome(scheme);
    const bg = Lof(t, "--bg");
    const ladder = ["--surface", "--raised", "--raised-2", "--line"].map((k) =>
      Math.abs(Lof(t, k) - bg),
    );
    for (let i = 1; i < ladder.length; i++) {
      assert.ok(ladder[i]! > ladder[i - 1]!, `${name} ladder not increasing at step ${i}: ${ladder}`);
    }
  });

  test(`deriveChrome[${name}]: text contrasts with background`, () => {
    const t = deriveChrome(scheme);
    assert.ok(Math.abs(Lof(t, "--tx") - Lof(t, "--bg")) >= 0.4, "tx/bg lightness gap too small");
  });

  test(`deriveChrome[${name}]: dark/light orientation of --bg`, () => {
    const t = deriveChrome(scheme);
    assert.equal(Lof(t, "--bg") < 0.5, dark);
  });

  test(`deriveChrome[${name}]: the six ink hues are mutually distinguishable`, () => {
    const t = deriveChrome(scheme);
    const hues = ["--ink-clay", "--ink-ochre", "--ink-olive", "--ink-teal", "--ink-slate", "--ink-plum"]
      .map((k) => parseOklch(t[k]!).H);
    for (let i = 0; i < hues.length; i++) {
      for (let j = i + 1; j < hues.length; j++) {
        const d = Math.abs(hues[i]! - hues[j]!);
        const sep = Math.min(d, 360 - d);
        assert.ok(sep >= 12, `inks ${i},${j} too close: ${hues[i]}° vs ${hues[j]}°`);
      }
    }
  });
}
