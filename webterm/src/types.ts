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
