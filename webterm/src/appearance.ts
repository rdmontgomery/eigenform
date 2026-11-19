// Appearance: the color scheme and terminal typography, both persisted, plus the
// two top-bar popovers that edit them and the ⌘/Ctrl +/−/0 zoom keys.
//
// The shell owns the terminals, so it supplies the side effects: repaint every
// terminal on a scheme change, re-apply + refit on a font change, and re-render
// the top bar whenever a popover opens/closes or the scheme name changes.

import { DEFAULT_FONT } from "./pty.ts";
import type { FontSettings } from "./pty.ts";
import { SCHEMES, DEFAULT_SCHEME_ID, schemeById } from "./themes/schemes.ts";
import type { Scheme } from "./themes/schemes.ts";
import { deriveChrome, hexToOklch } from "./themes/derive.ts";
import type { TermTheme } from "./themes/derive.ts";
import { completeTheme, normalizeHex, parseTheme, pushRecent, slugFor } from "./themes/parse.ts";
import type { ThemeKey } from "./themes/parse.ts";
import { mountShimmer, SHIMMER_LEVELS } from "./themes/shimmer.ts";
import type { ShimmerLevel } from "./themes/shimmer.ts";
import { eventStream } from "./event-stream.ts";
import { el } from "./dom.ts";
import { icon } from "./icons.ts";

const LS_THEME = "eigenform:term:theme:v1"; // legacy light/dark — migrated to v2
const LS_SCHEME = "eigenform:term:theme:v2"; // scheme id
const LS_FONT = "eigenform:term:font:v1";
const LS_RECENT = "eigenform:term:theme:recent:v1"; // scheme ids, most recent first
const LS_CUSTOM = "eigenform:term:themes:cache:v1"; // last /api/themes, for first paint
const LS_SHIMMER = "eigenform:term:shimmer:v1";

function readLs<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}
function writeLs(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage off: session-only */ }
}

/** A custom theme as `/api/themes` stores it: only background + foreground required. */
export interface StoredTheme {
  id: string;
  name: string;
  theme: Partial<TermTheme>;
  updatedAt?: number;
}

/** Complete a stored theme into a scheme, filling gaps from Warm Ink of the same
 *  lightness. */
export function customToScheme(c: StoredTheme): Scheme {
  const bg = c.theme.background ? normalizeHex(c.theme.background) : null;
  const darkBg = !bg || hexToOklch(bg).L < 0.5;
  const base = schemeById(darkBg ? "warm-ink-dark" : "warm-ink-light")!.theme;
  const theme = completeTheme(c.theme, base);
  return { id: c.id, name: c.name, dark: hexToOklch(theme.background).L < 0.5, theme, custom: true };
}

function isStored(v: unknown): v is StoredTheme {
  const t = v as StoredTheme;
  return !!t && typeof t.id === "string" && typeof t.name === "string" && !!t.theme && typeof t.theme === "object";
}

/** Resolve the active scheme: stored v2 id → migrated v1 light/dark → default. */
function loadSchemeId(resolve: (id: string) => Scheme | undefined): string {
  const v2 = localStorage.getItem(LS_SCHEME);
  if (v2 && resolve(v2)) return v2;
  const v1 = localStorage.getItem(LS_THEME);
  if (v1 === "light") return "warm-ink-light";
  if (v1 === "dark") return "warm-ink-dark";
  return DEFAULT_SCHEME_ID;
}

/** Terminal typefaces offered in the font popover. macOS-first: "System Mono"
 *  resolves to SF Mono / Menlo with no webfont round-trip. */
const TERM_FACES: { label: string; stack: string }[] = [
  { label: "Plex Mono", stack: DEFAULT_FONT.family },
  {
    label: "System Mono",
    stack: 'ui-monospace, "SF Mono", Menlo, "Cascadia Mono", Consolas, monospace',
  },
];

/** Bounds for the font controls (and ⌘ +/− zoom). */
const FONT_BOUNDS = {
  size: { min: 9, max: 24, step: 0.5 },
  lineHeight: { min: 1.0, max: 2.0, step: 0.05 },
  letterSpacing: { min: -2, max: 4, step: 0.5 },
};

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Validate one persisted numeric field, falling back to a default. */
function numField(v: unknown, fallback: number, lo: number, hi: number): number {
  return typeof v === "number" && Number.isFinite(v) ? clamp(v, lo, hi) : fallback;
}

