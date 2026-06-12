/**
 * toolview.ts — Pure per-type presentation of real tool calls.
 *
 * Maps the wire Tool {kind, arg, input, output} into a typed, glanceable view
 * for the drawer: verb + tinted icon, a one-line headline, an optional
 * accessory (diff stat / match count), and a structured body — no raw JSON
 * for the common types (eigen design handoff, tool-call pass 2026-06-12).
 *
 * Honesty rule: everything here is DERIVED from fields that actually exist in
 * session_json. The prototype's status ✓/✗ and duration columns are omitted —
 * the render crate carries no exit code or timing per tool (backlog: a render
 * field could light them up). Unknown kinds and malformed inputs degrade to
 * body {kind:"raw"}, which the drawer renders with the pre-existing
 * input-JSON + output drill-down so no data is ever hidden.
 *
 * PURE: no DOM, no fetch — tested with node --test (toolview.test.ts).
 */

import type { Tool } from "./turns.ts";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type ToolType =
  | "bash"
  | "read"
  | "edit"
  | "write"
  | "grep"
  | "fetch"
  | "todo"
  | "skill"
  | "task"
  | "other";

export type ToolAccessory =
  | { kind: "stat"; add: number; del: number }
  | { kind: "count"; n: number };

export type ToolBody =
  | { kind: "command"; command: string; output?: string }
  | { kind: "readinfo"; file: string; lines: number; range?: string }
  | { kind: "diff"; lines: DiffLine[]; truncated: number }
  | { kind: "matches"; lines: string[] }
  | { kind: "todos"; items: { text: string; s: "done" | "doing" | "todo" }[] }
  | { kind: "inset"; label: string; text: string }
  | { kind: "raw" };

export interface DiffLine {
  sign: "+" | "-";
  text: string;
}

export interface ToolView {
  type: ToolType;
  /** Display verb ("Bash", "Search", …). */
  verb: string;
  /** Icon name from icons.ts. */
  icon: string;
  /** Ink hue key (--ink-<tint>). */
  tint: string;
  /** One-line truncatable summary. */
  headline: string;
  /** Render the headline in mono (commands, patterns). */
  mono: boolean;
  accessory?: ToolAccessory;
  body: ToolBody;
}

// ---------------------------------------------------------------------------
// Per-type metadata (mirrors the design's TOOL_META)
// ---------------------------------------------------------------------------

const META: Record<ToolType, { icon: string; tint: string; verb: string }> = {
  bash:  { icon: "terminal", tint: "clay",  verb: "Bash" },
  read:  { icon: "doc",      tint: "teal",  verb: "Read" },
  edit:  { icon: "pencil",   tint: "olive", verb: "Edit" },
  write: { icon: "pencil",   tint: "olive", verb: "Write" },
  grep:  { icon: "search",   tint: "ochre", verb: "Search" },
  fetch: { icon: "globe",    tint: "slate", verb: "Fetch" },
  todo:  { icon: "list",     tint: "plum",  verb: "Todo" },
  skill: { icon: "skill",    tint: "olive", verb: "Skill" },
  task:  { icon: "fork",     tint: "plum",  verb: "Task" },
  other: { icon: "bolt",     tint: "slate", verb: "Tool" },
};

