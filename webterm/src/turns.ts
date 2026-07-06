/**
 * turns.ts — Pure grouping logic for the transcript drawer.
 *
 * Wire type `Exchange` mirrors `eigenform_render::session_json` output exactly.
 * `groupTurns` collapses consecutive non-user exchanges between user turns
 * into a single `TurnGroup` (design §6: "a multi-message assistant turn
 * (tool-use rounds before the next user turn) collapses as one group").
 *
 * No DOM, no xterm, no imports — safe to test with `node --test`.
 */

// ---------------------------------------------------------------------------
// Wire types (mirror crates/render/src/lib.rs session_json output shape)
// ---------------------------------------------------------------------------

/** Detailed diff lines carried by some tool responses (woland ToolDetail). */
export interface ToolDetail {
  tok: number;
  lines: { t: string; c: "dim" | "add" | "rem" | "cool" }[];
}

/**
 * Tool object from session_json.
 *
 * Naming note (historical-compat):
 *   `truncated`      — output was truncated to 50 KB (pre-existing field name)
 *   `inputTruncated` — input was truncated to 50 KB (added in task 4.1)
 * Do not normalise without a simultaneous woland update.
 */
export interface Tool {
  kind: string;
  arg: string;
  delta: string;
  /** Full input params (JSON value, capped at 50 KB). */
  input?: unknown;
  /** Full result output (string, capped at 50 KB). */
  output?: string;
  /** true when `output` was truncated */
  truncated?: boolean;
  /** true when `input` was truncated */
  inputTruncated?: boolean;
  /** Preview diff lines for drill-down (present when render has detail). */
  detail?: ToolDetail;
  /**
   * A resolved async subagent transcript (Agent tool only), nested exchanges from
   * its own session_json. Absent when the tool isn't an Agent launch, or the
   * launch's agentId hasn't been resolved yet (still running / not discovered).
   */
  subagent?: {
    agentType: string | null;
    description: string | null;
    exchanges: Exchange[];
  };
}

/**
 * One exchange from session_json — ground-truth content only.
 * Fields mirror the Rust struct literally; absent fields are undefined.
 *
 * `user`  is always present (may be "" for non-user exchanges and the leaf).
 * `leaf`  marks the live end that the UI renders as an input affordance.
 * `uuid`  is the user turn's real JSONL uuid — the fork target (live sessions only).
 */
export interface Exchange {
  n: number;
  tok: number;
  user: string;
  assistant?: string;
  system?: string;
  tool?: Tool;
  leaf?: boolean;
  uuid?: string;
}

// ---------------------------------------------------------------------------
// Grouped turn type (drawer's view model)
// ---------------------------------------------------------------------------

/**
 * One narration chunk or tool call, in the exact order they occurred on the wire.
 * `toolIndex` is the item's 0-based position within the group's `toolExchanges`
 * (i.e. `toolExpandKey(group.turnNumber, item.toolIndex)` is its expansion key).
 */
export type TurnItem =
  | { kind: "text"; text: string }
  | { kind: "tool"; exchange: Exchange; toolIndex: number };

/**
 * One group in the transcript drawer:
 *   - a user turn (or the leaf)
 *   - all assistant/tool/system exchanges that followed, up to the next user turn
 *
 * `toolExchanges` preserves insertion order — Task 4.3 renders them as one-liners.
 * `assistantText` is the concatenation of all `assistant` texts in the group
 * (blank-line joined, matching session_json's append_text behaviour).
 * `items` carries the SAME content as `assistantText`/`toolExchanges` but as one
 * ordered sequence — use it to render narration next to the tool call it actually
 * preceded, instead of all narration first and all tools after.
 */
export interface TurnGroup {
  /** Exchange `n` of the opening user turn (or leaf). */
  turnNumber: number;
  /** The user's message text. "" for the leaf group. */
  userText: string;
  /** Concatenated assistant reply text (blank-line joined). "" if none. */
  assistantText: string;
  /** System timing string (e.g. "8.2s · 4 files read"). "" if none. */
  systemText: string;
  /** All tool exchanges belonging to this group, in order. */
  toolExchanges: Exchange[];
  /** Narration and tool calls, interleaved in true wire order. */
  items: TurnItem[];
  /** true when this group represents the leaf (live input affordance). */
  isLeaf: boolean;
  /** The user turn's JSONL uuid — fork target for Task 4.4. */
  uuid?: string;
}

