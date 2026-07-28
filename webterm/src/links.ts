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
 * Normalize a URL for dedup *comparison* only — the `LinkEntry.url` kept for
 * display/click-through is always the original, untouched string (whichever
 * variant was mentioned first). Collapses the superficial variations that
 * produce a "duplicate" link in practice:
 *   - scheme (http/https — the same resource, encrypted or not)
 *   - host case ("GitHub.com" vs "github.com")
 *   - an explicit default port (":443" on https, ":80" on http)
 *   - a single trailing slash on a non-root path
 * Query string and fragment are compared as-is: they can point at a genuinely
 * different resource or anchor (e.g. a `#issuecomment-…` deep link), so two
 * URLs differing only there are NOT treated as duplicates.
 * Falls back to the raw url if it doesn't parse (should be unreachable —
 * URL_RE only matches strings `new URL` already accepts).
 */
function normalizeForDedup(url: string): string {
  try {
    const u = new URL(url);
    const defaultPort = u.protocol === "https:" ? "443" : "80";
    const port = u.port && u.port !== defaultPort ? `:${u.port}` : "";
    const path = u.pathname.length > 1 && u.pathname.endsWith("/")
      ? u.pathname.slice(0, -1)
      : u.pathname;
    return `${u.hostname.toLowerCase()}${port}${path}${u.search}${u.hash}`;
  } catch {
    return url;
  }
}

/**
 * Extract every URL mentioned in a session's chat, deduped (earliest mention
 * wins — see `normalizeForDedup` for what counts as "the same link"), in
 * order of first appearance.
 */
export function extractUrls(exchanges: Exchange[]): LinkEntry[] {
  const seen = new Map<string, LinkEntry>();

  function scan(text: string | undefined, n: number, role: "user" | "assistant") {
    if (!text) return;
    for (const m of text.matchAll(URL_RE)) {
      const url = cleanUrl(m[0]);
      if (!url) continue;
      const key = normalizeForDedup(url);
      if (!seen.has(key)) seen.set(key, { url, turnNumber: n, role });
    }
  }

  for (const ex of exchanges) {
    scan(ex.user, ex.n, "user");
    scan(ex.assistant, ex.n, "assistant");
  }

  return [...seen.values()];
}
