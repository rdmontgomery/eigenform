/**
 * reach.ts — Pure model of an agent's *reach* across a session.
 *
 * The transcript drawer (turns.ts / toolview.ts) answers "what did each tool
 * call do?". This module answers a different, higher-altitude question:
 * "how far did the agent's hands stretch, and in what order?".
 *
 * Every tool exchange is classified into at most one TARGET — a node the call
 * reached: a subdirectory of the session root, a sibling repo under the same
 * parent, a path elsewhere on disk, an external web host, an MCP server, or a
 * spawned subagent/skill. Targets are ringed by *distance from home* so the
 * rendered overlay reads as a spiderweb: inner = inside the workspace, outer =
 * off the machine entirely (web/MCP). Events keep their session order, so the
 * overlay can play the reach back as a time evolution.
 *
 * Two MCP surfaces get special tags because they are the classic exfiltration
 * shape: `secret` (a vault / secret-manager) and `comms` (Slack, email, …).
 * When a secret read is followed by an egress (comms or web), `exfil` flags the
 * ordered pair — "talked to a secret manager, then out through Slack" becomes
 * visible at a glance.
 *
 * PURE: no DOM, no fetch — tested with `node --test` (reach.test.ts). Everything
 * is DERIVED from fields that actually exist in session_json (kind + input).
 */

import type { Exchange, Tool } from "./turns.ts";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** What a reached node *is* — drives ring placement and color. */
export type ReachKind =
  | "workspace" // a path inside the session root subtree
  | "repo" //      a sibling directory under the root's parent (another repo/org dir)
  | "external" //  a local path outside both root and its parent
  | "web" //       an external http(s) host (WebFetch/WebSearch or a shell egress)
  | "loopback" //  a localhost / 127.x / ::1 host — on this machine, not off-box
  | "mcp" //       a generic MCP server
  | "secret" //    an MCP server / tool that looks like a secret manager or vault
  | "comms" //     an MCP server / tool that looks like a messaging / exfil surface
  | "agent" //     a spawned subagent or skill
  | "shell"; //    a Bash exec with no resolved filesystem or network target

/** What the agent *did* to a node. */
export type ReachAction = "read" | "write" | "search" | "exec" | "network" | "spawn";

/** One tool call, reduced to (when, where, what). Session-ordered. */
export interface ReachEvent {
  /** Exchange number (turn) the tool ran in — the time axis. */
  turn: number;
  /** Monotonic order across the whole session (0-based). */
  seq: number;
  /** Raw tool kind ("Read", "mcp__github__search_code", …). */
  kind: string;
  action: ReachAction;
  /** id of the ReachNode this event reached. */
  node: string;
  /** True for secret reads and comms/web egress — the exfil surfaces. */
  sensitive: boolean;
}

/** A distinct place the agent reached, aggregated across the session. */
export interface ReachNode {
  id: string;
  /** Short display label (relative dir, host, server name). */
  label: string;
  /** Fully-qualified detail for the tooltip (abs path, url, server·tool). */
  detail: string;
  kind: ReachKind;
  /** Number of events that reached this node. */
  count: number;
  /** seq of the first event that reached it (reveal order on the time axis). */
  firstSeq: number;
  firstTurn: number;
  lastTurn: number;
  /** Distinct actions, in first-seen order. */
  actions: ReachAction[];
  sensitive: boolean;
}

/** A secret-read → egress pair: the classic exfiltration shape. */
export interface ExfilFlag {
  /** secret node id. */
  from: string;
  /** comms/web node id. */
  to: string;
  fromTurn: number;
  toTurn: number;
}

export interface ReachModel {
  /** Absolute session root (provided, or inferred from file paths), or "session". */
  root: string;
  /** basename of the root, for the hub label. */
  rootLabel: string;
  /** Reached nodes (excludes the root hub itself). */
  nodes: ReachNode[];
  /** Session-ordered events. */
  events: ReachEvent[];
  /** Distinct turns that produced ≥1 event (scrubber stops). */
  turns: number[];
  /** First secret→egress pair, or null. */
  exfil: ExfilFlag | null;
}

export interface BuildReachOptions {
  /** The session's working directory. When absent it is inferred from paths. */
  root?: string;
}

// ---------------------------------------------------------------------------
// Heuristics — what reads as a secret manager / a comms (exfil) surface
// ---------------------------------------------------------------------------