// ---------------------------------------------------------------------------
// toolExpandKey — stable expansion key for a tool row
// ---------------------------------------------------------------------------

/**
 * Returns a stable string key for a tool row in the drawer's expansion state map.
 *
 * Key: `${turnNumber}:${toolIndex}` where
 *   - `turnNumber` is `group.turnNumber` == exchange `n` of the opening user turn.
 *     The daemon never renumbers exchanges, so this is stable for the session's
 *     lifetime. A new group appearing above shifts no existing turnNumbers.
 *   - `toolIndex` is the 0-based index within `group.toolExchanges`.
 *     Tool insertion order within a group is append-only (the daemon doesn't
 *     reorder past exchanges), so this is stable across SSE re-renders.
 *     A hypothetical mid-group insertion would silently shift expansion state
 *     for subsequent tools (currently impossible — the daemon is append-only).
 *
 * The combination is globally unique: two groups cannot share a turnNumber, and
 * two tools within the same group cannot share an index.
 */
export function toolExpandKey(turnNumber: number, toolIndex: number): string {
  return `${turnNumber}:${toolIndex}`;
}

// ---------------------------------------------------------------------------
// groupTurns — the one pure function
// ---------------------------------------------------------------------------

/**
 * Collapse a flat `exchanges` array into `TurnGroup[]`.
 *
 * Algorithm:
 *   1. Walk exchanges in order.
 *   2. A non-empty `user` field (or `leaf: true`) opens a new group.
 *   3. Subsequent assistant/tool/system exchanges accumulate into the current group.
 *   4. A second `user` exchange closes the current group and opens a new one.
 *
 * Edge cases:
 *   - Exchanges before the first user turn are discarded (rare in practice; the
 *     session_json builder handles the None arm, see render lib.rs TODO).
 *   - The leaf exchange (`leaf: true`) opens its own group, flagged `isLeaf: true`.
 *   - If the session is empty, returns [].
 */
export function groupTurns(exchanges: Exchange[]): TurnGroup[] {
  const groups: TurnGroup[] = [];
  let current: TurnGroup | null = null;

  function flush() {
    if (current !== null) groups.push(current);
    current = null;
  }

  function openGroup(ex: Exchange): TurnGroup {
    return {
      turnNumber: ex.n,
      userText: ex.user,
      assistantText: "",
      systemText: "",
      toolExchanges: [],
      items: [],
      isLeaf: ex.leaf === true,
      uuid: ex.uuid,
    };
  }

  // Fold one exchange's assistant/tool/system content into `g`, in wire order.
  // Runs for the group-opening exchange too: the Rust emitter's common shape is
  // {user: "...", assistant: "...", tool: {...}} all on ONE exchange (whatever
  // was still `exchanges.last_mut()` when each turn was processed) — treating
  // the opening exchange as a special case that skips assistant/tool handling
  // silently dropped that text/tool, so this is the single accumulation path.
  function accumulate(g: TurnGroup, ex: Exchange) {
    if (ex.assistant !== undefined && ex.assistant !== "") {
      g.assistantText = g.assistantText === ""
        ? ex.assistant
        : `${g.assistantText}\n\n${ex.assistant}`;
      g.items.push({ kind: "text", text: ex.assistant });
    }
    if (ex.tool !== undefined) {
      g.toolExchanges.push(ex);
      g.items.push({ kind: "tool", exchange: ex, toolIndex: g.toolExchanges.length - 1 });
    }
    if (ex.system !== undefined && ex.system !== "") {
      g.systemText = ex.system;
    }
  }

  for (const ex of exchanges) {
    // A real user turn (non-empty user text) or a leaf always opens a new group.
    if (ex.leaf === true || ex.user !== "") {
      flush();
      current = openGroup(ex);
      accumulate(current, ex);
      continue;
    }

    // No current group: discard orphaned assistant/tool/system exchanges.
    if (current === null) continue;

    accumulate(current, ex);
  }

  flush();
  return groups;
}