/** kind (as emitted in JSONL, e.g. "Bash", "TodoWrite", "mcp__x__y") → type. */
function toolType(kind: string): ToolType {
  const k = kind.toLowerCase();
  if (k === "bash" || k === "shell") return "bash";
  if (k === "read" || k === "notebookread") return "read";
  if (k === "edit" || k === "multiedit" || k === "notebookedit") return "edit";
  if (k === "write") return "write";
  if (k === "grep" || k === "glob") return "grep";
  if (k === "webfetch" || k === "websearch" || k === "fetch") return "fetch";
  if (k === "todowrite" || k === "todoread") return "todo";
  if (k === "skill") return "skill";
  if (k === "task" || k === "agent" || k.startsWith("task")) return "task";
  return "other";
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function basename(p: string): string {
  const trimmed = p.replace(/\/+$/, "");
  const i = trimmed.lastIndexOf("/");
  return i >= 0 ? trimmed.slice(i + 1) : trimmed;
}

function lineCount(s: string): number {
  return s === "" ? 0 : s.split("\n").length;
}

/** input as a record, or null when absent/not an object. */
function rec(input: unknown): Record<string, unknown> | null {
  return input !== null && typeof input === "object" && !Array.isArray(input)
    ? (input as Record<string, unknown>)
    : null;
}

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

// ---------------------------------------------------------------------------
// miniDiff — old/new string pair → minimal +/− line list
// ---------------------------------------------------------------------------

export interface MiniDiff {
  lines: DiffLine[];
  /** Lines dropped by the cap. */
  truncated: number;
  /** Full changed-line counts (pre-cap) — these drive the +N −M stat chip. */
  add: number;
  del: number;
}

/**
 * Naive but predictable mini-diff for Edit inputs: trim the lines common to
 * both ends, then emit the remaining old lines as "-" and new lines as "+".
 * Not an LCS — an interleaved change renders as one removal block followed by
 * one addition block, which is the right reading granularity for a glance.
 * Capped at `cap` emitted lines; `truncated` reports how many were dropped.
 */
export function miniDiff(oldS: string, newS: string, cap = 24): MiniDiff {
  const a = oldS.split("\n");
  const b = newS.split("\n");
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const del = endA - start;
  const add = endB - start;
  const all: DiffLine[] = [
    ...a.slice(start, endA).map((text): DiffLine => ({ sign: "-", text })),
    ...b.slice(start, endB).map((text): DiffLine => ({ sign: "+", text })),
  ];
  return { lines: all.slice(0, cap), truncated: Math.max(0, all.length - cap), add, del };
}

// ---------------------------------------------------------------------------
// toolsSummary — collapsed-turn summary line ("Skill · Bash · Edit")
// ---------------------------------------------------------------------------

export function toolsSummary(tools: Tool[]): string {
  const seen = new Set<string>();
  const verbs: string[] = [];
  for (const t of tools) {
    const verb = META[toolType(t.kind)].verb;
    if (!seen.has(verb)) {
      seen.add(verb);
      verbs.push(verb);
    }
  }
  return verbs.join(" · ");
}

// ---------------------------------------------------------------------------
// toolView — the main mapper
// ---------------------------------------------------------------------------

export function toolView(tool: Tool): ToolView {
  const type = toolType(tool.kind);
  const meta = META[type];
  const input = rec(tool.input);

  const base: ToolView = {
    type,
    verb: meta.verb,
    icon: meta.icon,
    tint: meta.tint,
    headline: tool.arg,
    mono: false,
    body: { kind: "raw" },
  };

  switch (type) {
    case "bash": {
      const command = input ? str(input.command) : null;
      if (!command) return base;
      const view: ToolView = {
        ...base,
        headline: command.split("\n")[0]!,
        mono: true,
        body: { kind: "command", command },
      };
      if (tool.output !== undefined) {
        (view.body as { output?: string }).output = tool.output;
      }
      return view;
    }

    case "read": {
      const file = input ? str(input.file_path) : null;
      if (!file) return base;
      const offset = input ? num(input.offset) : null;
      const limit = input ? num(input.limit) : null;
      const range =
        offset !== null && limit !== null ? `lines ${offset}–${offset + limit - 1}` : undefined;
      const lines = tool.output !== undefined ? lineCount(tool.output) : 0;
      const body: ToolBody = range
        ? { kind: "readinfo", file, lines, range }
        : { kind: "readinfo", file, lines };
      return {
        ...base,
        headline: range ? `${basename(file)} · ${range}` : basename(file),
        body,
      };
    }

    case "edit": {
      const file = input ? str(input.file_path) : null;
      const oldS = input ? str(input.old_string) : null;
      const newS = input ? str(input.new_string) : null;
      if (!file || oldS === null || newS === null) {
        // MultiEdit / NotebookEdit shapes — keep the honest fallback.
        return { ...base, headline: file ? basename(file) : tool.arg };
      }
      const d = miniDiff(oldS, newS);
      return {
        ...base,
        headline: basename(file),
        accessory: { kind: "stat", add: d.add, del: d.del },
        body: { kind: "diff", lines: d.lines, truncated: d.truncated },
      };
    }

    case "write": {
      const file = input ? str(input.file_path) : null;
      const content = input ? str(input.content) : null;
      if (!file || content === null) return base;
      const d = miniDiff("", content);
      return {
        ...base,
        headline: basename(file),
        accessory: { kind: "stat", add: lineCount(content), del: 0 },
        body: { kind: "diff", lines: d.lines, truncated: d.truncated },
      };
    }

    case "grep": {
      const pattern = input ? (str(input.pattern) ?? str(input.query)) : null;
      const outLines = tool.output ? tool.output.split("\n").filter((l) => l !== "") : [];
      const view: ToolView = {
        ...base,
        headline: pattern ?? tool.arg,
        mono: true,
        body: outLines.length > 0 ? { kind: "matches", lines: outLines } : { kind: "raw" },
      };
      if (outLines.length > 0) view.accessory = { kind: "count", n: outLines.length };
      return view;
    }

    case "todo": {
      const todos = input && Array.isArray(input.todos) ? (input.todos as unknown[]) : null;
      if (!todos) return base;
      const items = todos.flatMap((t) => {
        const r = rec(t);
        const text = r ? (str(r.content) ?? str(r.subject)) : null;
        if (!text) return [];
        const status = r ? str(r.status) : null;
        const s = status === "completed" ? "done" : status === "in_progress" ? "doing" : "todo";
        return [{ text, s }] as { text: string; s: "done" | "doing" | "todo" }[];
      });
      const done = items.filter((i) => i.s === "done").length;
      return {
        ...base,
        headline: `${done}/${items.length} complete`,
        body: { kind: "todos", items },
      };
    }

    case "skill": {
      const skill = input ? str(input.skill) : null;
      const args = input ? str(input.args) : null;
      return {
        ...base,
        headline: skill ?? tool.arg,
        body: { kind: "inset", label: "skill", text: skill ? skill + (args ? ` ${args}` : "") : tool.arg },
      };
    }

    case "fetch": {
      const url = input ? (str(input.url) ?? str(input.query)) : null;
      return {
        ...base,
        headline: url ?? tool.arg,
        mono: true,
        body: { kind: "inset", label: "url", text: url ?? tool.arg },
      };
    }

    case "task": {
      const headline = input
        ? (str(input.subject) ?? str(input.description) ?? tool.arg)
        : tool.arg;
      return { ...base, headline };
    }

    case "other":
      return base;
  }
}