const SECRET_RE =
  /vault|secret|credential|passwd|password|\bssm\b|\bkms\b|1password|onepassword|keychain|\bsops\b|\bkv\b|cyberark|doppler|infisical|hashicorp/i;
const COMMS_RE =
  /slack|discord|\bemail\b|gmail|smtp|mailgun|sendgrid|telegram|twilio|\bsms\b|webhook|teams|mattermost|pagerduty|zapier|exfil/i;

/** Tool-verb → action for MCP tools (best-effort from the tool name). */
const MCP_WRITE_RE = /create|update|write|post|send|delete|put|merge|push|comment|upload|set/i;
const MCP_READ_RE = /get|read|list|search|fetch|find|view|scan|describe|show/i;

// ---------------------------------------------------------------------------
// Small path helpers (no fs — string-only, honest about "/")
// ---------------------------------------------------------------------------

function normPath(p: string): string {
  let s = p.trim();
  if (s === "") return s;
  s = s.replace(/\/{2,}/g, "/");
  if (s.length > 1) s = s.replace(/\/+$/, "");
  return s;
}

function basename(p: string): string {
  const s = normPath(p);
  const i = s.lastIndexOf("/");
  return i >= 0 ? s.slice(i + 1) || s : s;
}

function dirname(p: string): string {
  const s = normPath(p);
  const i = s.lastIndexOf("/");
  if (i < 0) return "";
  return i === 0 ? "/" : s.slice(0, i);
}

/** `path` relative to `base` if it is `base` or under it, else null. */
function relUnder(path: string, base: string): string | null {
  if (base === "") return null;
  if (path === base) return "";
  if (path.startsWith(base + "/")) return path.slice(base.length + 1);
  return null;
}

/** Keep at most the first `n` segments of a "/"-joined relative path. */
function topSegments(rel: string, n: number): string {
  if (rel === "") return "";
  return rel.split("/").slice(0, n).join("/");
}

