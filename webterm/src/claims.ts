/**
 * claims.ts — the "what does Claude think is running" modal.
 *
 * Claude Code drops a `~/.claude/sessions/<pid>.json` claim for every session it
 * runs and (usually) removes it on exit. The daemon's `GET /api/claims` reads every
 * claim and judges it against the process table: alive, dead (pid gone, claim left
 * behind), or reused (pid alive but now a different process — a ghost). This
 * overlay lists them so old sessions can be ended at a glance and stale claims
 * swept, with headless (`claude -p` / SDK) runs separable from interactive ones.
 *
 * Split mirrors inspect.ts: the shaping below is PURE and unit-tested with
 * `node --test`; the DOM overlay references `document` only inside function
 * bodies, so importing this module under the test runner is side-effect-free.
 */

import { icon } from "./icons.ts";
import type { Claim } from "./types.ts";
import { el } from "./dom.ts";

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

export type ClaimFilter = "all" | "interactive" | "headless";

export interface ClaimSummary {
  alive: number;
  headless: number;
  stale: number;
}

/** Counts for the header: live claims, how many of those are headless, and the
 *  stale (dead or reused) claims a sweep would clear. */
export function summarizeClaims(claims: Claim[]): ClaimSummary {
  const alive = claims.filter((c) => c.health === "alive");
  return {
    alive: alive.length,
    headless: alive.filter((c) => c.headless).length,
    stale: claims.length - alive.length,
  };
}

export function filterClaims(claims: Claim[], f: ClaimFilter): Claim[] {
  if (f === "all") return claims;
  return claims.filter((c) => (f === "headless" ? c.headless : !c.headless));
}

/** Split into the live list and the stale list, each newest-first. */
export function partitionClaims(claims: Claim[]): { alive: Claim[]; stale: Claim[] } {
  const newest = (a: Claim, b: Claim) => (b.startedAt ?? 0) - (a.startedAt ?? 0);
  return {
    alive: claims.filter((c) => c.health === "alive").sort(newest),
    stale: claims.filter((c) => c.health !== "alive").sort(newest),
  };
}

/** Best human name: AI title → Claude's derived name → short session id. */
export function claimLabel(c: Claim): string {
  return c.title || c.name || c.sessionId.slice(0, 8);
}

/** Claude's own busy/idle status → the rail's activity vocabulary. */
export function claimActivity(c: Claim): "working" | "waiting" | "idle" {
  if (c.status === "busy") return "working";
  if (c.status === "idle") return "waiting";
  return "idle";
}

