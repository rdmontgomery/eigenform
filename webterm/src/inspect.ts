/**
 * inspect.ts — the config-inventory surface for the browser.
 *
 * webterm's "inspect panel" (the drawer dock) is session-centric: reach map +
 * transcript. Skills and memory — the config that shapes every session — had no
 * browser surface; they lived only as a CLI stdout dump. This module is that
 * surface: it fetches the unified inventory from `GET /api/inspect` (skills +
 * memory across resolution layers, every entry token-budgeted, skills carrying
 * their shadowing verdict) and renders it as a navigable, collapsible tree in a
 * full-area overlay.
 *
 * Split mirrors toolview.ts / drawer.ts: the data shaping (wire types, token
 * formatting, shadowing verdict, summary) is PURE and unit-tested with
 * `node --test`; the DOM/overlay below references `document` only inside function
 * bodies, so importing this module under the test runner is side-effect-free.
 */

import { icon } from "./icons.ts";

// ---------------------------------------------------------------------------
// Wire types — mirror eigenform_render::inspect_json field-for-field.
// ---------------------------------------------------------------------------

export interface InspectSkill {
  name: string;
  description: string;
  path: string;
  size: number;
  tokens: number;
  /** True when this contribution wins resolution for its name. */
  wins: boolean;
  /** True when the name resolves through plugin namespacing, not shadowing. */
  namespaced: boolean;
}

export interface InspectMemory {
  name: string;
  description: string;
  /** The memory `type` tag (feedback / project / reference / user / …). */
  kind: string;
  path: string;
  size: number;
  tokens: number;
}

export interface InspectLayer {
  /** Short label, e.g. `global` / `plugin:foo` / `repo:eigenform`. */
  label: string;
  tokens: number;
  skills: InspectSkill[];
  memory: InspectMemory[];
}

export interface InspectData {
  tokens: number;
  layers: InspectLayer[];
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/** Token count for display: `~N tok` under 1k, `~N.Nk tok` above. Mirrors the
 *  Rust `fmt_tokens` so the CLI and the browser read identically. */
export function fmtTokens(tokens: number): string {
  if (tokens >= 1000) return `~${(tokens / 1000).toFixed(1)}k tok`;
  return `~${tokens} tok`;
}

export type SkillStatus = "wins" | "shadowed" | "namespaced";

/** The resolution verdict for a skill contribution. */
export function skillStatus(s: InspectSkill): SkillStatus {
  if (s.namespaced) return "namespaced";
  return s.wins ? "wins" : "shadowed";
}

export interface InspectSummary {
  layers: number;
  tokens: number;
  skills: number;
  memory: number;
  /** How many skill contributions are shadowed (a real override happened). */
  shadowed: number;
}

/** Top-line counts for the overlay header. */
export function inspectSummary(data: InspectData): InspectSummary {
  let skills = 0;
  let memory = 0;
  let shadowed = 0;
  for (const layer of data.layers) {
    skills += layer.skills.length;
    memory += layer.memory.length;
    for (const s of layer.skills) {
      if (skillStatus(s) === "shadowed") shadowed += 1;
    }
  }
  return { layers: data.layers.length, tokens: data.tokens, skills, memory, shadowed };
}

// ---------------------------------------------------------------------------
// DOM — the navigable tree (typecheck-only; no node --test coverage)
// ---------------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  return e;
}

/** A collapsible row: a header (click to toggle) over a children container. */
function group(headerContent: (h: HTMLElement) => void, open = true): { row: HTMLElement; body: HTMLElement } {
  const row = el("div", "ix-group");
  const header = el("button", "ix-grouphead");
  const caret = icon("chevron", 12);
  caret.classList.add("ix-caret");
  header.append(caret);
  headerContent(header);
  const body = el("div", "ix-groupbody");
  row.append(header, body);
  const apply = () => {
    row.classList.toggle("ix-open", open);
    body.style.display = open ? "" : "none";
  };
  header.addEventListener("click", () => {
    open = !open;
    apply();
  });
  apply();
  return { row, body };
}

function tokenTag(tokens: number): HTMLElement {
  const t = el("span", "ix-tok");
  t.textContent = fmtTokens(tokens);
  return t;
}

function skillRow(s: InspectSkill): HTMLElement {
  const status = skillStatus(s);
  const { row, body } = group((h) => {
    h.append(icon("skill", 13));
    const name = el("span", "ix-name");
    name.textContent = s.name;
    const badge = el("span", `ix-badge ix-badge--${status}`);
    badge.textContent = status;
    h.append(name, badge, tokenTag(s.tokens));
  }, false);
  if (s.description) {
    const desc = el("div", "ix-desc");
    desc.textContent = s.description;
    body.append(desc);
  }
  const path = el("div", "ix-path");
  path.textContent = s.path;
  body.append(path);
  return row;
}

function memoryRow(m: InspectMemory): HTMLElement {
  const { row, body } = group((h) => {
    h.append(icon("doc", 13));
    const name = el("span", "ix-name");
    name.textContent = m.name;
    const badge = el("span", "ix-badge ix-badge--kind");
    badge.textContent = m.kind;
    h.append(name, badge, tokenTag(m.tokens));
  }, false);
  if (m.description) {
    const desc = el("div", "ix-desc");
    desc.textContent = m.description;
    body.append(desc);
  }
  const path = el("div", "ix-path");
  path.textContent = m.path;
  body.append(path);
  return row;
}