function hostOf(url: string): string | null {
  const m = /^[a-z][a-z0-9+.-]*:\/\/([^/\s:?#]+)/i.exec(url.trim());
  return m && m[1] ? m[1].toLowerCase() : null;
}

/** First http(s) host mentioned in a shell command, if any (egress detection). */
function egressHost(cmd: string): string | null {
  const m = /\bhttps?:\/\/([^/\s"'`)]+)/i.exec(cmd);
  return m && m[1] ? m[1].toLowerCase() : null;
}

/**
 * Is this host on the local machine (loopback)? Accepts an optional :port and
 * bracketed IPv6. Loopback reach is not off-box and is not an egress surface, so
 * hitting 127.0.0.1 after reading a secret is not exfil.
 */
function isLoopback(hostPort: string): boolean {
  let h = hostPort.trim().toLowerCase();
  const br = /^\[([^\]]+)\](?::\d+)?$/.exec(h); // [::1]:4317 → ::1
  if (br && br[1]) h = br[1];
  else if ((h.match(/:/g) || []).length === 1) h = h.replace(/:\d+$/, ""); // host:port (v4/name)
  return (
    h === "localhost" ||
    h === "0.0.0.0" ||
    h === "::1" ||
    h === "::" ||
    h.endsWith(".localhost") ||
    /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(h)
  );
}

/** A network host → a loopback node (local) or a web node (off-box). */
function hostTarget(host: string, detail: string): Target {
  if (isLoopback(host)) {
    return { id: "local:" + host, label: host, detail, kind: "loopback", action: "network", sensitive: false };
  }
  return { id: "web:" + host, label: host, detail, kind: "web", action: "network", sensitive: false };
}

// ---------------------------------------------------------------------------
// Input accessors (mirror toolview.ts's defensive shape)
// ---------------------------------------------------------------------------

function rec(input: unknown): Record<string, unknown> | null {
  return input !== null && typeof input === "object" && !Array.isArray(input)
    ? (input as Record<string, unknown>)
    : null;
}

function str(v: unknown): string | null {
  return typeof v === "string" && v !== "" ? v : null;
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

interface Target {
  id: string;
  label: string;
  detail: string;
  kind: ReachKind;
  action: ReachAction;
  sensitive: boolean;
}

interface Ctx {
  root: string;
  rootLabel: string;
}

/**
 * Classify an absolute (or relative) path into a workspace / repo / external
 * node, grouped at a directory granularity so the spiderweb stays legible:
 *   - workspace: the file's directory relative to root, capped to 2 segments
 *     (e.g. "crates/render"); root-level files collapse to the hub-adjacent "·".
 *   - repo:      the sibling top-level dir under the root's parent.
 *   - external:  the path's first two absolute segments.
 */
export function classifyPath(rawPath: string, ctx: Ctx): Omit<Target, "action" | "sensitive"> {
  const p = normPath(rawPath);

  const relRoot = relUnder(p, ctx.root);
  if (relRoot !== null) {
    const dirRel = relRoot.includes("/") ? relRoot.slice(0, relRoot.lastIndexOf("/")) : "";
    const grouped = topSegments(dirRel, 2);
    return {
      id: "ws:" + (grouped || "."),
      label: grouped || "·",
      detail: ctx.root + (grouped ? "/" + grouped : ""),
      kind: "workspace",
    };
  }

  const parent = dirname(ctx.root);
  const relParent = relUnder(p, parent);
  if (relParent !== null && relParent !== "") {
    const sib = relParent.split("/")[0] ?? relParent;
    return { id: "repo:" + sib, label: sib, detail: parent + "/" + sib, kind: "repo" };
  }

  // Elsewhere on disk (or a bare relative path we cannot anchor).
  const segs = p.split("/").filter((s) => s !== "");
  const top = segs.slice(0, 2).join("/");
  const label = p.startsWith("/") ? "/" + top : top || p;
  return { id: "ext:" + label, label, detail: p, kind: "external" };
}

/** Reduce one tool to the single node it reached, or null (no reach). */
export function targetOf(tool: Tool, ctx: Ctx): Target | null {
  const kind = tool.kind;
  const k = kind.toLowerCase();
  const input = rec(tool.input);

  // ── MCP tools — the server is the node; secret/comms get tagged. ──────────
  if (k.startsWith("mcp__")) {
    const parts = kind.split("__");
    const server = parts[1] ?? "mcp";
    const mtool = parts.slice(2).join(".");
    const probe = server + " " + mtool;
    const secret = SECRET_RE.test(probe);
    const comms = COMMS_RE.test(probe);
    const nodeKind: ReachKind = secret ? "secret" : comms ? "comms" : "mcp";
    const action: ReachAction = MCP_WRITE_RE.test(mtool)
      ? "write"
      : MCP_READ_RE.test(mtool)
        ? "read"
        : "network";
    return {
      id: "mcp:" + server,
      label: server,
      detail: "mcp · " + server + (mtool ? " · " + mtool : ""),
      kind: nodeKind,
      action,
      // A secret READ and any comms touch are the exfil surfaces we light up.
      sensitive: comms || (secret && action === "read"),
    };
  }

  // ── Filesystem tools ──────────────────────────────────────────────────────
  if (k === "read" || k === "notebookread") return pathTarget(input?.["file_path"], "read", ctx);
  if (k === "edit" || k === "multiedit" || k === "notebookedit")
    return pathTarget(input?.["file_path"], "write", ctx);
  if (k === "write") return pathTarget(input?.["file_path"], "write", ctx);
  if (k === "grep" || k === "glob")
    return pathTarget(input?.["path"] ?? ctx.root, "search", ctx);

  // ── Web ───────────────────────────────────────────────────────────────────
  if (k === "webfetch" || k === "fetch") {
    const url = str(input?.["url"]);
    const host = url ? hostOf(url) : null;
    if (host) return hostTarget(host, url ?? host);
    return { id: "web:fetch", label: "web", detail: url ?? "web fetch", kind: "web", action: "network", sensitive: false };
  }
  if (k === "websearch") {
    return { id: "web:search", label: "web search", detail: "web search", kind: "web", action: "network", sensitive: false };
  }

  // ── Bash — exec; surface an egress host when the command makes a request. ──
  if (k === "bash" || k === "shell") {
    const cmd = str(input?.["command"]) ?? "";
    const host = egressHost(cmd);
    if (host) return hostTarget(host, host + " (via shell)");
    return { id: "shell", label: "shell", detail: "bash exec", kind: "shell", action: "exec", sensitive: false };
  }

  // ── Spawned subagents / skills ─────────────────────────────────────────────
  if (k === "task" || k === "agent" || k.startsWith("task")) {
    const t = str(input?.["subagent_type"]) ?? str(input?.["description"]) ?? "subagent";
    return { id: "agent:" + t, label: t, detail: "subagent · " + t, kind: "agent", action: "spawn", sensitive: false };
  }
  if (k === "skill") {
    const s = str(input?.["skill"]) ?? "skill";
    return { id: "skill:" + s, label: s, detail: "skill · " + s, kind: "agent", action: "spawn", sensitive: false };
  }

  // TodoWrite/TodoRead, AskUserQuestion, ToolSearch, … reach nowhere.
  return null;
}

function pathTarget(v: unknown, action: ReachAction, ctx: Ctx): Target | null {
  const p = str(v);
  if (!p) return null;
  return { ...classifyPath(p, ctx), action, sensitive: false };
}

// ---------------------------------------------------------------------------
// Root inference — longest common directory of the touched file paths
// ---------------------------------------------------------------------------

export function inferRoot(paths: string[]): string {
  const abs = paths.filter((p) => p.startsWith("/")).map(normPath);
  const first = abs[0];
  if (first === undefined) return "";
  let prefix = dirname(first);
  for (let i = 1; i < abs.length; i++) {
    const p = abs[i]!;
    while (prefix !== "" && !(p === prefix || p.startsWith(prefix + "/"))) {
      const d = dirname(prefix);
      if (d === prefix) {
        prefix = "";
        break;
      }
      prefix = d;
    }
    if (prefix === "") break;
  }
  return prefix;
}

// ---------------------------------------------------------------------------
// buildReach — the one entry point
// ---------------------------------------------------------------------------

/** File paths used for root inference (only the unambiguously-path tools). */
function filePathsOf(exchanges: Exchange[]): string[] {
  const out: string[] = [];
  for (const ex of exchanges) {
    const t = ex.tool;
    if (!t) continue;
    const k = t.kind.toLowerCase();
    const input = rec(t.input);
    if (!input) continue;
    if (k === "read" || k === "edit" || k === "multiedit" || k === "write" || k === "notebookedit" || k === "notebookread") {
      const fp = str(input["file_path"]);
      if (fp && fp.startsWith("/")) out.push(fp);
    }
  }
  return out;
}

export function buildReach(exchanges: Exchange[], opts: BuildReachOptions = {}): ReachModel {
  const root = opts.root ? normPath(opts.root) : inferRoot(filePathsOf(exchanges));
  const rootLabel = root ? basename(root) : "session";
  const ctx: Ctx = { root, rootLabel };

  const nodeMap = new Map<string, ReachNode>();
  const events: ReachEvent[] = [];
  let seq = 0;

  for (const ex of exchanges) {
    if (!ex.tool) continue;
    const t = targetOf(ex.tool, ctx);
    if (!t) continue;

    const turn = ex.n;
    events.push({ turn, seq, kind: ex.tool.kind, action: t.action, node: t.id, sensitive: t.sensitive });

    const existing = nodeMap.get(t.id);
    if (existing) {
      existing.count++;
      existing.lastTurn = turn;
      if (!existing.actions.includes(t.action)) existing.actions.push(t.action);
      existing.sensitive = existing.sensitive || t.sensitive;
    } else {
      nodeMap.set(t.id, {
        id: t.id,
        label: t.label,
        detail: t.detail,
        kind: t.kind,
        count: 1,
        firstSeq: seq,
        firstTurn: turn,
        lastTurn: turn,
        actions: [t.action],
        sensitive: t.sensitive,
      });
    }
    seq++;
  }

  const nodes = [...nodeMap.values()];
  const turns = [...new Set(events.map((e) => e.turn))].sort((a, b) => a - b);
  const exfil = detectExfil(events, nodeMap);

  return { root: root || "session", rootLabel, nodes, events, turns, exfil };
}

/** First secret-read followed by any comms/web egress, in session order. */
function detectExfil(events: ReachEvent[], nodeMap: Map<string, ReachNode>): ExfilFlag | null {
  const kindOf = (id: string): ReachKind | undefined => nodeMap.get(id)?.kind;
  const secret = events.find((e) => kindOf(e.node) === "secret" && e.action === "read");
  if (!secret) return null;
  const egress = events.find(
    (e) => e.seq > secret.seq && (kindOf(e.node) === "comms" || kindOf(e.node) === "web"),
  );
  if (!egress) return null;
  return { from: secret.node, to: egress.node, fromTurn: secret.turn, toTurn: egress.turn };
}
