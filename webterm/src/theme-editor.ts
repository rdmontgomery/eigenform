// Theme editor: a floating panel that edits a scheme's colors live. Every change
// repaints the terminals and chrome at once (via host.preview); Save keeps the
// edit as a named custom theme in the daemon, Cancel puts the old theme back.

import { el } from "./dom.ts";
import { deriveChrome, oklchToHex, parseOklch, OVERRIDE_KEYS } from "./themes/derive.ts";
import type { ChromeOverrides } from "./themes/derive.ts";
import { customToScheme, exportTheme, paletteOf, themeIdFor } from "./themes/custom.ts";
import type { CustomTheme, PaletteKey } from "./themes/custom.ts";
import type { Scheme } from "./themes/schemes.ts";

export interface EditorHost {
  /** Every scheme id in use (built-in and custom), to keep new ids unique. */
  allIds(): string[];
  builtin(id: string): Scheme | undefined;
  /** Paint a scheme without persisting it as the active choice. */
  preview(s: Scheme): void;
  /** Paint and persist a scheme as the active choice. */
  commit(s: Scheme): void;
  save(t: CustomTheme): Promise<void>;
  remove(id: string): Promise<void>;
  onClose(): void;
}

const LABELS: Record<PaletteKey, string> = {
  background: "Background", foreground: "Foreground", cursor: "Cursor", selectionBackground: "Selection",
  black: "Black", red: "Red", green: "Green", yellow: "Yellow",
  blue: "Blue", magenta: "Magenta", cyan: "Cyan", white: "White",
  brightBlack: "Bright black", brightRed: "Bright red", brightGreen: "Bright green", brightYellow: "Bright yellow",
  brightBlue: "Bright blue", brightMagenta: "Bright magenta", brightCyan: "Bright cyan", brightWhite: "Bright white",
};
const GROUPS: { title: string; keys: PaletteKey[] }[] = [
  { title: "Surface", keys: ["background", "foreground", "cursor", "selectionBackground"] },
  { title: "Normal", keys: ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] },
  { title: "Bright", keys: ["brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite"] },
];
const OVERRIDE_LABELS: Record<(typeof OVERRIDE_KEYS)[number], [string, string]> = {
  accent: ["Accent", "--accent"],
  running: ["Running", "--st-running"],
  idle: ["Idle", "--st-idle"],
  error: ["Error", "--st-error"],
  success: ["Success", "--st-success"],
};

interface Draft {
  name: string;
  theme: Record<PaletteKey, string>;
  overrides: ChromeOverrides;
}

export interface ThemeEditor {
  close(): void;
  isOpen(): boolean;
}

