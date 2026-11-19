// The paste reader: one forgiving pass over the formats people copy schemes from.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseTheme, normalizeHex, completeTheme, pushRecent, slugFor, mapKey } from "./parse.ts";
import { schemeById } from "./schemes.ts";

const base = schemeById("warm-ink-dark")!.theme;

test("normalizeHex: the spellings schemes use", () => {
  assert.equal(normalizeHex("#ABC"), "#aabbcc");
  assert.equal(normalizeHex("0x1f1f28"), "#1f1f28");
  assert.equal(normalizeHex("'1f1f28'"), "#1f1f28");
  assert.equal(normalizeHex("#11223344"), "#112233");
  assert.equal(normalizeHex("rgb(255, 0, 16)"), "#ff0010");
  assert.equal(normalizeHex("tomato"), null);
  assert.equal(normalizeHex("#12345"), null);
});

test("kitty", () => {
  const { colors } = parseTheme(`
# a comment
foreground   #c0caf5
background   #1a1b26
selection_background #283457
cursor #c0caf5
cursor_text_color #1a1b26
color0 #15161e
color9 #f7768e
url_color #73daca
`);
  assert.deepEqual(colors, {
    foreground: "#c0caf5", background: "#1a1b26", selectionBackground: "#283457",
    cursor: "#c0caf5", cursorAccent: "#1a1b26", black: "#15161e", brightRed: "#f7768e",
  });
});

test("Ghostty", () => {
  const { colors } = parseTheme(`palette = 0=#45475a
palette = 15=#a6adc8
background = 1e1e2e
foreground = cdd6f4
cursor-color = f5e0dc
selection-background = 353749`);
  assert.equal(colors.black, "#45475a");
  assert.equal(colors.brightWhite, "#a6adc8");
  assert.equal(colors.background, "#1e1e2e");
  assert.equal(colors.cursor, "#f5e0dc");
  assert.equal(colors.selectionBackground, "#353749");
});

test("Alacritty TOML: sections decide normal vs bright; dim/search are ignored", () => {
  const { colors } = parseTheme(`
[colors.primary]
background = "0x1f1f28"
foreground = "0xdcd7ba"

[colors.cursor]
text = "#1f1f28"
cursor = "#c8c093"

[colors.selection]
background = "0x2d4f67"
foreground = "0xc8c093"

[colors.normal]
black = "0x090618"
red = "0xc34043"

[colors.bright]
black = "0x727169"
red = "0xe82424"

[colors.dim]
red = "#000000"

[colors.search.matches]
background = "#ffffff"
`);
  assert.deepEqual(colors, {
    background: "#1f1f28", foreground: "#dcd7ba", cursorAccent: "#1f1f28", cursor: "#c8c093",
    selectionBackground: "#2d4f67", black: "#090618", red: "#c34043",
    brightBlack: "#727169", brightRed: "#e82424",
  });
});

test("Alacritty YAML: indentation is the section", () => {
  const { colors } = parseTheme(`colors:
  primary:
    background: '#282828'
    foreground: '#ebdbb2'
  normal:
    red: '#cc241d'
  bright:
    red: '#fb4934'
`);
  assert.equal(colors.background, "#282828");
  assert.equal(colors.red, "#cc241d");
  assert.equal(colors.brightRed, "#fb4934");
});

test("Windows Terminal JSON: purple is magenta, name is kept", () => {
  const p = parseTheme(JSON.stringify({
    name: "Campbell", background: "#0C0C0C", foreground: "#CCCCCC", cursorColor: "#FFFFFF",
    selectionBackground: "#FFFFFF", purple: "#881798", brightPurple: "#B4009E", black: "#0C0C0C",
  }));
  assert.equal(p.name, "Campbell");
  assert.equal(p.colors.magenta, "#881798");
  assert.equal(p.colors.brightMagenta, "#b4009e");
  assert.equal(p.colors.cursor, "#ffffff");
});

test("VS Code workbench colors", () => {
  const { colors } = parseTheme(`{"workbench.colorCustomizations": {
    "terminal.background": "#002b36", "terminal.foreground": "#839496",
    "terminalCursor.foreground": "#93a1a1", "terminal.ansiBrightBlack": "#586e75",
    "terminal.selectionBackground": "#073642", "editor.background": "#ffffff"}}`);
  assert.deepEqual(colors, {
    background: "#002b36", foreground: "#839496", cursor: "#93a1a1",
    brightBlack: "#586e75", selectionBackground: "#073642",
  });
});

test("WezTerm arrays and Xresources and CSS variables", () => {
  const w = parseTheme(`[colors]
foreground = "#cdd6f4"
ansi = ["#45475a","#f38ba8","#a6e3a1","#f9e2af","#89b4fa","#f5c2e7","#94e2d5","#bac2de"]
brights = ["#585b70","#f38ba8","#a6e3a1","#f9e2af","#89b4fa","#f5c2e7","#94e2d5","#a6adc8"]`);
  assert.equal(w.colors.black, "#45475a");
  assert.equal(w.colors.brightBlack, "#585b70");
  assert.equal(w.colors.foreground, "#cdd6f4");

  const x = parseTheme(`! Xresources\n*.foreground: #c5c8c6\n*.color1:     #cc6666\n*color12: #81a2be`);
  assert.deepEqual(x.colors, { foreground: "#c5c8c6", red: "#cc6666", brightBlue: "#81a2be" });

  const c = parseTheme(`:root {\n  --background: #191724;\n  --red: #eb6f92;\n  --bright-cyan: #ebbcba;\n}`);
  assert.deepEqual(c.colors, { background: "#191724", red: "#eb6f92", brightCyan: "#ebbcba" });
});

test("a bare list of 16 colors is the ANSI palette", () => {
  const hexes = Array.from({ length: 16 }, (_, i) => `#0000${i.toString(16).padStart(2, "0")}`);
  const { colors } = parseTheme(hexes.join(" "));
  assert.equal(colors.black, "#000000");
  assert.equal(colors.brightWhite, "#00000f");
});

test("mapKey: an unrelated key maps to nothing", () => {
  assert.equal(mapKey("url_color"), null);
  assert.equal(mapKey("font_size"), null);
  assert.equal(mapKey("blackBright"), "brightBlack");
});

test("completeTheme fills from twins, then the base", () => {
  const t = completeTheme({ background: "#000000", foreground: "#ffffff", red: "#ff0000", brightBlue: "#0000ff" }, base);
  assert.equal(t.brightRed, "#ff0000", "bright from normal");
  assert.equal(t.blue, "#0000ff", "normal from bright");
  assert.equal(t.green, base.green, "neither: base");
  assert.equal(t.cursor, "#ffffff");
  assert.equal(t.cursorAccent, "#000000");
  assert.equal(t.selectionBackground, "#404040");
});

test("pushRecent and slugFor", () => {
  assert.deepEqual(pushRecent(["a", "b", "c"], "b"), ["b", "a", "c"]);
  assert.deepEqual(pushRecent(["a", "b"], "z", 2), ["z", "a"]);
  assert.equal(slugFor("Rosé Pine (mine)", new Set()), "rose-pine-mine");
  assert.equal(slugFor("Nord", new Set(["nord", "nord-2"])), "nord-3");
  assert.equal(slugFor("!!!", new Set()), "theme");
});
