/**
 * fuzzy.ts — Pure fuzzy candidate scorer for the new-session launcher.
 *
 * No DOM, no fetch. Strict TS.
 *
 * ## Ranking (ordinal — not weighted; each criterion is a tiebreaker for the next)
 *
 *   1. **basenameHit** — query is a subsequence of `basename(path)` (1 = yes, 0 = no)
 *   2. **longestRun** — length of the longest contiguous run of consecutive query-character
 *      matches found anywhere in the full path (higher is better)
 *   3. **wordBoundaryStarts** — number of match starts that fall on a word-boundary character
 *      (path separator `/`, `-`, `_`, `.` or the very start of the path) (higher is better)
 *   4. **recent** — candidate.recent === true (1 = yes, 0 = no)
 *   5. **input order** — stable: equal-scored candidates preserve their original order, so
 *      the recents-first ordering from GET /api/candidates survives tie-breaking
 *
 * ## Case
 *
 * Queries are lowercased before matching; paths are matched case-insensitively.
 * This is pinned behaviour — do not change without updating tests.
 *
 * ## Exclusion
 *
 * Candidates where the query is NOT a subsequence of the full path are excluded entirely.
 *
 * ## Match positions
 *
 * The scorer exposes `matchPositions` on `ScoredCandidate` for future highlight rendering.
 * Each position is an index into the lowercased full path string. YAGNI for now — the
 * picker (Task 3.4) can ignore or use this field at its discretion.
 */

import type { Candidate } from "./types.ts";

// ---------------------------------------------------------------------------
// Word-boundary characters (separators between path segments / identifiers)
// ---------------------------------------------------------------------------

const WORD_BOUNDARY = new Set(["/", "-", "_", "."]);

// ---------------------------------------------------------------------------
// isSubsequence — does `query` appear as a subsequence in `target`?
// Both must already be lowercased.
// ---------------------------------------------------------------------------

function isSubsequence(query: string, target: string): boolean {
  let qi = 0;
  for (let ti = 0; ti < target.length && qi < query.length; ti++) {
    if (query[qi] === target[ti]) qi++;
  }
  return qi === query.length;
}

// ---------------------------------------------------------------------------
// scoreMatch — compute (basenameHit, longestRun, wordBoundaryStarts) plus
// match positions for a query against a lowercased path.
// Returns null when the query is not a subsequence of the path.
// ---------------------------------------------------------------------------

interface MatchScore {
  basenameHit: number;    // 1 if subsequence match lands in basename, else 0
  longestRun: number;     // longest contiguous matched run
  wordBoundaryStarts: number; // count of match starts at word boundaries
  matchPositions: number[]; // indices into lowercased full path
}

function scoreMatch(query: string, lowerPath: string): MatchScore | null {
  if (query.length === 0) {
    // Empty query: everything passes, all scores zero
    return { basenameHit: 0, longestRun: 0, wordBoundaryStarts: 0, matchPositions: [] };
  }

  // --- Subsequence scan using a greedy left-to-right approach.
  // We want the match that maximises the longest contiguous run, so we use a
  // simple greedy scan (take the leftmost match for each query char). This is
  // standard and produces stable results.
  const positions: number[] = [];
  let qi = 0;
  for (let ti = 0; ti < lowerPath.length && qi < query.length; ti++) {
    if (query[qi] === lowerPath[ti]) {
      positions.push(ti);
      qi++;
    }
  }
  if (qi < query.length) return null; // not a subsequence

  // --- Longest contiguous run
  let longestRun = 1;
  let currentRun = 1;
  for (let i = 1; i < positions.length; i++) {
    if (positions[i]! === positions[i - 1]! + 1) {
      currentRun++;
      if (currentRun > longestRun) longestRun = currentRun;
    } else {
      currentRun = 1;
    }
  }

  // --- Word-boundary starts
  // A "start" of a run is any position that is either:
  //   - the very first matched character, or
  //   - preceded by a gap (positions[i] !== positions[i-1]+1)
  // A boundary means: the character immediately before the matched position
  // is a WORD_BOUNDARY char, OR the position is 0.
  let wordBoundaryStarts = 0;
  for (let i = 0; i < positions.length; i++) {
    const isRunStart = i === 0 || positions[i]! !== positions[i - 1]! + 1;
    if (!isRunStart) continue;
    const pos = positions[i]!;
    if (pos === 0 || WORD_BOUNDARY.has(lowerPath[pos - 1]!)) {
      wordBoundaryStarts++;
    }
  }

  // --- Basename hit
  // basename = everything after the last "/" in the path
  const lastSlash = lowerPath.lastIndexOf("/");
  const basenameStart = lastSlash + 1; // 0 if no slash
  const basenameStr = lowerPath.slice(basenameStart);
  const basenameHit = isSubsequence(query, basenameStr) ? 1 : 0;

  return { basenameHit, longestRun, wordBoundaryStarts, matchPositions: positions };
}

// ---------------------------------------------------------------------------
// ScoredCandidate — internal; not exported (Task 3.4 gets the filtered list)
// ---------------------------------------------------------------------------

interface ScoredCandidate {
  candidate: Candidate;
  score: MatchScore;
  inputIndex: number;
}

// ---------------------------------------------------------------------------
// rankCandidates — the public API
//
// query:      the typed filter string (case-insensitive; lowercased internally)
// candidates: ordered list from GET /api/candidates (recents first)
// returns:    filtered + sorted list, same Candidate shape
// ---------------------------------------------------------------------------

export function rankCandidates(
  query: string,
  candidates: Candidate[],
): Candidate[] {
  const lowerQuery = query.toLowerCase();

  // Empty query: return all in input order (no filtering, no sorting)
  if (lowerQuery.length === 0) {
    return candidates.slice();
  }

  // Score and filter
  const scored: ScoredCandidate[] = [];
  for (let i = 0; i < candidates.length; i++) {
    const candidate = candidates[i]!;
    const lowerPath = candidate.path.toLowerCase();
    const score = scoreMatch(lowerQuery, lowerPath);
    if (score !== null) {
      scored.push({ candidate, score, inputIndex: i });
    }
  }

  // Sort — ordinal priority: basenameHit > longestRun > wordBoundaryStarts > recent > inputIndex
  // All comparisons descending (higher is better), except inputIndex (ascending = stable).
  scored.sort((a, b) => {
    const sa = a.score;
    const sb = b.score;

    if (sa.basenameHit !== sb.basenameHit) return sb.basenameHit - sa.basenameHit;
    if (sa.longestRun !== sb.longestRun) return sb.longestRun - sa.longestRun;
    if (sa.wordBoundaryStarts !== sb.wordBoundaryStarts)
      return sb.wordBoundaryStarts - sa.wordBoundaryStarts;

    const recA = a.candidate.recent ? 1 : 0;
    const recB = b.candidate.recent ? 1 : 0;
    if (recA !== recB) return recB - recA;

    // Stable: preserve input order
    return a.inputIndex - b.inputIndex;
  });

  return scored.map((s) => s.candidate);
}
