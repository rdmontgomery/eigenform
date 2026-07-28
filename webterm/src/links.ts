/**
 * links.ts — Pure URL extraction from a session's exchanges, for the rail's
 * "Links" sidebar section (a running list of URLs dropped in chat — PR
 * links, docs, external sites referenced — scoped to the active tab).
 *
 * Deliberately scans only `user`/`assistant` prose, not tool input/output:
 * a Read of some_file.py or a Bash output full of URLs isn't "dropped in
 * chat" the way a pasted link is.
 *
 * No DOM, no xterm — safe to test with `node --test`.
 */

import type { Exchange } from "./turns.ts";

export interface LinkEntry {
  url: string;
  /** Exchange `n` where the URL was first mentioned (dedup keeps the earliest). */
  turnNumber: number;
  /** Which side said it first. */
  role: "user" | "assistant";
}

const URL_RE = /https?:\/\/[^\s<>"'`]+/g;
// Punctuation a URL can trail into from surrounding prose ("see https://x.com.")
// but a real URL practically never ends with.
const TRAILING_PUNCT = /[.,;:!?)\]}'"]$/;

/** Strip prose punctuation trailing a matched URL, without eating a balanced ")". */
function cleanUrl(raw: string): string {
  let u = raw;
  while (TRAILING_PUNCT.test(u)) {
    if (u.endsWith(")")) {
      const opens = (u.match(/\(/g) ?? []).length;
      const closes = (u.match(/\)/g) ?? []).length;
      if (closes <= opens) break; // this ")" closes a "(" inside the URL — keep it
    }
    u = u.slice(0, -1);
  }
  return u;
}

/**
 * Extract every URL mentioned in a session's chat, deduped by URL (earliest
 * mention wins), in order of first appearance.
 */
export function extractUrls(exchanges: Exchange[]): LinkEntry[] {
  const seen = new Map<string, LinkEntry>();

  function scan(text: string | undefined, n: number, role: "user" | "assistant") {
    if (!text) return;
    for (const m of text.matchAll(URL_RE)) {
      const url = cleanUrl(m[0]);
      if (url && !seen.has(url)) seen.set(url, { url, turnNumber: n, role });
    }
  }

  for (const ex of exchanges) {
    scan(ex.user, ex.n, "user");
    scan(ex.assistant, ex.n, "assistant");
  }

  return [...seen.values()];
}
