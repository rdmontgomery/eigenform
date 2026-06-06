// data.ts — the single live/stub boundary. loadForest() is LIVE (real /api/sessions).
// loadSession(), MIND and MIND_DELTAS are STUBBED faithfully against the design's
// sample (turns 132–137 of a 137-turn session) until the backend grows a structured
// transcript + token-accounting endpoint. Swap the stub bodies; the view never changes.

export interface ToolDetail {
  tok: number;
  lines: { t: string; c: "dim" | "add" | "rem" | "cool" }[];
}
export interface Tool {
  kind: string;
  arg: string;
  delta: string;
  detail?: ToolDetail;
}
export interface Exchange {
  n: number;
  tok: number;
  user: string;
  assistant?: string;
  system?: string;
  tool?: Tool;
  leaf?: boolean;
  uuid?: string; // the user turn's real JSONL uuid — the fork target (live sessions only)
}
export interface Session {
  id: string;
  total: number;
  branches: number;
  windowStart: number;
  exchanges: Exchange[];
}

export type ForestState = "working" | "ready" | "recent";
export interface ForestEntry {
  id: string;
  name: string;
  state: ForestState;
  live: boolean;
  recency: string; // ISO timestamp, for the relative recency stamp
  shape: number[]; // the activity spark: output tokens per completed turn
  branches: number; // forks (0 until fork-lineage tracking exists)
  active: boolean; // the session currently shown in the Manuscript
  uuid?: string; // present for live entries (used to resume the pty)
}

// ── STUB: the sample session at 100s-scale ────────────────────────────────
export function loadSession(): Session {
  // TODO(live): fetch a structured JSON transcript (e.g. GET /api/session/:uuid.json)
  // and map it to Session. Today /api/session/:uuid returns rendered HTML only.
  return {
    id: "8f3ad1",
    total: 137,
    branches: 2,
    windowStart: 132,
    exchanges: [
      { n: 132, tok: 5200,
        user: "Let’s render the JSONL transcript as collapsible exchanges in the right pane.",
        assistant: "I’ll add a session_html() renderer that groups each user turn with its assistant + system replies into one <details>. Source of truth stays the JSONL — we never mutate it.",
        system: "8.2s · 4 files read · 1 written" },
      { n: 133, tok: 4800,
        user: "Good. Now follow the session live so the pane updates as it writes.",
        assistant: "Wired an SSE channel on /api/watch/:uuid; the pane re-fetches on each write and auto-scrolls unless you’ve scrolled up.",
        tool: { kind: "Edit", arg: "web/src/main.ts", delta: "+22 −3", detail: { tok: 1800, lines: [
          { t: "  watchSession(uuid, (ev) => {", c: "dim" },
          { t: "+   if (ev.boundary) repaint();   // flush, not per-line", c: "add" },
          { t: "-   repaint();                     // every append", c: "rem" },
          { t: "  });", c: "dim" },
        ] } },
        system: "5.6s" },
      { n: 134, tok: 3100,
        user: "The pane flickers on every keystroke during a live turn.",
        assistant: "The re-fetch fires per-line. I’ll debounce to the JSONL’s flush boundary instead of every append.",
        system: "3.1s" },
      { n: 135, tok: 6400,
        user: "Now let me edit a turn in place and have it fork.",
        assistant: "That’s edit_then_fork: re-author turn N, drop N+1…end, write a NEW session file, leave the original untouched. Copy-on-fork — the source can’t burn even in principle.",
        tool: { kind: "Bash", arg: "cargo test surgery::fork", delta: "ok · 12 passed", detail: { tok: 1400, lines: [
          { t: "running 12 tests", c: "dim" },
          { t: "test surgery::fork::keeps_original ... ok", c: "add" },
          { t: "test surgery::fork::drops_downstream ... ok", c: "add" },
          { t: "test result: ok. 12 passed; 0 failed", c: "cool" },
        ] } },
        system: "11.4s" },
      { n: 136, tok: 2600,
        user: "Show the fork cost in the margin as I hover each turn.",
        assistant: "Added a per-turn reading: cache temperature at the fork point, shaded hot→cold from the leaf backward. Recent turns are warm and cheap; the deep past is cold.",
        system: "4.0s" },
      { n: 137, tok: 0, user: "", leaf: true },
    ],
  };
}

// ── LIVE: a real session's transcript as structured exchanges ──────────────
// GET /api/session/:uuid/json (eigen_render::session_json). Content is ground truth;
// per-turn token/cost fields stay 0 (the cache model is still stubbed). Returns null
// if the session can't be loaded, so the caller can leave the current view in place.
export async function fetchSession(uuid: string): Promise<Session | null> {
  try {
    const res = await fetch(`/api/session/${encodeURIComponent(uuid)}/json`);
    if (!res.ok) return null;
    return (await res.json()) as Session;
  } catch {
    return null;
  }
}