function loadFont(): FontSettings {
  try {
    const raw = JSON.parse(localStorage.getItem(LS_FONT) ?? "{}") as Partial<FontSettings>;
    return {
      family: typeof raw.family === "string" && raw.family ? raw.family : DEFAULT_FONT.family,
      size: numField(raw.size, DEFAULT_FONT.size, FONT_BOUNDS.size.min, FONT_BOUNDS.size.max),
      lineHeight: numField(raw.lineHeight, DEFAULT_FONT.lineHeight, FONT_BOUNDS.lineHeight.min, FONT_BOUNDS.lineHeight.max),
      letterSpacing: numField(raw.letterSpacing, DEFAULT_FONT.letterSpacing, FONT_BOUNDS.letterSpacing.min, FONT_BOUNDS.letterSpacing.max),
    };
  } catch {
    return { ...DEFAULT_FONT };
  }
}

function applyChrome(s: Scheme) {
  const root = document.documentElement;
  for (const [k, v] of Object.entries(deriveChrome(s.theme))) {
    root.style.setProperty(k, v);
  }
  root.style.colorScheme = s.dark ? "dark" : "light";
}

interface Popover {
  isOpen(): boolean;
  toggle(anchor: HTMLElement): void;
  /** Rebuild the body from current state (no-op while closed). */
  rerender(): void;
}

/**
 * A top-bar popover anchored under its button: closes on Escape or on a pointer
 * down outside both itself and its button (capture phase, so a click on a
 * tab/terminal closes it before that target acts).
 */
function createPopover(
  className: string,
  buttonSelector: string,
  render: (pop: HTMLElement) => void,
  onOpenChange: () => void,
  onClose?: () => void,
): Popover {
  let pop: HTMLElement | null = null;

  const onOutside = (e: PointerEvent) => {
    const t = e.target as HTMLElement;
    if (pop && !pop.contains(t) && !t.closest(buttonSelector)) close();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") { e.preventDefault(); close(); }
  };

  function open(anchor: HTMLElement) {
    pop = el("div", className);
    document.body.append(pop);
    const r = anchor.getBoundingClientRect();
    pop.style.top = `${Math.round(r.bottom + 6)}px`;
    pop.style.right = `${Math.round(window.innerWidth - r.right)}px`;
    rerender();
    onOpenChange();
    document.addEventListener("pointerdown", onOutside, true);
    document.addEventListener("keydown", onKey, true);
  }

  function close() {
    pop?.remove();
    pop = null;
    onClose?.();
    document.removeEventListener("pointerdown", onOutside, true);
    document.removeEventListener("keydown", onKey, true);
    onOpenChange();
  }

  function rerender() {
    if (!pop) return;
    pop.innerHTML = "";
    render(pop);
  }

  return {
    isOpen: () => pop !== null,
    toggle: (anchor) => (pop ? close() : open(anchor)),
    rerender,
  };
}


// The 6 ANSI hues shown in a swatch strip — a quick read of a scheme's palette.
const SWATCH_KEYS = ["red", "yellow", "green", "cyan", "blue", "magenta"] as const;

/** Recently used schemes shown above the catalog (when there are at least 2). */
const RECENT_SHOWN = 4;

/** Editor layout: the base colors, then the ANSI table as normal | bright pairs. */
const BASE_FIELDS: [ThemeKey, string][] = [
  ["background", "Background"], ["foreground", "Foreground"], ["cursor", "Cursor"],
  ["cursorAccent", "Cursor text"], ["selectionBackground", "Selection"],
];
const ANSI_ROWS = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;
const brightOf = (n: string) => `bright${n[0]!.toUpperCase()}${n.slice(1)}` as ThemeKey;

/** The theme being edited in the popover. Changes preview live; Cancel (or closing
 *  the popover) puts `restore` back. */
interface Draft {
  /** The custom theme's id; null for a new one (the id is minted on save). */
  id: string | null;
  name: string;
  /** True once the user typed a name, so a paste's name doesn't overwrite it. */
  named: boolean;
  colors: TermTheme;
  paste: string;
  status: string;
  error: boolean;
  restore: Scheme;
}

export interface Appearance {
  scheme(): Scheme;
  font(): FontSettings;
  themePopover: Popover;
  fontPopover: Popover;
}

/**
 * Load the persisted scheme + font and paint the chrome immediately (call before
 * any layout so first paint is correct). Registers the global zoom keys.
 */
