// Built-in terminal color schemes. Each is a full xterm ITheme (16 ANSI +
// fg/bg/cursor/selection); deriveChrome() turns it into the whole app's chrome.
//
// Picked for agent TUIs (Claude Code and friends): the 16 ANSI colors come from
// each theme's own upstream terminal port, noted per scheme. Where we depart from
// upstream it is called out: an `accent` override when the upstream cursor is grey
// (so the derived accent would otherwise land on an alarming red), or a
// selection color when upstream inverts the selection (xterm has no selection fg
// here, so an opaque light selection would hide the text).
import type { TermTheme, ChromeOverrides } from "./derive.ts";

export interface Scheme {
  /** Stable id, persisted in localStorage. */
  id: string;
  /** Human label shown in the picker. */
  name: string;
  /** Whether the background is dark (drives color-scheme). */
  dark: boolean;
  theme: TermTheme;
  /** Chrome tokens set directly instead of derived from the palette. */
  overrides?: ChromeOverrides;
  /** A user's saved theme (persisted by the daemon), not a built-in. */
  custom?: boolean;
  /** The built-in a custom theme started from; "reset to original" restores it. */
  base?: string;
}

export const SCHEMES: Scheme[] = [
  {
    // This app's own identity, and the default.
    id: "warm-ink-dark",
    name: "Warm Ink",
    dark: true,
    theme: {
      background: "#110b07", foreground: "#e9e0d4", cursor: "#df8353", cursorAccent: "#110b07",
      selectionBackground: "#3a2a20",
      black: "#2a201a", red: "#d65f4e", green: "#9aa05a", yellow: "#d9a066",
      blue: "#6f93a8", magenta: "#c08aa8", cyan: "#7fae9e", white: "#cabfae",
      brightBlack: "#6a5d50", brightRed: "#e8775f", brightGreen: "#b3b96f", brightYellow: "#f0b97a",
      brightBlue: "#8aa9bd", brightMagenta: "#d6a0bd", brightCyan: "#97c3b2", brightWhite: "#f3ead9",
    },
  },
  {
    // github.com/morhetz/gruvbox (via alacritty-theme gruvbox_dark.toml).
    id: "gruvbox-dark",
    name: "Gruvbox Dark",
    dark: true,
    theme: {
      background: "#282828", foreground: "#ebdbb2", cursor: "#fe8019", cursorAccent: "#282828",
      selectionBackground: "#504945",
      black: "#282828", red: "#cc241d", green: "#98971a", yellow: "#d79921",
      blue: "#458588", magenta: "#b16286", cyan: "#689d6a", white: "#a89984",
      brightBlack: "#928374", brightRed: "#fb4934", brightGreen: "#b8bb26", brightYellow: "#fabd2f",
      brightBlue: "#83a598", brightMagenta: "#d3869b", brightCyan: "#8ec07c", brightWhite: "#ebdbb2",
    },
  },
  {
    // github.com/dracula/alacritty. Cursor is Dracula purple rather than upstream's
    // foreground, so the chrome accent stays purple.
    id: "dracula",
    name: "Dracula",
    dark: true,
    theme: {
      background: "#282a36", foreground: "#f8f8f2", cursor: "#bd93f9", cursorAccent: "#282a36",
      selectionBackground: "#44475a",
      black: "#21222c", red: "#ff5555", green: "#50fa7b", yellow: "#f1fa8c",
      blue: "#bd93f9", magenta: "#ff79c6", cyan: "#8be9fd", white: "#f8f8f2",
      brightBlack: "#6272a4", brightRed: "#ff6e6e", brightGreen: "#69ff94", brightYellow: "#ffffa5",
      brightBlue: "#d6acff", brightMagenta: "#ff92df", brightCyan: "#a4ffff", brightWhite: "#ffffff",
    },
  },
  {
    // github.com/folke/tokyonight.nvim extras/alacritty/tokyonight_night.toml.
    id: "tokyo-night",
    name: "Tokyo Night",
    dark: true,
    theme: {
      background: "#1a1b26", foreground: "#c0caf5", cursor: "#7aa2f7", cursorAccent: "#1a1b26",
      selectionBackground: "#283457",
      black: "#15161e", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#ff899d", brightGreen: "#9fe044", brightYellow: "#faba4a",
      brightBlue: "#8db0ff", brightMagenta: "#c7a9ff", brightCyan: "#a4daff", brightWhite: "#c0caf5",
    },
  },
  {
    // github.com/folke/tokyonight.nvim extras/alacritty/tokyonight_storm.toml.
    id: "tokyo-night-storm",
    name: "Tokyo Night Storm",
    dark: true,
    theme: {
      background: "#24283b", foreground: "#c0caf5", cursor: "#7aa2f7", cursorAccent: "#24283b",
      selectionBackground: "#2e3c64",
      black: "#1d202f", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#ff899d", brightGreen: "#9fe044", brightYellow: "#faba4a",
      brightBlue: "#8db0ff", brightMagenta: "#c7a9ff", brightCyan: "#a4daff", brightWhite: "#c0caf5",
    },
  },
  {
    // github.com/catppuccin/alacritty catppuccin-mocha.toml. Cursor is mauve (not
    // rosewater) for a chromatic accent; selection is surface2, since upstream's
    // opaque rosewater selection needs a selection foreground xterm isn't given.
    id: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    dark: true,
    theme: {
      background: "#1e1e2e", foreground: "#cdd6f4", cursor: "#cba6f7", cursorAccent: "#1e1e2e",
      selectionBackground: "#585b70",
      black: "#45475a", red: "#f38ba8", green: "#a6e3a1", yellow: "#f9e2af",
      blue: "#89b4fa", magenta: "#f5c2e7", cyan: "#94e2d5", white: "#a6adc8",
      brightBlack: "#585b70", brightRed: "#f37799", brightGreen: "#89d88b", brightYellow: "#ebd391",
      brightBlue: "#74a8fc", brightMagenta: "#f2aede", brightCyan: "#6bd7ca", brightWhite: "#bac2de",
    },
  },
  {
    // github.com/catppuccin/alacritty catppuccin-macchiato.toml (same departures as Mocha).
    id: "catppuccin-macchiato",
    name: "Catppuccin Macchiato",
    dark: true,
    theme: {
      background: "#24273a", foreground: "#cad3f5", cursor: "#c6a0f6", cursorAccent: "#24273a",
      selectionBackground: "#5b6078",
      black: "#494d64", red: "#ed8796", green: "#a6da95", yellow: "#eed49f",
      blue: "#8aadf4", magenta: "#f5bde6", cyan: "#8bd5ca", white: "#a5adcb",
      brightBlack: "#5b6078", brightRed: "#ec7486", brightGreen: "#8ccf7f", brightYellow: "#e1c682",
      brightBlue: "#78a1f6", brightMagenta: "#f2a9dd", brightCyan: "#63cbc0", brightWhite: "#b8c0e0",
    },
  },
  {
    // github.com/projekt0n/github-theme-contrib themes/kitty/github_dark_default.conf.
    // Selection is Primer's selection blue; accent is upstream's active border blue.
    id: "github-dark",
    name: "GitHub Dark",
    dark: true,
    theme: {
      background: "#0d1117", foreground: "#e6edf3", cursor: "#e6edf3", cursorAccent: "#0d1117",
      selectionBackground: "#264f78",
      black: "#484f58", red: "#ff7b72", green: "#3fb950", yellow: "#d29922",
      blue: "#58a6ff", magenta: "#bc8cff", cyan: "#39c5cf", white: "#b1bac4",
      brightBlack: "#6e7681", brightRed: "#ffa198", brightGreen: "#56d364", brightYellow: "#e3b341",
      brightBlue: "#79c0ff", brightMagenta: "#d2a8ff", brightCyan: "#56d4dd", brightWhite: "#ffffff",
    },
    overrides: { accent: "#2f81f7" },
  },
  {
    // github.com/rose-pine/alacritty dist/rose-pine.toml. Upstream repeats the normal
    // colors as brights. Accent is iris; the upstream cursor is a grey highlight.
    id: "rose-pine",
    name: "Rosé Pine",
    dark: true,
    theme: {
      background: "#191724", foreground: "#e0def4", cursor: "#524f67", cursorAccent: "#e0def4",
      selectionBackground: "#403d52",
      black: "#26233a", red: "#eb6f92", green: "#31748f", yellow: "#f6c177",
      blue: "#9ccfd8", magenta: "#c4a7e7", cyan: "#ebbcba", white: "#e0def4",
      brightBlack: "#6e6a86", brightRed: "#eb6f92", brightGreen: "#31748f", brightYellow: "#f6c177",
      brightBlue: "#9ccfd8", brightMagenta: "#c4a7e7", brightCyan: "#ebbcba", brightWhite: "#e0def4",
    },
    overrides: { accent: "#c4a7e7" },
  },
  {
    // github.com/rebelot/kanagawa.nvim extras/alacritty/kanagawa_wave.toml. Cursor is
    // upstream's oldWhite; accent is crystalBlue (else it derives to samuraiRed).
    id: "kanagawa-wave",
    name: "Kanagawa Wave",
    dark: true,
    theme: {
      background: "#1f1f28", foreground: "#dcd7ba", cursor: "#c8c093", cursorAccent: "#1f1f28",
      selectionBackground: "#2d4f67",
      black: "#090618", red: "#c34043", green: "#76946a", yellow: "#c0a36e",
      blue: "#7e9cd8", magenta: "#957fb8", cyan: "#6a9589", white: "#c8c093",
      brightBlack: "#727169", brightRed: "#e82424", brightGreen: "#98bb6c", brightYellow: "#e6c384",
      brightBlue: "#7fb4ca", brightMagenta: "#938aa9", brightCyan: "#7aa89f", brightWhite: "#dcd7ba",
    },
    overrides: { accent: "#7e9cd8" },
  },
  {
    // github.com/sainnhe/everforest autoload/everforest.vim (dark, medium). Upstream
    // maps brights to normals and brightBlack to bg3 (~1.4:1, so dim text vanishes);
    // brightBlack here is Everforest's own grey1, the color it uses for comments.
    id: "everforest-dark",
    name: "Everforest Dark",
    dark: true,
    theme: {
      background: "#2d353b", foreground: "#d3c6aa", cursor: "#a7c080", cursorAccent: "#2d353b",
      selectionBackground: "#543a48",
      black: "#475258", red: "#e67e80", green: "#a7c080", yellow: "#dbbc7f",
      blue: "#7fbbb3", magenta: "#d699b6", cyan: "#83c092", white: "#d3c6aa",
      brightBlack: "#859289", brightRed: "#e67e80", brightGreen: "#a7c080", brightYellow: "#dbbc7f",
      brightBlue: "#7fbbb3", brightMagenta: "#d699b6", brightCyan: "#83c092", brightWhite: "#d3c6aa",
    },
  },
  {
    // github.com/raunofreiberg/vesper (via alacritty-theme vesper.toml). Accent is
    // Vesper's signature orange; cursor is upstream's grey. Selection is a neutral
    // grey: the iTerm2 port's ochre selection relies on a selection foreground.
    id: "vesper",
    name: "Vesper",
    dark: true,
    theme: {
      background: "#101010", foreground: "#ffffff", cursor: "#acb1ab", cursorAccent: "#101010",
      selectionBackground: "#343434",
      black: "#101010", red: "#f5a191", green: "#90b99f", yellow: "#e6b99d",
      blue: "#aca1cf", magenta: "#e29eca", cyan: "#ea83a5", white: "#a0a0a0",
      brightBlack: "#7e7e7e", brightRed: "#ff8080", brightGreen: "#99ffe4", brightYellow: "#ffc799",
      brightBlue: "#b9aeda", brightMagenta: "#ecaad6", brightCyan: "#f591b2", brightWhite: "#ffffff",
    },
    overrides: { accent: "#ffc799" },
  },
  {
    // github.com/kepano/flexoki kitty/flexoki_dark.conf. Accent is Flexoki orange-400,
    // which has no ANSI slot; the upstream cursor is the foreground grey.
    id: "flexoki-dark",
    name: "Flexoki Dark",
    dark: true,
    theme: {
      background: "#100f0f", foreground: "#cecdc3", cursor: "#cecdc3", cursorAccent: "#100f0f",
      selectionBackground: "#403e3c",
      black: "#100f0f", red: "#af3029", green: "#66800b", yellow: "#ad8301",
      blue: "#205ea6", magenta: "#a02f6f", cyan: "#24837b", white: "#878580",
      brightBlack: "#6f6e69", brightRed: "#d14d41", brightGreen: "#879a39", brightYellow: "#d0a215",
      brightBlue: "#4385be", brightMagenta: "#ce5d97", brightCyan: "#3aa99f", brightWhite: "#cecdc3",
    },
    overrides: { accent: "#da702c" },
  },
];

export const DEFAULT_SCHEME_ID = "warm-ink-dark";

export function schemeById(id: string): Scheme | undefined {
  return SCHEMES.find((s) => s.id === id);
}
