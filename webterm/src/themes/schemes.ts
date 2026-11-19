// Curated terminal color schemes. Each is a full xterm ITheme (16 ANSI +
// fg/bg/cursor/selection); deriveChrome() turns it into the whole app's chrome.
// Canonical palettes from the iTerm2-Color-Schemes lineage, plus Warm Ink (this
// app's own identity) as the default. To grow the catalog, see
// scripts/gen-schemes.mjs.
import type { TermTheme } from "./derive.ts";

export interface Scheme {
  /** Stable id, persisted in localStorage. */
  id: string;
  /** Human label shown in the picker. */
  name: string;
  /** Whether the background is dark (drives color-scheme + ordering). */
  dark: boolean;
  theme: TermTheme;
}

export const SCHEMES: Scheme[] = [
  {
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
    id: "warm-ink-light",
    name: "Warm Ink Light",
    dark: false,
    theme: {
      background: "#f3ecdf", foreground: "#3c332a", cursor: "#c0612f", cursorAccent: "#f3ecdf",
      selectionBackground: "#e2d6c0",
      black: "#2a201a", red: "#b5402f", green: "#6f7a2e", yellow: "#a9762a",
      blue: "#3f6f87", magenta: "#9a5577", cyan: "#4d8a78", white: "#d8cdb8",
      brightBlack: "#6a5d50", brightRed: "#c4513c", brightGreen: "#7f8a39", brightYellow: "#b9863a",
      brightBlue: "#4f7f97", brightMagenta: "#aa6587", brightCyan: "#5d9a88", brightWhite: "#efe6d6",
    },
  },
  {
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
    id: "nord",
    name: "Nord",
    dark: true,
    theme: {
      background: "#2e3440", foreground: "#d8dee9", cursor: "#88c0d0", cursorAccent: "#2e3440",
      selectionBackground: "#434c5e",
      black: "#3b4252", red: "#bf616a", green: "#a3be8c", yellow: "#ebcb8b",
      blue: "#81a1c1", magenta: "#b48ead", cyan: "#88c0d0", white: "#e5e9f0",
      brightBlack: "#4c566a", brightRed: "#bf616a", brightGreen: "#a3be8c", brightYellow: "#ebcb8b",
      brightBlue: "#81a1c1", brightMagenta: "#b48ead", brightCyan: "#8fbcbb", brightWhite: "#eceff4",
    },
  },
  {
    id: "tokyo-night",
    name: "Tokyo Night",
    dark: true,
    theme: {
      background: "#1a1b26", foreground: "#c0caf5", cursor: "#7aa2f7", cursorAccent: "#1a1b26",
      selectionBackground: "#283457",
      black: "#15161e", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#f7768e", brightGreen: "#9ece6a", brightYellow: "#e0af68",
      brightBlue: "#7aa2f7", brightMagenta: "#bb9af7", brightCyan: "#7dcfff", brightWhite: "#c0caf5",
    },
  },
  {
    id: "tokyo-night-storm",
    name: "Tokyo Night Storm",
    dark: true,
    theme: {
      background: "#24283b", foreground: "#c0caf5", cursor: "#7aa2f7", cursorAccent: "#24283b",
      selectionBackground: "#2e3c64",
      black: "#1d202f", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#f7768e", brightGreen: "#9ece6a", brightYellow: "#e0af68",
      brightBlue: "#7aa2f7", brightMagenta: "#bb9af7", brightCyan: "#7dcfff", brightWhite: "#c0caf5",
    },
  },
  {
    id: "tokyo-night-day",
    name: "Tokyo Night Day",
    dark: false,
    theme: {
      background: "#e1e2e7", foreground: "#3760bf", cursor: "#2e7de9", cursorAccent: "#e1e2e7",
      selectionBackground: "#b7c1e3",
      black: "#e9e9ed", red: "#f52a65", green: "#587539", yellow: "#8c6c3e",
      blue: "#2e7de9", magenta: "#9854f1", cyan: "#007197", white: "#6172b0",
      brightBlack: "#a1a6c5", brightRed: "#f52a65", brightGreen: "#587539", brightYellow: "#8c6c3e",
      brightBlue: "#2e7de9", brightMagenta: "#9854f1", brightCyan: "#007197", brightWhite: "#3760bf",
    },
  },
  {
    id: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    dark: true,
    theme: {
      background: "#1e1e2e", foreground: "#cdd6f4", cursor: "#cba6f7", cursorAccent: "#1e1e2e",
      selectionBackground: "#585b70",
      black: "#45475a", red: "#f38ba8", green: "#a6e3a1", yellow: "#f9e2af",
      blue: "#89b4fa", magenta: "#f5c2e7", cyan: "#94e2d5", white: "#bac2de",
      brightBlack: "#585b70", brightRed: "#f38ba8", brightGreen: "#a6e3a1", brightYellow: "#f9e2af",
      brightBlue: "#89b4fa", brightMagenta: "#f5c2e7", brightCyan: "#94e2d5", brightWhite: "#a6adc8",
    },
  },
  {
    id: "catppuccin-macchiato",
    name: "Catppuccin Macchiato",
    dark: true,
    theme: {
      background: "#24273a", foreground: "#cad3f5", cursor: "#c6a0f6", cursorAccent: "#24273a",
      selectionBackground: "#5b6078",
      black: "#494d64", red: "#ed8796", green: "#a6da95", yellow: "#eed49f",
      blue: "#8aadf4", magenta: "#f5bde6", cyan: "#8bd5ca", white: "#b8c0e0",
      brightBlack: "#5b6078", brightRed: "#ed8796", brightGreen: "#a6da95", brightYellow: "#eed49f",
      brightBlue: "#8aadf4", brightMagenta: "#f5bde6", brightCyan: "#8bd5ca", brightWhite: "#a5adcb",
    },
  },
  {
    id: "catppuccin-frappe",
    name: "Catppuccin Frappé",
    dark: true,
    theme: {
      background: "#303446", foreground: "#c6d0f5", cursor: "#ca9ee6", cursorAccent: "#303446",
      selectionBackground: "#626880",
      black: "#51576d", red: "#e78284", green: "#a6d189", yellow: "#e5c890",
      blue: "#8caaee", magenta: "#f4b8e4", cyan: "#81c8be", white: "#b5bfe2",
      brightBlack: "#626880", brightRed: "#e78284", brightGreen: "#a6d189", brightYellow: "#e5c890",
      brightBlue: "#8caaee", brightMagenta: "#f4b8e4", brightCyan: "#81c8be", brightWhite: "#a5adce",
    },
  },
  {
    id: "catppuccin-latte",
    name: "Catppuccin Latte",
    dark: false,
    theme: {
      background: "#eff1f5", foreground: "#4c4f69", cursor: "#8839ef", cursorAccent: "#eff1f5",
      selectionBackground: "#ccced7",
      black: "#5c5f77", red: "#d20f39", green: "#40a02b", yellow: "#df8e1d",
      blue: "#1e66f5", magenta: "#ea76cb", cyan: "#179299", white: "#acb0be",
      brightBlack: "#6c6f85", brightRed: "#d20f39", brightGreen: "#40a02b", brightYellow: "#df8e1d",
      brightBlue: "#1e66f5", brightMagenta: "#ea76cb", brightCyan: "#179299", brightWhite: "#bcc0cc",
    },
  },
  {
    id: "one-dark",
    name: "One Dark",
    dark: true,
    theme: {
      background: "#282c34", foreground: "#abb2bf", cursor: "#528bff", cursorAccent: "#282c34",
      selectionBackground: "#3e4451",
      black: "#282c34", red: "#e06c75", green: "#98c379", yellow: "#e5c07b",
      blue: "#61afef", magenta: "#c678dd", cyan: "#56b6c2", white: "#abb2bf",
      brightBlack: "#5c6370", brightRed: "#e06c75", brightGreen: "#98c379", brightYellow: "#e5c07b",
      brightBlue: "#61afef", brightMagenta: "#c678dd", brightCyan: "#56b6c2", brightWhite: "#ffffff",
    },
  },
  {
    id: "monokai",
    name: "Monokai",
    dark: true,
    theme: {
      background: "#272822", foreground: "#f8f8f2", cursor: "#f92672", cursorAccent: "#272822",
      selectionBackground: "#49483e",
      black: "#272822", red: "#f92672", green: "#a6e22e", yellow: "#f4bf75",
      blue: "#66d9ef", magenta: "#ae81ff", cyan: "#a1efe4", white: "#f8f8f2",
      brightBlack: "#75715e", brightRed: "#f92672", brightGreen: "#a6e22e", brightYellow: "#f4bf75",
      brightBlue: "#66d9ef", brightMagenta: "#ae81ff", brightCyan: "#a1efe4", brightWhite: "#f9f8f5",
    },
  },
  {
    id: "night-owl",
    name: "Night Owl",
    dark: true,
    theme: {
      background: "#011627", foreground: "#d6deeb", cursor: "#80a4c2", cursorAccent: "#011627",
      selectionBackground: "#1d3b53",
      black: "#011627", red: "#ef5350", green: "#22da6e", yellow: "#c5e478",
      blue: "#82aaff", magenta: "#c792ea", cyan: "#21c7a8", white: "#ffffff",
      brightBlack: "#575656", brightRed: "#ef5350", brightGreen: "#22da6e", brightYellow: "#ffeb95",
      brightBlue: "#82aaff", brightMagenta: "#c792ea", brightCyan: "#7fdbca", brightWhite: "#ffffff",
    },
  },
  // Rosé Pine: palette from @rose-pine/palette 4.0.1, ANSI mapping per its terminal ports.
  {
    id: "rose-pine",
    name: "Rosé Pine",
    dark: true,
    theme: {
      background: "#191724", foreground: "#e0def4", cursor: "#ebbcba", cursorAccent: "#191724",
      selectionBackground: "#403d52",
      black: "#26233a", red: "#eb6f92", green: "#31748f", yellow: "#f6c177",
      blue: "#9ccfd8", magenta: "#c4a7e7", cyan: "#ebbcba", white: "#e0def4",
      brightBlack: "#6e6a86", brightRed: "#eb6f92", brightGreen: "#31748f", brightYellow: "#f6c177",
      brightBlue: "#9ccfd8", brightMagenta: "#c4a7e7", brightCyan: "#ebbcba", brightWhite: "#e0def4",
    },
  },
  {
    id: "rose-pine-moon",
    name: "Rosé Pine Moon",
    dark: true,
    theme: {
      background: "#232136", foreground: "#e0def4", cursor: "#ea9a97", cursorAccent: "#232136",
      selectionBackground: "#44415a",
      black: "#393552", red: "#eb6f92", green: "#3e8fb0", yellow: "#f6c177",
      blue: "#9ccfd8", magenta: "#c4a7e7", cyan: "#ea9a97", white: "#e0def4",
      brightBlack: "#6e6a86", brightRed: "#eb6f92", brightGreen: "#3e8fb0", brightYellow: "#f6c177",
      brightBlue: "#9ccfd8", brightMagenta: "#c4a7e7", brightCyan: "#ea9a97", brightWhite: "#e0def4",
    },
  },
  {
    id: "rose-pine-dawn",
    name: "Rosé Pine Dawn",
    dark: false,
    theme: {
      background: "#faf4ed", foreground: "#575279", cursor: "#d7827e", cursorAccent: "#faf4ed",
      selectionBackground: "#dfdad9",
      black: "#f2e9e1", red: "#b4637a", green: "#286983", yellow: "#ea9d34",
      blue: "#56949f", magenta: "#907aa9", cyan: "#d7827e", white: "#575279",
      brightBlack: "#9893a5", brightRed: "#b4637a", brightGreen: "#286983", brightYellow: "#ea9d34",
      brightBlue: "#56949f", brightMagenta: "#907aa9", brightCyan: "#d7827e", brightWhite: "#575279",
    },
  },
  // Kanagawa (kanagawa.nvim): Wave, Dragon, Lotus.
  {
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
  },
  {
    id: "kanagawa-dragon",
    name: "Kanagawa Dragon",
    dark: true,
    theme: {
      background: "#181616", foreground: "#c5c9c5", cursor: "#c8c093", cursorAccent: "#181616",
      selectionBackground: "#2d4f67",
      black: "#0d0c0c", red: "#c4746e", green: "#8a9a7b", yellow: "#c4b28a",
      blue: "#8ba4b0", magenta: "#a292a3", cyan: "#8ea4a2", white: "#c8c093",
      brightBlack: "#a6a69c", brightRed: "#e46876", brightGreen: "#87a987", brightYellow: "#e6c384",
      brightBlue: "#7fb4ca", brightMagenta: "#938aa9", brightCyan: "#7aa89f", brightWhite: "#c5c9c5",
    },
  },
  {
    id: "kanagawa-lotus",
    name: "Kanagawa Lotus",
    dark: false,
    theme: {
      background: "#f2ecbc", foreground: "#545464", cursor: "#43436c", cursorAccent: "#f2ecbc",
      selectionBackground: "#c9cbd1",
      black: "#1f1f28", red: "#c84053", green: "#6f894e", yellow: "#77713f",
      blue: "#4d699b", magenta: "#b35b79", cyan: "#597b75", white: "#545464",
      brightBlack: "#8a8980", brightRed: "#d7474b", brightGreen: "#6e915f", brightYellow: "#836f4a",
      brightBlue: "#6693bf", brightMagenta: "#624c83", brightCyan: "#5e857a", brightWhite: "#43436c",
    },
  },
  // Everforest (medium contrast).
  {
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
    id: "everforest-light",
    name: "Everforest Light",
    dark: false,
    theme: {
      background: "#fdf6e3", foreground: "#5c6a72", cursor: "#8da101", cursorAccent: "#fdf6e3",
      selectionBackground: "#eaedc8",
      black: "#5c6a72", red: "#f85552", green: "#8da101", yellow: "#dfa000",
      blue: "#3a94c5", magenta: "#df69ba", cyan: "#35a77c", white: "#939f91",
      brightBlack: "#829181", brightRed: "#f85552", brightGreen: "#8da101", brightYellow: "#dfa000",
      brightBlue: "#3a94c5", brightMagenta: "#df69ba", brightCyan: "#35a77c", brightWhite: "#a6b0a0",
    },
  },
  // Flexoki (kepano): 600-weight accents as normal, 400-weight as bright.
  {
    id: "flexoki-dark",
    name: "Flexoki Dark",
    dark: true,
    theme: {
      background: "#100f0f", foreground: "#cecdc3", cursor: "#da702c", cursorAccent: "#100f0f",
      selectionBackground: "#403e3c",
      black: "#100f0f", red: "#af3029", green: "#66800b", yellow: "#ad8301",
      blue: "#205ea6", magenta: "#a02f6f", cyan: "#24837b", white: "#878580",
      brightBlack: "#575653", brightRed: "#d14d41", brightGreen: "#879a39", brightYellow: "#d0a215",
      brightBlue: "#4385be", brightMagenta: "#ce5d97", brightCyan: "#3aa99f", brightWhite: "#cecdc3",
    },
  },
  {
    id: "flexoki-light",
    name: "Flexoki Light",
    dark: false,
    theme: {
      background: "#fffcf0", foreground: "#100f0f", cursor: "#bc5215", cursorAccent: "#fffcf0",
      selectionBackground: "#cecdc3",
      black: "#100f0f", red: "#af3029", green: "#66800b", yellow: "#ad8301",
      blue: "#205ea6", magenta: "#a02f6f", cyan: "#24837b", white: "#6f6e69",
      brightBlack: "#878580", brightRed: "#d14d41", brightGreen: "#879a39", brightYellow: "#d0a215",
      brightBlue: "#4385be", brightMagenta: "#ce5d97", brightCyan: "#3aa99f", brightWhite: "#b7b5ac",
    },
  },
  // Ayu: terminal blocks from the `ayu` npm palette 9.1.0. Its bright black equals
  // black (invisible on the background), so bright black is the UI's muted fg.
  {
    id: "ayu-dark",
    name: "Ayu Dark",
    dark: true,
    theme: {
      background: "#0d1017", foreground: "#bfbdb6", cursor: "#e6b450", cursorAccent: "#0d1017",
      selectionBackground: "#172e51",
      black: "#0a0000", red: "#e6495a", green: "#97c142", yellow: "#e89d37",
      blue: "#17acf2", magenta: "#c385fe", cyan: "#84ceb5", white: "#ffffff",
      brightBlack: "#5a6378", brightRed: "#f07178", brightGreen: "#aad94c", brightYellow: "#ffb454",
      brightBlue: "#59c2ff", brightMagenta: "#d2a6ff", brightCyan: "#95e6cb", brightWhite: "#ffffff",
    },
  },
  {
    id: "ayu-mirage",
    name: "Ayu Mirage",
    dark: true,
    theme: {
      background: "#1f2430", foreground: "#cccac2", cursor: "#ffcc66", cursorAccent: "#1f2430",
      selectionBackground: "#274364",
      black: "#0a0000", red: "#f06b5c", green: "#bfe76d", yellow: "#e6b752",
      blue: "#3bbbf4", magenta: "#d09ffd", cyan: "#84ceb5", white: "#d2d6dc",
      brightBlack: "#707a8c", brightRed: "#f39184", brightGreen: "#d5ff80", brightYellow: "#ffcd66",
      brightBlue: "#73d0ff", brightMagenta: "#dfbfff", brightCyan: "#95e6cb", brightWhite: "#e3e6ea",
    },
  },
  {
    id: "ayu-light",
    name: "Ayu Light",
    dark: false,
    theme: {
      background: "#f8f9fa", foreground: "#5c6166", cursor: "#f29718", cursorAccent: "#f8f9fa",
      selectionBackground: "#d3e1f5",
      black: "#86878c", red: "#f07171", green: "#86b300", yellow: "#eba400",
      blue: "#22a4e6", magenta: "#a37acc", cyan: "#4cbf99", white: "#adaeb1",
      brightBlack: "#939498", brightRed: "#f07171", brightGreen: "#86b300", brightYellow: "#eba400",
      brightBlue: "#22a4e6", brightMagenta: "#a37acc", brightCyan: "#4cbf99", brightWhite: "#c5c5c8",
    },
  },
  // Nightfox family (nightfox.nvim).
  {
    id: "nightfox",
    name: "Nightfox",
    dark: true,
    theme: {
      background: "#192330", foreground: "#cdcecf", cursor: "#719cd6", cursorAccent: "#192330",
      selectionBackground: "#2b3b51",
      black: "#393b44", red: "#c94f6d", green: "#81b29a", yellow: "#dbc074",
      blue: "#719cd6", magenta: "#9d79d6", cyan: "#63cdcf", white: "#dfdfe0",
      brightBlack: "#575860", brightRed: "#d16983", brightGreen: "#8ebaa4", brightYellow: "#e0c989",
      brightBlue: "#86abdc", brightMagenta: "#baa1e2", brightCyan: "#7ad5d6", brightWhite: "#e4e4e5",
    },
  },
  {
    id: "carbonfox",
    name: "Carbonfox",
    dark: true,
    theme: {
      background: "#161616", foreground: "#f2f4f8", cursor: "#78a9ff", cursorAccent: "#161616",
      selectionBackground: "#2a2a2a",
      black: "#282828", red: "#ee5396", green: "#25be6a", yellow: "#08bdba",
      blue: "#78a9ff", magenta: "#be95ff", cyan: "#33b1ff", white: "#dfdfe0",
      brightBlack: "#484848", brightRed: "#f16da6", brightGreen: "#46c880", brightYellow: "#2dc7c4",
      brightBlue: "#8cb6ff", brightMagenta: "#c8a5ff", brightCyan: "#52bdff", brightWhite: "#e4e4e5",
    },
  },
  {
    id: "dawnfox",
    name: "Dawnfox",
    dark: false,
    theme: {
      background: "#faf4ed", foreground: "#575279", cursor: "#286983", cursorAccent: "#faf4ed",
      selectionBackground: "#d0d8d8",
      black: "#575279", red: "#b4637a", green: "#618774", yellow: "#ea9d34",
      blue: "#286983", magenta: "#907aa9", cyan: "#56949f", white: "#e5e9f0",
      brightBlack: "#5f5695", brightRed: "#c26d85", brightGreen: "#629f81", brightYellow: "#eea846",
      brightBlue: "#2d81a3", brightMagenta: "#9a80b9", brightCyan: "#5ca7b4", brightWhite: "#e6ebf3",
    },
  },
  // Poimandres: its bright black equals the foreground, so bright black is the
  // theme's darker grey to keep dim text distinguishable.
  {
    id: "poimandres",
    name: "Poimandres",
    dark: true,
    theme: {
      background: "#1b1e28", foreground: "#a6accd", cursor: "#5de4c7", cursorAccent: "#1b1e28",
      selectionBackground: "#303340",
      black: "#1b1e28", red: "#d0679d", green: "#5de4c7", yellow: "#fffac2",
      blue: "#89ddff", magenta: "#fcc5e9", cyan: "#add7ff", white: "#ffffff",
      brightBlack: "#767c9d", brightRed: "#d0679d", brightGreen: "#5de4c7", brightYellow: "#fffac2",
      brightBlue: "#add7ff", brightMagenta: "#f087bd", brightCyan: "#89ddff", brightWhite: "#ffffff",
    },
  },
  // GitHub Dark (dark default): ANSI from @primer/primitives 7.10.0 with
  // github-vscode-theme's fg/accent overrides.
  {
    id: "github-dark",
    name: "GitHub Dark",
    dark: true,
    theme: {
      background: "#0d1117", foreground: "#e6edf3", cursor: "#2f81f7", cursorAccent: "#0d1117",
      selectionBackground: "#17335a",
      black: "#484f58", red: "#ff7b72", green: "#3fb950", yellow: "#d29922",
      blue: "#58a6ff", magenta: "#bc8cff", cyan: "#39c5cf", white: "#b1bac4",
      brightBlack: "#6e7681", brightRed: "#ffa198", brightGreen: "#56d364", brightYellow: "#e3b341",
      brightBlue: "#79c0ff", brightMagenta: "#d2a8ff", brightCyan: "#56d4dd", brightWhite: "#ffffff",
    },
  },
  {
    id: "github-light",
    name: "GitHub Light",
    dark: false,
    theme: {
      background: "#ffffff", foreground: "#24292e", cursor: "#044289", cursorAccent: "#ffffff",
      selectionBackground: "#c8e1ff",
      black: "#24292e", red: "#d73a49", green: "#28a745", yellow: "#dbab09",
      blue: "#0366d6", magenta: "#5a32a3", cyan: "#1b7c83", white: "#6a737d",
      brightBlack: "#959da5", brightRed: "#cb2431", brightGreen: "#22863a", brightYellow: "#b08800",
      brightBlue: "#005cc5", brightMagenta: "#5a32a3", brightCyan: "#3192aa", brightWhite: "#d1d5da",
    },
  },
];

export const DEFAULT_SCHEME_ID = "warm-ink-dark";

export function schemeById(id: string): Scheme | undefined {
  return SCHEMES.find((s) => s.id === id);
}