export function createAppearance(hooks: {
  /** Repaint every open terminal with the new scheme. */
  onScheme: (s: Scheme) => void;
  /** Re-apply the font to every open terminal (and refit). */
  onFont: (f: FontSettings) => void;
  /** Re-render the top bar (popover open state, theme title). */
  onChromeChange: () => void;
}): Appearance {
  // Custom themes: the cached list paints the first frame; /api/themes then refreshes.
  let customs: Scheme[] = readLs<unknown[]>(LS_CUSTOM, []).filter(isStored).map(customToScheme);
  const resolve = (id: string) => customs.find((s) => s.id === id) ?? schemeById(id);

  let scheme: Scheme = resolve(loadSchemeId(resolve)) ?? SCHEMES[0]!;
  let recent = readLs<unknown[]>(LS_RECENT, []).filter((x): x is string => typeof x === "string");
  let shimmerLevel: ShimmerLevel = SHIMMER_LEVELS.find((l) => l === readLs(LS_SHIMMER, "off")) ?? "off";
  const shimmer = mountShimmer();
  let draft: Draft | null = null;

  /** Paint chrome, terminals and shimmer from `s` (no persistence). */
  function paint(s: Scheme) {
    applyChrome(s);
    shimmer.set(shimmerLevel, s.theme, s.dark);
    hooks.onScheme(s);
  }
  applyChrome(scheme);
  shimmer.set(shimmerLevel, scheme.theme, scheme.dark);
  let font = loadFont();

  /** Paint the whole surface from a scheme: chrome tokens on :root + every
   *  open terminal's colors. Persisted so it survives reload. */
  function applyScheme(next: Scheme) {
    scheme = next;
    localStorage.setItem(LS_SCHEME, next.id);
    recent = pushRecent(recent, next.id);
    writeLs(LS_RECENT, recent);
    paint(next);
    hooks.onChromeChange();
    themePopover.rerender();
  }

  function setShimmer(level: ShimmerLevel) {
    shimmerLevel = level;
    writeLs(LS_SHIMMER, level);
    const s = draft ? draftScheme(draft) : scheme;
    shimmer.set(level, s.theme, s.dark);
    themePopover.rerender();
  }

  // ── Custom themes ───────────────────────────────────────────────────────
  function setCustoms(list: StoredTheme[]) {
    customs = list.filter(isStored).map(customToScheme);
    writeLs(LS_CUSTOM, list);
    // Someone (another window, a Claude session) changed the theme in use: repaint.
    const live = scheme.custom ? customs.find((s) => s.id === scheme.id) : undefined;
    if (live && !draft) {
      scheme = live;
      paint(live);
      hooks.onChromeChange();
    }
    if (!draft) themePopover.rerender();
  }

  async function refreshCustoms() {
    try {
      const res = await fetch("/api/themes");
      if (res.ok) setCustoms((await res.json()) as StoredTheme[]);
    } catch { /* daemon unreachable: keep the cache */ }
  }
  void refreshCustoms();
  eventStream.subscribe({
    onEvent: (ev) => { if (ev.kind === "themes-changed") void refreshCustoms(); },
    onReopen: () => void refreshCustoms(),
  });

  const draftScheme = (d: Draft): Scheme => ({
    id: d.id ?? "draft",
    name: d.name,
    dark: hexToOklch(d.colors.background).L < 0.5,
    theme: d.colors,
    custom: true,
  });

  /** Live preview of the draft, at most once a frame (color pickers fire fast). */
  let previewQueued = false;
  function previewDraft() {
    if (previewQueued) return;
    previewQueued = true;
    requestAnimationFrame(() => {
      previewQueued = false;
      if (draft) paint(draftScheme(draft));
    });
  }

  function startEdit(from: Scheme) {
    const own = from.custom === true;
    draft = {
      id: own ? from.id : null,
      name: own ? from.name : `${from.name} (mine)`,
      named: own,
      colors: { ...from.theme },
      paste: "",
      status: "",
      error: false,
      restore: scheme,
    };
    themePopover.rerender();
    previewDraft();
  }

  function endEdit(restore: boolean) {
    const d = draft;
    draft = null;
    if (d && restore) paint(d.restore);
  }

  async function saveDraft(d: Draft) {
    const taken = new Set([...SCHEMES, ...customs].map((s) => s.id));
    const id = d.id ?? slugFor(d.name, taken);
    try {
      const res = await fetch(`/api/themes/${encodeURIComponent(id)}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: d.name, theme: d.colors }),
      });
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      setCustoms((await res.json()) as StoredTheme[]);
      endEdit(false);
      applyScheme(resolve(id) ?? scheme);
    } catch (e) {
      d.status = `Couldn't save: ${(e as Error).message}`;
      d.error = true;
      themePopover.rerender();
    }
  }

  async function deleteDraft(d: Draft) {
    if (!d.id) return;
    try {
      const res = await fetch(`/api/themes/${encodeURIComponent(d.id)}`, { method: "DELETE" });
      if (!res.ok && res.status !== 404) throw new Error((await res.text()) || `HTTP ${res.status}`);
      if (res.ok) setCustoms((await res.json()) as StoredTheme[]);
      else setCustoms(customs.filter((s) => s.id !== d.id).map((s) => ({ id: s.id, name: s.name, theme: s.theme })));
      const back = d.restore.id === d.id ? resolve(DEFAULT_SCHEME_ID)! : d.restore;
      recent = recent.filter((r) => r !== d.id);
      endEdit(false);
      applyScheme(back);
    } catch (e) {
      d.status = `Couldn't delete: ${(e as Error).message}`;
      d.error = true;
      themePopover.rerender();
    }
  }

  /** Merge a paste into the draft: pasted keys win, the rest re-derive or stay. */
  function applyPaste(d: Draft, text: string) {
    d.paste = text;
    const parsed = parseTheme(text);
    const n = Object.keys(parsed.colors).length;
    d.error = false;
    if (!text.trim()) d.status = "";
    else if (!n) { d.status = "No colors found in that."; d.error = true; }
    else {
      d.colors = completeTheme(parsed.colors, d.colors);
      if (parsed.name && !d.named) d.name = parsed.name;
      d.status = `Read ${n} color${n === 1 ? "" : "s"}.`;
    }
    themePopover.rerender();
    previewDraft();
  }

  /** Merge a patch into the live font settings, persist, and re-lay every grid. */
  function setFont(patch: Partial<FontSettings>) {
    font = { ...font, ...patch };
    localStorage.setItem(LS_FONT, JSON.stringify(font));
    hooks.onFont(font);
  }

  function bumpFontSize(delta: number) {
    const b = FONT_BOUNDS.size;
    setFont({ size: clamp(Math.round((font.size + delta) * 4) / 4, b.min, b.max) });
    fontPopover.rerender();
  }

  // ⌘/Ctrl +/−/0 — terminal zoom (overrides browser page zoom, which is the
  // wrong granularity for a grid we own).
  window.addEventListener("keydown", (e) => {
    if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
    if (e.key === "=" || e.key === "+") { e.preventDefault(); bumpFontSize(0.5); }
    else if (e.key === "-" || e.key === "_") { e.preventDefault(); bumpFontSize(-0.5); }
    else if (e.key === "0") { e.preventDefault(); setFont({ size: DEFAULT_FONT.size }); fontPopover.rerender(); }
  });

  /** Font popover — typeface + size + line-height + letter-spacing. */
  const fontPopover = createPopover(
    "font-pop",
    ".font-btn",
    (pop) => {
      // Typeface — a row of pills; the active stack is highlighted.
      const faceRow = el("div", "font-faces");
      for (const f of TERM_FACES) {
        const pill = el("button", `font-face${font.family === f.stack ? " font-face--on" : ""}`);
        pill.textContent = f.label;
        pill.style.fontFamily = f.stack;
        pill.addEventListener("click", () => { setFont({ family: f.stack }); fontPopover.rerender(); });
        faceRow.append(pill);
      }
      pop.append(faceRow);

      const stepper = (
        label: string,
        value: number,
        fmt: (n: number) => string,
        key: keyof FontSettings,
        bounds: { min: number; max: number; step: number },
      ) => {
        const row = el("div", "font-pop-row");
        const name = el("span", "font-pop-label");
        name.textContent = label;
        const ctl = el("div", "font-step");
        const dec = el("button");
        dec.textContent = "−";
        const val = el("span", "font-step-val");
        val.textContent = fmt(value);
        const inc = el("button");
        inc.textContent = "+";
        const set = (n: number) => {
          setFont({ [key]: clamp(Math.round(n / bounds.step) * bounds.step, bounds.min, bounds.max) } as Partial<FontSettings>);
          fontPopover.rerender();
        };
        dec.addEventListener("click", () => set(value - bounds.step));
        inc.addEventListener("click", () => set(value + bounds.step));
        ctl.append(dec, val, inc);
        row.append(name, ctl);
        return row;
      };

      pop.append(
        stepper("Size", font.size, (n) => `${n}px`, "size", FONT_BOUNDS.size),
        stepper("Line height", font.lineHeight, (n) => n.toFixed(2), "lineHeight", FONT_BOUNDS.lineHeight),
        stepper("Tracking", font.letterSpacing, (n) => `${n}px`, "letterSpacing", FONT_BOUNDS.letterSpacing),
      );

      const reset = el("button", "font-pop-reset");
      reset.textContent = "Reset to defaults";
      reset.addEventListener("click", () => { setFont({ ...DEFAULT_FONT }); fontPopover.rerender(); });
      pop.append(reset);
    },
    hooks.onChromeChange,
  );

  function swatchOf(t: TermTheme): HTMLElement {
    const swatch = el("div", "theme-swatch");
    swatch.style.background = t.background;
    for (const k of SWATCH_KEYS) {
      const dot = el("span", "theme-dot");
      dot.style.background = t[k];
      swatch.append(dot);
    }
    const fg = el("span", "theme-dot theme-dot--fg");
    fg.style.background = t.foreground;
    swatch.append(fg);
    return swatch;
  }

  /** The list view: shimmer, then Recent, Custom, Dark, Light. */
  function renderList(pop: HTMLElement) {
    const shim = el("div", "theme-shimmer");
    const label = el("span", "theme-shimmer-label");
    label.textContent = "Shimmer";
    const seg = el("div", "theme-seg");
    for (const level of SHIMMER_LEVELS) {
      const b = el("button", `theme-seg-btn${level === shimmerLevel ? " theme-seg-btn--on" : ""}`);
      b.textContent = level[0]!.toUpperCase() + level.slice(1);
      b.addEventListener("click", () => setShimmer(level));
      seg.append(b);
    }
    shim.append(label, seg);
    pop.append(shim);

    const recentSchemes = recent.map(resolve).filter((s): s is Scheme => !!s).slice(0, RECENT_SHOWN);
    const builtins = SCHEMES;
    const groups: [string, Scheme[]][] = [
      ["Recent", recentSchemes.length >= 2 ? recentSchemes : []],
      ["Custom", customs],
      ["Dark", builtins.filter((s) => s.dark)],
      ["Light", builtins.filter((s) => !s.dark)],
    ];
    let current: HTMLElement | null = null;
    for (const [title, schemes] of groups) {
      if (!schemes.length) continue;
      const head = el("div", "theme-group");
      head.textContent = title;
      pop.append(head);
      for (const s of schemes) {
        const on = s.id === scheme.id;
        const row = el("div", `theme-row${on ? " theme-row--on" : ""}`);
        if (on && !current && title !== "Recent") current = row;

        const pick = el("button", "theme-row-pick");
        const name = el("span", "theme-row-name");
        name.textContent = s.name;
        pick.append(swatchOf(s.theme), name);
        pick.addEventListener("click", () => applyScheme(s));

        const edit = el("button", "theme-row-edit");
        edit.title = s.custom ? `Edit ${s.name}` : `Copy ${s.name} to a theme of your own`;
        edit.append(icon(s.custom ? "pencil" : "copy", 13));
        edit.addEventListener("click", () => startEdit(s));

        row.append(pick, edit);
        pop.append(row);
      }
    }

    const add = el("button", "theme-new");
    add.append(icon("plus", 13), document.createTextNode("New theme"));
    add.title = "Start a theme from the current one, or paste one in";
    add.addEventListener("click", () => {
      startEdit({ ...scheme, custom: false, name: "Untitled" });
      if (draft) { draft.name = "Untitled"; draft.named = false; }
      themePopover.rerender();
    });
    pop.append(add);

    // The list scrolls; open on the active scheme.
    if (current) requestAnimationFrame(() => current?.scrollIntoView({ block: "nearest" }));
  }

  /** One color: a native picker plus a hex field, both live. */
  function colorField(d: Draft, key: ThemeKey, label?: string): HTMLElement {
    const cell = el("label", "theme-field");
    if (label) {
      const l = el("span", "theme-field-label");
      l.textContent = label;
      cell.append(l);
    }
    const picker = el("input", "theme-field-picker");
    picker.type = "color";
    picker.value = d.colors[key] ?? "#000000";
    const hex = el("input", "theme-field-hex");
    hex.value = d.colors[key] ?? "";
    hex.spellcheck = false;
    hex.setAttribute("aria-label", `${label ?? key} hex`);
    picker.title = key;
    picker.addEventListener("input", () => {
      d.colors[key] = picker.value;
      hex.value = picker.value;
      previewDraft();
    });
    hex.addEventListener("change", () => {
      const v = normalizeHex(hex.value);
      if (v) { d.colors[key] = v; picker.value = v; previewDraft(); }
      hex.value = d.colors[key] ?? "";
    });
    cell.append(picker, hex);
    return cell;
  }

  /** The editor view: name, paste box, every color, and the actions. */
  function renderEditor(pop: HTMLElement, d: Draft) {
    const head = el("div", "theme-edit-head");
    const back = el("button", "theme-edit-back");
    back.textContent = "‹ Themes";
    back.addEventListener("click", () => { endEdit(true); themePopover.rerender(); });
    const title = el("span", "theme-edit-title");
    title.textContent = d.id ? "Edit theme" : "New theme";
    head.append(back, title);

    const name = el("input", "theme-edit-name");
    name.value = d.name;
    name.placeholder = "Name";
    name.setAttribute("aria-label", "Theme name");
    name.addEventListener("input", () => { d.name = name.value; d.named = true; });

    const paste = el("textarea", "theme-edit-paste");
    paste.rows = 3;
    paste.value = d.paste;
    paste.spellcheck = false;
    paste.placeholder = "Paste a theme: kitty, Ghostty, Alacritty, WezTerm, Windows Terminal, VS Code, Xresources, CSS variables, JSON, or 16 hex codes";
    paste.setAttribute("aria-label", "Paste a theme");
    paste.addEventListener("input", () => {
      applyPaste(d, paste.value);
      // The rerender replaced the box; keep typing where it was.
      const again = pop.querySelector<HTMLTextAreaElement>(".theme-edit-paste");
      if (again) { again.focus(); again.selectionStart = again.selectionEnd = again.value.length; }
    });

    const status = el("div", `theme-edit-status${d.error ? " theme-edit-status--error" : ""}`);
    status.textContent = d.status;

    const baseGrid = el("div", "theme-edit-base");
    for (const [k, label] of BASE_FIELDS) baseGrid.append(colorField(d, k, label));

    const ansi = el("div", "theme-edit-ansi");
    for (const h of ["", "Normal", "Bright"]) {
      const c = el("span", "theme-edit-ansi-head");
      c.textContent = h;
      ansi.append(c);
    }
    for (const n of ANSI_ROWS) {
      const l = el("span", "theme-field-label");
      l.textContent = n;
      ansi.append(l, colorField(d, n), colorField(d, brightOf(n)));
    }

    const actions = el("div", "theme-edit-actions");
    if (d.id) {
      const del = el("button", "theme-edit-btn theme-edit-btn--danger");
      del.textContent = "Delete";
      del.addEventListener("click", () => void deleteDraft(d));
      actions.append(del);
    }
    const spacer = el("span", "theme-edit-spacer");
    const copy = el("button", "theme-edit-btn");
    copy.textContent = "Copy JSON";
    copy.title = "The theme as /api/themes takes it";
    copy.addEventListener("click", () => {
      void navigator.clipboard?.writeText(JSON.stringify({ name: d.name, theme: d.colors }, null, 2));
      d.status = "Copied.";
      d.error = false;
      themePopover.rerender();
    });
    const cancel = el("button", "theme-edit-btn");
    cancel.textContent = "Cancel";
    cancel.addEventListener("click", () => { endEdit(true); themePopover.rerender(); });
    const save = el("button", "theme-edit-btn theme-edit-btn--primary");
    save.textContent = "Save";
    save.addEventListener("click", () => {
      if (!d.name.trim()) { d.status = "Give it a name."; d.error = true; themePopover.rerender(); return; }
      void saveDraft(d);
    });
    actions.append(spacer, copy, cancel, save);

    const hint = el("div", "theme-edit-hint");
    hint.textContent = "A Claude session can add one too: PUT /api/themes/<id> on this daemon.";

    pop.append(head, name, paste, status, baseGrid, ansi, actions, hint);
  }

  /** Theme popover — pick a scheme by sight (live swatch strips), or edit one. */
  const themePopover = createPopover(
    "theme-pop",
    ".theme-btn",
    (pop) => {
      pop.classList.toggle("theme-pop--edit", draft !== null);
      if (draft) renderEditor(pop, draft);
      else renderList(pop);
    },
    hooks.onChromeChange,
    () => endEdit(true),
  );

  return {
    scheme: () => scheme,
    font: () => font,
    themePopover,
    fontPopover,
  };
}