// ── LIVE: edit-then-fork a session at a turn ───────────────────────────────
// POST /api/session/:src/fork {turn, text} → the new branch's uuid (copy-on-fork;
// the source is never mutated). Returns null on failure so the caller can stay put.
export async function forkSession(srcUuid: string, turn: string, text: string): Promise<string | null> {
  try {
    const res = await fetch(`/api/session/${encodeURIComponent(srcUuid)}/fork`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ turn, text }),
    });
    if (!res.ok) return null;
    const j = (await res.json()) as { uuid?: string };
    return typeof j.uuid === "string" ? j.uuid : null;
  } catch {
    return null;
  }
}

// ── STUB: the resident ledger · sums to CTX_FULL ──────────────────────────
export interface MindGroup {
  key: string;
  label: string;
  tok: number;
  count: number | null;
  unit?: string;
  note: string;
  items: string[];
}
export const MIND: { total: number; groups: MindGroup[] } = {
  total: 118000,
  groups: [
    { key: "system", label: "System prompt", tok: 2100, count: null, note: "identity · guardrails",
      items: ["core directives", "output contract", "never mutate the JSONL"] },
    { key: "tools", label: "Tool definitions", tok: 14200, count: 8, note: "callable this session",
      items: ["Edit", "Bash", "Read", "Write", "Grep", "Watch", "Fork", "Glob"] },
    { key: "skills", label: "Skills", tok: 6000, count: 3, note: "loaded procedures",
      items: ["render::session", "sse::watch", "surgery::fork"] },
    { key: "mcp", label: "MCP servers", tok: 9100, count: 2, unit: " servers", note: "connected",
      items: ["fs-bridge · 6 tools", "cargo-runner · 3 tools"] },
    { key: "memories", label: "Memories", tok: 4300, count: 5, note: "persistent notes",
      items: ["copy-on-fork is inviolable", "debounce to flush boundary", "JSONL is source of truth", "cost = cache temp × prefix", "leaf is free"] },
    { key: "files", label: "Files in context", tok: 31400, count: 11, note: "read this session",
      items: ["web/src/main.ts", "web/src/cost.ts", "session_html.rs", "surgery/fork.rs", "web/src/watch.ts", "+6 more"] },
    { key: "transcript", label: "Transcript", tok: 50900, count: 137, unit: " turns", note: "137 turns of exchange", items: [] },
  ],
};

export interface MindDelta { s: "+" | "−"; label: string; tok: number; }
export const MIND_DELTAS: Record<number, MindDelta[]> = {
  132: [{ s: "+", label: "4 files", tok: 12000 }, { s: "+", label: "render::session", tok: 2000 }],
  133: [{ s: "+", label: "watch def", tok: 1800 }, { s: "+", label: "main.ts", tok: 900 }],
  134: [{ s: "+", label: "flush-boundary", tok: 300 }],
  135: [{ s: "+", label: "surgery::fork", tok: 2100 }, { s: "+", label: "cargo result", tok: 1400 }],
  136: [{ s: "+", label: "cost gradient", tok: 300 }],
};

// the calm warm-ink ramp for the Mind's categorical coding (never a rainbow)
export function mindGroupColor(i: number, n: number): string {
  const pct = Math.round(82 - i * (62 / Math.max(1, n - 1)));
  return `color-mix(in oklab, var(--ink) ${pct}%, var(--panel))`;
}

// ── LIVE: the Forest from /api/forest (corroborated liveness × state × spark). ──
interface ForestItem {
  uuid: string;
  title: string | null;
  cwd: string;
  recency: string;
  live: boolean;
  state: ForestState;
  spark: number[];
}

function mapForestItem(it: ForestItem): ForestEntry {
  return {
    id: it.uuid.slice(0, 6),
    uuid: it.uuid,
    name: it.title || it.cwd.split("/").filter(Boolean).pop() || it.uuid.slice(0, 8),
    state: it.state,
    live: it.live,
    recency: it.recency,
    shape: it.spark,
    branches: 0,
    active: false,
  };
}

export async function loadForest(): Promise<ForestEntry[]> {
  try {
    const items: ForestItem[] = await (await fetch("/api/forest")).json();
    return items.map(mapForestItem);
  } catch {
    return [];
  }
}

// Live Forest: the daemon pushes the corroborated snapshot whenever it changes
// (process liveness, turn activity, new sessions). Returns the EventSource so the
// caller can close it.
export function watchForest(onEntries: (entries: ForestEntry[]) => void): EventSource {
  const es = new EventSource("/api/watch/forest");
  es.onmessage = (ev) => {
    try {
      const items = JSON.parse(ev.data as string) as ForestItem[];
      onEntries(items.map(mapForestItem));
    } catch {
      /* malformed frame — ignore */
    }
  };
  return es;
}
