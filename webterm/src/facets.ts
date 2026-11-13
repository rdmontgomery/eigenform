/**
 * facets.ts — the search grammar shared by the forest (rail) and the overstory
 * (tab strip): plain words match a session's title and dir as before; facet terms
 * match what the daemon's session digest extracted from the transcript.
 *
 *   pr:56   pr:app#56   pr:me/app#56   pr:app     a PR the session worked on
 *   issue:12                                     an issue it worked on
 *   #56   app#56   me/app#56                     a PR or issue, either kind
 *   repo:app                                     a repo (substring of owner/repo)
 *   branch:feat                                  a branch it ran on (substring)
 *   ABC-123   ticket:abc-123   ticket:abc        a Jira issue it worked on, or any in a project
 *
 * A bare `ABC-123` is still a plain word (a pty id like `pty-7` has the same shape):
 * it matches the title and dir as before, or a Jira issue the session worked on.
 *   ~pr:56   ~#56   ~repo:app                    …or merely mentioned in chat
 *
 * "Worked on" is the default because it is what you are usually looking for:
 * the session whose cwd is the repo, that ran `gh pr …` on the PR, opened it,
 * or called a GitHub tool on it. A leading `~` widens a term to sessions that
 * only talked about it (pasted the URL, wrote `owner/repo#N`). All terms AND.
 *
 * No DOM — safe to test with `node --test`.
 */

/** `GET /api/forest` facets: `kind:value` strings (see crates/forest/src/digest.rs). */
export interface Facets {
  work: string[];
  mention: string[];
}

export type FacetTerm =
  | { kind: "pr" | "issue" | "num"; repo?: string; num?: number; loose: boolean }
  | { kind: "repo" | "branch"; text: string; loose: boolean }
  | { kind: "ticket"; id: string; loose: boolean };

/** `ABC-123` shapes that are standards and encodings, not issues (as in digest.rs). */
const NOT_PROJECTS = new Set([
  "aes", "ansi", "cuda", "cve", "ecma", "gpt", "http", "ieee", "ipv", "iso", "pep", "rfc", "rsa",
  "sha", "ssl", "tls", "utf", "wcag",
]);
const TICKET = /^([a-z][a-z0-9]{1,9})-\d{1,7}$/;

export interface ParsedQuery {
  /** Plain words, lowercased: each must appear in the item's text. */
  words: string[];
  facets: FacetTerm[];
}

/** `app#56` / `#56` / `56` / `app` → repo part and number. */
function refParts(v: string): { repo?: string; num?: number } | null {
  const m = /^(?:([\w.\-/]+)#)?(\d+)$/.exec(v);
  if (m) return m[1] ? { repo: m[1], num: Number(m[2]) } : { num: Number(m[2]) };
  return /^[\w.\-/]+$/.test(v) ? { repo: v } : null;
}

/** Split a query into plain words and facet terms. Anything that doesn't parse as
 *  a facet (an unknown `x:y`, a URL, `~` alone) is a plain word. */
export function parseQuery(query: string): ParsedQuery {
  const words: string[] = [];
  const facets: FacetTerm[] = [];
  for (const raw of query.toLowerCase().split(/\s+/).filter(Boolean)) {
    const loose = raw.startsWith("~");
    const t = loose ? raw.slice(1) : raw;
    const kv = /^(pr|issue|repo|branch|ticket|jira):(.+)$/.exec(t);
    if (kv) {
      const [, kind, value] = kv as unknown as [string, string, string];
      if (kind === "ticket" || kind === "jira") {
        if (TICKET.test(value) || /^[a-z][a-z0-9]{1,9}$/.test(value)) {
          facets.push({ kind: "ticket", id: value, loose });
        } else {
          words.push(raw);
        }
        continue;
      }
      if (kind === "repo" || kind === "branch") {
        facets.push({ kind, text: value, loose });
        continue;
      }
      const parts = refParts(value.replace(/^#/, ""));
      if (parts) {
        facets.push({ kind: kind as "pr" | "issue", ...parts, loose });
        continue;
      }
    } else if (loose && isTicket(t)) {
      facets.push({ kind: "ticket", id: t, loose });
      continue;
    } else if (/^[\w.\-/]*#\d+$/.test(t)) {
      facets.push({ kind: "num", ...refParts(t.replace(/^#/, ""))!, loose });
      continue;
    }
    words.push(raw);
  }
  return { words, facets };
}

function isTicket(word: string): boolean {
  const m = TICKET.exec(word);
  return m !== null && !NOT_PROJECTS.has(m[1]!);
}

/** Does a plain word match? It is in the item's text (lowercased), or it is a Jira key
 *  the session worked on. */
export function wordMatches(word: string, hay: string, facets: Facets | undefined): boolean {
  if (hay.includes(word)) return true;
  return isTicket(word) && (facets?.work.some((f) => f.toLowerCase() === `ticket:${word}`) ?? false);
}

/** Does one extracted facet (`kind:value`) satisfy one term? */
function facetMatches(term: FacetTerm, facet: string): boolean {
  const i = facet.indexOf(":");
  const kind = facet.slice(0, i);
  const value = facet.slice(i + 1);
  if (term.kind === "ticket") {
    if (kind !== "ticket") return false;
    const v = value.toLowerCase();
    return term.id.includes("-") ? v === term.id : v.startsWith(`${term.id}-`);
  }
  if ("text" in term) {
    const v = term.kind === "branch" ? value.toLowerCase() : value;
    return kind === term.kind && v.includes(term.text);
  }
  // pr / issue / num against `pr:o/r#N`, `issue:o/r#N`, `ref:o/r#N` (a bare #N of
  // unknown kind could be either, so it satisfies both).
  const okKind =
    term.kind === "num"
      ? kind === "pr" || kind === "issue" || kind === "ref"
      : kind === term.kind || kind === "ref";
  if (!okKind) return false;
  const hash = value.lastIndexOf("#");
  const repo = value.slice(0, hash);
  const num = Number(value.slice(hash + 1));
  if (term.num !== undefined && term.num !== num) return false;
  if (term.repo !== undefined && !repo.includes(term.repo)) return false;
  return true;
}

/** True when every facet term is satisfied by the item's facets. No terms → true;
 *  terms but no facets (a Codex thread, a session not yet on disk) → false. */
export function matchesFacets(terms: readonly FacetTerm[], facets: Facets | undefined): boolean {
  return terms.every((t) => {
    if (!facets) return false;
    if (facets.work.some((f) => facetMatches(t, f))) return true;
    return t.loose && facets.mention.some((f) => facetMatches(t, f));
  });
}

/** Tooltip for both search boxes. */
export const FACET_HELP =
  "Words match the title and dir.\n" +
  "pr:56 · pr:app#56 · issue:12 · #56 · repo:app · branch:feat · ABC-123 · ticket:abc — sessions that worked on it\n" +
  "~pr:56 · ~repo:app · ~ABC-123 — …or only mentioned it";