function layerNode(layer: InspectLayer): HTMLElement {
  const { row, body } = group((h) => {
    h.append(icon("panel", 13));
    const label = el("span", "ix-label");
    label.textContent = layer.label;
    h.append(label, tokenTag(layer.tokens));
  });

  if (layer.skills.length) {
    const skillTok = layer.skills.reduce((a, s) => a + s.tokens, 0);
    const { row: g, body: gb } = group((h) => {
      const t = el("span", "ix-subhead");
      t.textContent = `skills (${layer.skills.length})`;
      h.append(t, tokenTag(skillTok));
    });
    for (const s of layer.skills) gb.append(skillRow(s));
    body.append(g);
  }
  if (layer.memory.length) {
    const memTok = layer.memory.reduce((a, m) => a + m.tokens, 0);
    const { row: g, body: gb } = group((h) => {
      const t = el("span", "ix-subhead");
      t.textContent = `memory (${layer.memory.length})`;
      h.append(t, tokenTag(memTok));
    });
    for (const m of layer.memory) gb.append(memoryRow(m));
    body.append(g);
  }
  if (!layer.skills.length && !layer.memory.length) {
    const empty = el("div", "ix-empty");
    empty.textContent = "(empty)";
    body.append(empty);
  }
  return row;
}

/** Build the config tree for an InspectData payload. */
export function renderInspectTree(data: InspectData): HTMLElement {
  const root = el("div", "ix-tree");
  if (!data.layers.length) {
    const empty = el("div", "ix-empty");
    empty.textContent = "(no config found)";
    root.append(empty);
    return root;
  }
  for (const layer of data.layers) root.append(layerNode(layer));
  return root;
}

// ---------------------------------------------------------------------------
// Overlay — fetch + scope toggle + render
// ---------------------------------------------------------------------------

export interface InspectOptions {
  /** When set, a "This project" scope is offered, resolved against this cwd. */
  cwd?: string;
}

/** Open the config-inventory overlay. Returns a close handle. */
export function openInspect(opts: InspectOptions = {}): { close: () => void } {
  const backdrop = el("div", "ix-backdrop");
  const panel = el("div", "ix-panel");
  backdrop.append(panel);

  const head = el("div", "ix-head");
  const titleWrap = el("div", "ix-title");
  titleWrap.append(icon("skill", 16));
  const title = el("span");
  title.textContent = "config";
  const sub = el("span", "ix-subtitle");
  titleWrap.append(title, sub);

  // Scope toggle: "All projects" always; "This project" only when a cwd is known.
  const scopes = el("div", "ix-scopes");
  // `all` is the default unless we have a cwd to focus.
  let allProjects = !opts.cwd;
  const allBtn = el("button", "ix-scope");
  allBtn.textContent = "All projects";
  const hereBtn = el("button", "ix-scope");
  hereBtn.textContent = "This project";
  scopes.append(allBtn);
  if (opts.cwd) scopes.append(hereBtn);

  const closeBtn = el("button", "ix-close icon-btn");
  closeBtn.title = "Close (Esc)";
  closeBtn.append(icon("x", 16));

  head.append(titleWrap, scopes, closeBtn);

  const bodyScroll = el("div", "ix-body scroll");
  panel.append(head, bodyScroll);

  function syncScopeButtons() {
    allBtn.classList.toggle("ix-scope--active", allProjects);
    hereBtn.classList.toggle("ix-scope--active", !allProjects);
  }

  async function load() {
    syncScopeButtons();
    bodyScroll.innerHTML = "";
    const loading = el("div", "ix-status");
    loading.textContent = "loading…";
    bodyScroll.append(loading);
    const url =
      allProjects || !opts.cwd
        ? "/api/inspect"
        : `/api/inspect?cwd=${encodeURIComponent(opts.cwd)}`;
    try {
      const res = await fetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as InspectData;
      bodyScroll.innerHTML = "";
      const s = inspectSummary(data);
      sub.textContent = `${s.layers} layers · ${s.skills} skills · ${s.memory} memory · ${fmtTokens(s.tokens)}${s.shadowed ? ` · ${s.shadowed} shadowed` : ""}`;
      bodyScroll.append(renderInspectTree(data));
    } catch (err) {
      bodyScroll.innerHTML = "";
      const e = el("div", "ix-status ix-status--err");
      e.textContent = `could not load config inventory (${String(err)})`;
      bodyScroll.append(e);
    }
  }

  allBtn.addEventListener("click", () => {
    if (allProjects) return;
    allProjects = true;
    void load();
  });
  hereBtn.addEventListener("click", () => {
    if (!allProjects) return;
    allProjects = false;
    void load();
  });

  function close() {
    backdrop.remove();
    window.removeEventListener("keydown", onKey);
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }
  closeBtn.addEventListener("click", close);
  backdrop.addEventListener("mousedown", (e) => {
    if (e.target === backdrop) close();
  });
  window.addEventListener("keydown", onKey);

  document.body.append(backdrop);
  void load();
  return { close };
}