/** Compact age since the claim started: `42s`, `17m`, `3h`, `5d`. */
export function claimAge(startedAt: number | null, now: number): string {
  if (startedAt === null) return "";
  const s = Math.max(0, Math.floor((now - startedAt) / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

function basename(p: string | null): string {
  if (!p) return "";
  const parts = p.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? "/";
}

// ---------------------------------------------------------------------------
// DOM overlay (typecheck-only; no node --test coverage)
// ---------------------------------------------------------------------------


export interface ClaimsOptions {
  /** Called after any claim is ended or cleared, so the rail can refresh. */
  onChange?: () => void;
  /** Focus the eigenform tab hosting this pty, if the user clicks a hosted row. */
  onOpenPty?: (ptyId: string) => void;
}

const POLL_MS = 3000;
/** How long an armed "end" button waits for its confirming second click. */
const ARM_MS = 3000;

export function openClaims(opts: ClaimsOptions = {}): { close: () => void } {
  const backdrop = el("div", "ix-backdrop");
  const panel = el("div", "ix-panel cl-panel");
  backdrop.append(panel);

  const head = el("div", "ix-head");
  const titleWrap = el("div", "ix-title");
  titleWrap.append(icon("pulse", 16));
  const title = el("span");
  title.textContent = "active sessions";
  const sub = el("span", "ix-subtitle");
  titleWrap.append(title, sub);

  let filter: ClaimFilter = "all";
  const scopes = el("div", "ix-scopes");
  const filterBtns = new Map<ClaimFilter, HTMLButtonElement>();
  for (const [f, label] of [
    ["all", "All"],
    ["interactive", "Interactive"],
    ["headless", "Headless"],
  ] as const) {
    const b = el("button", "ix-scope");
    b.textContent = label;
    b.addEventListener("click", () => {
      filter = f;
      render();
    });
    filterBtns.set(f, b);
    scopes.append(b);
  }

  const sweepBtn = el("button", "cl-sweep");
  sweepBtn.title = "Remove every claim whose process is gone or was replaced";
  sweepBtn.addEventListener("click", () => void sweep());

  const closeBtn = el("button", "ix-close icon-btn");
  closeBtn.title = "Close (Esc)";
  closeBtn.append(icon("x", 16));
  head.append(titleWrap, scopes, sweepBtn, closeBtn);

  const bodyScroll = el("div", "ix-body scroll");
  panel.append(head, bodyScroll);

  let claims: Claim[] = [];
  let error: string | null = null;
  let loaded = false;
  /** pid of the row whose end button is armed (awaiting confirmation). */
  let armed: number | null = null;
  let armTimer: number | undefined;
  /** pids with a request in flight — their buttons show a spinner state. */
  const pending = new Set<number>();

  async function load() {
    try {
      const res = await fetch("/api/claims");
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      claims = (await res.json()) as Claim[];
      error = null;
    } catch (err) {
      error = String(err);
    }
    loaded = true;
    render();
  }

  async function retire(pid: number) {
    pending.add(pid);
    render();
    try {
      const res = await fetch(`/api/claims/${pid}`, { method: "DELETE" });
      if (!res.ok) console.warn(`retire claim ${pid}: HTTP ${res.status} ${await res.text()}`);
    } catch (err) {
      console.warn(`retire claim ${pid}:`, err);
    }
    // SIGTERM → exit → claim removal takes a beat; re-read after it settles.
    window.setTimeout(() => {
      pending.delete(pid);
      void load();
      opts.onChange?.();
    }, 600);
  }

  async function sweep() {
    const stale = claims.filter((c) => c.health !== "alive").map((c) => c.pid);
    for (const pid of stale) pending.add(pid);
    render();
    await Promise.all(
      stale.map((pid) => fetch(`/api/claims/${pid}`, { method: "DELETE" }).catch(() => null)),
    );
    for (const pid of stale) pending.delete(pid);
    await load();
    opts.onChange?.();
  }

  function arm(pid: number) {
    armed = pid;
    window.clearTimeout(armTimer);
    armTimer = window.setTimeout(() => {
      armed = null;
      render();
    }, ARM_MS);
    render();
  }

  function renderRow(c: Claim, now: number): HTMLElement {
    const row = el("div", `cl-row cl-row--${c.health}`);

    const act = c.health === "alive" ? claimActivity(c) : "idle";
    const prov = c.health !== "alive" ? "dead" : c.ptyId ? "eigenform" : "external";
    const dot = el("span", `dot dot--${act} dot--${prov}`);
    dot.title =
      c.health === "alive"
        ? act === "working" ? "busy" : act === "waiting" ? "idle at prompt" : "running"
        : c.health === "dead" ? "process gone — claim left behind" : "pid reused by another process";

    const body = el("div", "cl-body");
    const label = el("div", "cl-label");
    label.textContent = claimLabel(c);
    label.title = c.sessionId;
    const meta = el("div", "cl-meta");
    const dir = el("span", "cl-dir");
    dir.textContent = basename(c.cwd);
    dir.title = c.cwd ?? "";
    meta.append(dir);
    const facts = el("span", "cl-facts");
    const age = claimAge(c.startedAt, now);
    facts.textContent = `pid ${c.pid}${age ? ` · ${age}` : ""}`;
    meta.append(facts);
    if (c.headless) meta.append(badge("headless", "cl-badge--headless", c.entrypoint ?? ""));
    if (c.ptyId) meta.append(badge("eigenform", "cl-badge--hosted", `pty ${c.ptyId}`));
    if (c.health === "dead") meta.append(badge("dead", "cl-badge--stale"));
    if (c.health === "reused") meta.append(badge("pid reused", "cl-badge--stale"));
    body.append(label, meta);

    if (c.ptyId && opts.onOpenPty) {
      const ptyId = c.ptyId;
      body.classList.add("cl-body--link");
      body.title = "Go to this tab";
      body.addEventListener("click", () => {
        opts.onOpenPty!(ptyId);
        close();
      });
    }

    const btn = el("button", "cl-act");
    if (pending.has(c.pid)) {
      btn.textContent = "…";
      btn.disabled = true;
    } else if (c.health !== "alive") {
      btn.textContent = "clear";
      btn.title = "Remove this stale claim file";
      btn.addEventListener("click", () => void retire(c.pid));
    } else if (armed === c.pid) {
      btn.classList.add("cl-act--armed");
      btn.textContent = "end?";
      btn.title = "Click again to send SIGTERM";
      btn.addEventListener("click", () => {
        armed = null;
        void retire(c.pid);
      });
    } else {
      btn.classList.add("cl-act--end");
      btn.append(icon("x", 13, 2));
      btn.title = "End this session (SIGTERM) — click twice";
      btn.addEventListener("click", () => arm(c.pid));
    }

    row.append(dot, body, btn);
    return row;
  }

  function badge(text: string, cls: string, title = ""): HTMLElement {
    const b = el("span", `cl-badge ${cls}`);
    b.textContent = text;
    if (title) b.title = title;
    return b;
  }

  function section(name: string, rows: Claim[], now: number): HTMLElement | null {
    if (rows.length === 0) return null;
    const wrap = el("div", "cl-section");
    const h = el("div", "cl-section-head");
    h.textContent = `${name} · ${rows.length}`;
    wrap.append(h);
    for (const c of rows) wrap.append(renderRow(c, now));
    return wrap;
  }

  function render() {
    for (const [f, b] of filterBtns) b.classList.toggle("ix-scope--active", f === filter);
    const s = summarizeClaims(claims);
    sub.textContent = `${s.alive} running · ${s.headless} headless`;
    sweepBtn.textContent = `clear stale (${s.stale})`;
    sweepBtn.hidden = s.stale === 0;

    bodyScroll.innerHTML = "";
    if (!loaded) {
      bodyScroll.append(status("loading…"));
      return;
    }
    if (error) {
      bodyScroll.append(status(`could not load claims (${error})`, true));
      return;
    }
    const { alive, stale } = partitionClaims(filterClaims(claims, filter));
    const now = Date.now();
    const parts = [section("Running", alive, now), section("Stale claims", stale, now)].filter(
      (x): x is HTMLElement => x !== null,
    );
    if (parts.length === 0) bodyScroll.append(status("nothing claims to be running"));
    else bodyScroll.append(...parts);
  }

  function status(text: string, err = false): HTMLElement {
    const e = el("div", `ix-status${err ? " ix-status--err" : ""}`);
    e.textContent = text;
    return e;
  }

  const poll = window.setInterval(() => void load(), POLL_MS);

  function close() {
    window.clearInterval(poll);
    window.clearTimeout(armTimer);
    backdrop.remove();
    window.removeEventListener("keydown", onKey);
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (armed !== null) {
        armed = null;
        render();
      } else close();
    }
  }
  closeBtn.addEventListener("click", close);
  backdrop.addEventListener("mousedown", (e) => {
    if (e.target === backdrop) close();
  });
  window.addEventListener("keydown", onKey);

  document.body.append(backdrop);
  render();
  void load();
  return { close };
}
