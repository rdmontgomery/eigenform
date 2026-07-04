// Wire types for the eigenform-daemon HTTP/WS API.
// Each type mirrors serde's actual output field-for-field — do not diverge.

/**
 * One row from GET /api/candidates.
 * - `path`: absolute directory path.
 * - `recent`: true when this path comes from a recent eigenform session.
 */
export interface Candidate {
  path: string;
  recent: boolean;
}

/** State taxonomy from Task 1.9; matches `SessionState::as_str()` exactly. */
export type PtyState = "working" | "waiting" | "idle" | "exited";

/**
 * One row from `GET /api/pty`.
 * - `id`: u64 serialised as a string (JS Number cannot hold u64 exactly).
 * - `cwd`: None when the pty was spawned without a working directory → null.
 * - `uuid`: None until the JSONL watcher or reconcile detects the session → null.
 * - `spawnedAt` / `lastActivity`: ISO-8601 (rfc3339).
 */
export interface PtyInfo {
  id: string;
  cwd: string | null;
  uuid: string | null;
  state: PtyState;
  spawnedAt: string;
  lastActivity: string;
}

/**
 * One row from `GET /api/forest`.
 * - `title`: None for sessions whose transcript has no first-user-turn yet → null.
 * - `cwd`: always present (PathBuf.display()).
 * - `recency`: ISO-8601 rfc3339.
 * - `state`: SessionState::as_str() — same union as PtyState but not guaranteed identical.
 * - `spark`: Vec<u32> serialised as a JSON number array.
 */
export interface ForestItem {
  uuid: string;
  title: string | null;
  cwd: string;
  recency: string;
  live: boolean;
  state: string;
  spark: number[];
  /** Present iff a Fable→Opus guardrail downgrade was detected. */
  downgrade?: { offendingTurn: string } | null;
}

/**
 * One structured observability event from `GET /api/events` and the
 * `/api/events/stream` SSE. Mirrors the daemon `events::Event` serde shape.
 * - `seq`: monotonic sequence number (1-based) — dedup + `?since=<seq>` paging.
 * - `at`: ISO-8601 UTC timestamp.
 * - `kind`: kebab-case event kind, deliberately open-ended (future branches add
 *   new kinds without a schema change), so treat it as an arbitrary string.
 * - `data`: arbitrary payload, usually carrying `ptyId` and/or session `uuid`.
 */
export interface EventRecord {
  seq: number;
  at: string;
  kind: string;
  data: Record<string, unknown>;
}
