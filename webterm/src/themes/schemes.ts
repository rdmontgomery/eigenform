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
