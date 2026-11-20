import { test } from "node:test";
import assert from "node:assert/strict";
import {
  normHex, themeIdFor, readCustomTheme, customToScheme, exportTheme, parseThemeFile,
  parseItermColors, paletteOf, PALETTE_KEYS,
} from "./custom.ts";
import { SCHEMES } from "./schemes.ts";
import { deriveChrome } from "./derive.ts";

const dracula = SCHEMES.find((s) => s.id === "dracula")!;
const palette = paletteOf(dracula.theme);

test("normHex: accepts the common spellings, rejects the rest", () => {
  assert.equal(normHex("#ABCDEF"), "#abcdef");
  assert.equal(normHex("abc"), "#aabbcc");
  assert.equal(normHex("0x100F0F"), "#100f0f");
  assert.equal(normHex("#abcd"), null);
  assert.equal(normHex("red"), null);
  assert.equal(normHex(12), null);
});

test("themeIdFor: slugs the name and avoids taken ids", () => {
  assert.equal(themeIdFor("Dracula (mine)"), "custom-dracula-mine");
  assert.equal(themeIdFor("Rosé Pine"), "custom-rose-pine");
  assert.equal(themeIdFor("!!!"), "custom-theme");
  assert.equal(themeIdFor("x", ["custom-x", "custom-x-2"]), "custom-x-3");
});

test("readCustomTheme: round-trips a valid theme and drops junk overrides", () => {
  const t = readCustomTheme({
    id: "custom-a", name: "  A ", base: "dracula", theme: palette,
    overrides: { accent: "#FF0000", running: "nope", bogus: "#000000" },
  });
  assert.deepEqual(t, { id: "custom-a", name: "A", base: "dracula", theme: palette, overrides: { accent: "#ff0000" } });
});

test("readCustomTheme: rejects bad ids, empty names and incomplete palettes", () => {
  assert.equal(readCustomTheme({ id: "../x", name: "x", theme: palette }), null);
  assert.equal(readCustomTheme({ id: "custom-x", name: " ", theme: palette }), null);
  const { red: _red, ...noRed } = palette;
  assert.equal(readCustomTheme({ id: "custom-x", name: "x", theme: noRed }), null);
  assert.equal(readCustomTheme(null), null);
});

test("customToScheme: computes darkness and marks it custom", () => {
  const s = customToScheme({ id: "custom-a", name: "A", base: "dracula", theme: palette });
  assert.equal(s.dark, true);
  assert.equal(s.custom, true);
  assert.equal(s.base, "dracula");
  assert.equal(s.theme.cursorAccent, palette.background);
  const light = customToScheme({ id: "custom-b", name: "B", theme: { ...palette, background: "#fafafa" } });
  assert.equal(light.dark, false);
});

test("export → import round-trips name, colors, base and overrides", () => {
  const c = { id: "custom-a", name: "Mine", base: "dracula", theme: palette, overrides: { error: "#ff0000" } };
  const back = parseThemeFile(exportTheme(c), "whatever.json");
  assert.deepEqual(back, { name: "Mine", base: "dracula", theme: palette, overrides: { error: "#ff0000" } });
});

test("import: a bare palette takes its name from the file and gets a selection", () => {
  const { selectionBackground: _sel, ...bare } = palette;
  const t = parseThemeFile(JSON.stringify(bare), "My Theme.json");
  assert.equal(t.name, "My Theme");
  assert.match(t.theme.selectionBackground, /^#[0-9a-f]{6}$/);
});

test("import: explains what is wrong", () => {
  assert.throws(() => parseThemeFile("{", "x.json"), /not JSON/);
  assert.throws(() => parseThemeFile(JSON.stringify({ name: "x", theme: { red: "#f00" } })), /missing or malformed/);
});

function plistColor(key: string, r: number, g: number, b: number) {
  return `<key>${key}</key>
  <dict>
    <key>Alpha Component</key><real>1</real>
    <key>Blue Component</key><real>${b}</real>
    <key>Color Space</key><string>sRGB</string>
    <key>Green Component</key><real>${g}</real>
    <key>Red Component</key><real>${r}</real>
  </dict>`;
}

test("parseItermColors: reads every slot from an iTerm2 plist", () => {
  const entries = [plistColor("Background Color", 0, 0, 0), plistColor("Foreground Color", 1, 1, 1),
    plistColor("Cursor Color", 1, 0.5, 0), plistColor("Selection Color", 0.2, 0.2, 0.2)];
  for (let i = 0; i < 16; i++) entries.push(plistColor(`Ansi ${i} Color`, i / 15, 0, 1 - i / 15));
  const xml = `<?xml version="1.0"?><plist version="1.0"><dict>${entries.join("\n")}</dict></plist>`;
  const p = parseItermColors(xml);
  assert.equal(p.background, "#000000");
  assert.equal(p.foreground, "#ffffff");
  assert.equal(p.cursor, "#ff8000");
  assert.equal(p.black, "#0000ff");
  assert.equal(p.brightWhite, "#ff0000");
  assert.deepEqual(Object.keys(p).sort(), [...PALETTE_KEYS].sort());
  assert.equal(parseThemeFile(xml, "Night.itermcolors").name, "Night");
});

test("parseItermColors: names what's missing", () => {
  const xml = `<plist><dict>${plistColor("Background Color", 0, 0, 0)}</dict></plist>`;
  assert.throws(() => parseItermColors(xml), /missing foreground, cursor, black/);
});

test("deriveChrome: overrides pin accent and status colors", () => {
  const base = deriveChrome(dracula.theme);
  const pinned = deriveChrome(dracula.theme, { accent: "#ff0000", error: "#00ff00" });
  assert.notEqual(pinned["--accent"], base["--accent"]);
  assert.match(pinned["--accent"]!, /^oklch\(0\.62\d\d 0\.257\d 29\./);
  assert.notEqual(pinned["--accent-wash"], base["--accent-wash"], "wash follows the pinned accent");
  assert.match(pinned["--st-error"]!, /^oklch\(0\.86/);
  assert.equal(pinned["--st-running"], base["--st-running"], "unpinned tokens still derive");
});

test("oklchToHex inverts hexToOklch", async () => {
  const { oklchToHex, hexToOklch, parseOklch } = await import("./derive.ts");
  for (const hex of ["#ff0000", "#3fb950", "#191724", "#ffc799", "#ffffff", "#000000"]) {
    assert.equal(oklchToHex(hexToOklch(hex)), hex);
  }
  assert.deepEqual(parseOklch("oklch(0.5 0.1 200 / 0.16)"), { L: 0.5, C: 0.1, H: 200 });
  assert.equal(parseOklch("red"), null);
});