/** Open the editor on `start` (the active scheme). */
export function openThemeEditor(start: Scheme, host: EditorHost): ThemeEditor {
  const baseId = start.custom ? start.base : start.id;
  const fresh = (): Draft => ({
    name: start.custom ? start.name : `${start.name} (custom)`,
    theme: paletteOf(start.theme),
    overrides: { ...(start.overrides ?? {}) },
  });
  let draft = fresh();
  let dirty = false;
  let status = "";
  let frame = 0;

  const panel = el("div", "theme-editor");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-label", "Edit theme colors");
  document.body.append(panel);

  const draftScheme = (): Scheme => {
    const s = customToScheme({ id: start.id, name: draft.name, theme: draft.theme, overrides: draft.overrides });
    s.custom = start.custom ?? false;
    if (baseId) s.base = baseId;
    if (!Object.keys(draft.overrides).length) delete s.overrides;
    return s;
  };

  /** Repaint at most once a frame: a dragged color picker fires many inputs. */
  function schedulePreview() {
    if (frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      host.preview(draftScheme());
      syncDerived();
    });
  }

  function change(mut: () => void) {
    mut();
    dirty = true;
    status = "";
    schedulePreview();
    syncFooter();
  }

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !(e.target as HTMLElement).closest?.("input[type=text]")) {
      e.preventDefault();
      cancel();
    }
  };
  document.addEventListener("keydown", onKey, true);

  // ---- body -------------------------------------------------------------
  const head = el("div", "te-head");
  const title = el("span", "te-title");
  title.textContent = "Edit theme";
  const nameInput = el("input", "te-name");
  nameInput.type = "text";
  nameInput.spellcheck = false;
  nameInput.addEventListener("input", () => change(() => (draft.name = nameInput.value)));
  head.append(title, nameInput);

  const body = el("div", "te-body");
  const swatchInputs = new Map<PaletteKey, { color: HTMLInputElement; hex: HTMLInputElement }>();

  function colorRow(label: string, value: string, onSet: (hex: string) => void) {
    const row = el("label", "te-row");
    const color = el("input", "te-color");
    color.type = "color";
    color.value = value;
    const name = el("span", "te-label");
    name.textContent = label;
    const hex = el("input", "te-hex");
    hex.type = "text";
    hex.value = value;
    hex.spellcheck = false;
    hex.maxLength = 7;
    color.addEventListener("input", () => { hex.value = color.value; onSet(color.value); });
    hex.addEventListener("input", () => {
      const v = hex.value.trim().toLowerCase();
      const ok = /^#?[0-9a-f]{6}$/.test(v);
      hex.classList.toggle("te-hex--bad", !ok);
      if (ok) { const h = v.startsWith("#") ? v : `#${v}`; color.value = h; onSet(h); }
    });
    row.append(color, name, hex);
    return { row, color, hex };
  }

  for (const g of GROUPS) {
    const sec = el("div", "te-group");
    const h = el("div", "te-group-title");
    h.textContent = g.title;
    sec.append(h);
    const grid = el("div", "te-grid");
    for (const k of g.keys) {
      const r = colorRow(LABELS[k], draft.theme[k], (v) => change(() => (draft.theme[k] = v)));
      swatchInputs.set(k, r);
      grid.append(r.row);
    }
    sec.append(grid);
    body.append(sec);
  }

  // Chrome overrides: "auto" shows the derived color; picking a color pins it.
  const ovSec = el("div", "te-group");
  const ovTitle = el("div", "te-group-title");
  ovTitle.textContent = "Chrome (auto = derived from the palette)";
  ovSec.append(ovTitle);
  const ovGrid = el("div", "te-grid te-grid--chrome");
  const ovRows = new Map<(typeof OVERRIDE_KEYS)[number], { row: HTMLElement; color: HTMLInputElement; hex: HTMLInputElement; auto: HTMLInputElement }>();
  for (const k of OVERRIDE_KEYS) {
    const r = colorRow(OVERRIDE_LABELS[k][0], "#000000", (v) => change(() => {
      draft.overrides[k] = v;
      ovRows.get(k)!.auto.checked = false;
    }));
    const auto = el("input", "te-auto");
    auto.type = "checkbox";
    auto.title = "Derive from the palette";
    auto.addEventListener("change", () => change(() => {
      if (auto.checked) delete draft.overrides[k];
      else draft.overrides[k] = r.color.value;
    }));
    const autoLabel = el("label", "te-auto-label");
    autoLabel.append(auto, document.createTextNode("auto"));
    r.row.append(autoLabel);
    ovRows.set(k, { row: r.row, color: r.color, hex: r.hex, auto });
    ovGrid.append(r.row);
  }
  ovSec.append(ovGrid);
  body.append(ovSec);

  /** Show what "auto" currently derives to, so pinning starts from it. */
  function syncDerived() {
    const tokens = deriveChrome(draft.theme, draft.overrides);
    for (const [k, r] of ovRows) {
      const pinned = draft.overrides[k];
      r.auto.checked = !pinned;
      const lch = parseOklch(tokens[OVERRIDE_LABELS[k][1]] ?? "");
      const hex = pinned ?? (lch ? oklchToHex(lch) : "#000000");
      if (document.activeElement !== r.hex) r.hex.value = hex;
      r.color.value = hex;
      r.row.classList.toggle("te-row--auto", !pinned);
    }
  }

  /** Push the whole draft into the inputs (after reset or open). */
  function syncInputs() {
    nameInput.value = draft.name;
    for (const [k, r] of swatchInputs) {
      r.color.value = draft.theme[k];
      r.hex.value = draft.theme[k];
      r.hex.classList.remove("te-hex--bad");
    }
    syncDerived();
  }

  // ---- footer -----------------------------------------------------------
  const foot = el("div", "te-foot");
  const msg = el("div", "te-msg");
  const actions = el("div", "te-actions");

  const btn = (label: string, cls: string, fn: () => void) => {
    const b = el("button", `te-btn ${cls}`);
    b.textContent = label;
    b.addEventListener("click", fn);
    return b;
  };

  function resetToOriginal() {
    const orig = (baseId && host.builtin(baseId)) || (start.custom ? null : start);
    if (!orig) { status = "No built-in to reset to."; syncFooter(); return; }
    change(() => {
      draft = { name: draft.name, theme: paletteOf(orig.theme), overrides: { ...(orig.overrides ?? {}) } };
    });
    syncInputs();
  }

  function toCustom(id: string): CustomTheme {
    const t: CustomTheme = { id, name: draft.name.trim() || "Untitled", theme: { ...draft.theme } };
    if (baseId) t.base = baseId;
    if (Object.keys(draft.overrides).length) t.overrides = { ...draft.overrides };
    return t;
  }

  async function save(asNew: boolean) {
    const id = start.custom && !asNew ? start.id : themeIdFor(draft.name, host.allIds());
    const t = toCustom(id);
    try {
      await host.save(t);
      host.commit(customToScheme(t));
      dirty = false;
      close();
    } catch (e) {
      status = `Couldn't save: ${(e as Error).message}`;
      syncFooter();
    }
  }

  async function remove() {
    try {
      await host.remove(start.id);
      host.commit((baseId && host.builtin(baseId)) || host.builtin("warm-ink-dark")!);
      dirty = false;
      close();
    } catch (e) {
      status = `Couldn't delete: ${(e as Error).message}`;
      syncFooter();
    }
  }

  function exportDraft() {
    const t = toCustom(start.id);
    const blob = new Blob([exportTheme(t)], { type: "application/json" });
    const a = el("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${t.name.replace(/[^\w.-]+/g, "-")}.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  }

  function cancel() {
    if (dirty) host.preview(start);
    close();
  }

  function syncFooter() {
    msg.textContent = status || (dirty ? "Unsaved changes, previewing live." : "");
    msg.classList.toggle("te-msg--err", /^Couldn't|^No /.test(status));
    actions.innerHTML = "";
    actions.append(btn("Reset to original", "te-btn--ghost", resetToOriginal));
    actions.append(btn("Export", "te-btn--ghost", exportDraft));
    if (start.custom) actions.append(btn("Delete", "te-btn--ghost te-btn--danger", () => void remove()));
    actions.append(el("span", "te-spacer"));
    actions.append(btn(dirty ? "Cancel" : "Close", "", cancel));
    if (start.custom) actions.append(btn("Save as copy", "", () => void save(true)));
    actions.append(btn(start.custom ? "Save" : "Save as theme", "te-btn--primary", () => void save(false)));
  }

  foot.append(msg, actions);
  panel.append(head, body, foot);
  syncInputs();
  syncFooter();
  nameInput.focus();
  nameInput.select();

  let open = true;
  function close() {
    if (!open) return;
    open = false;
    if (frame) cancelAnimationFrame(frame);
    document.removeEventListener("keydown", onKey, true);
    panel.remove();
    host.onClose();
  }

  return { close: cancel, isOpen: () => open };
}
