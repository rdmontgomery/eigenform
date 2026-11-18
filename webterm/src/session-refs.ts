/**
 * session-refs.ts — finding references to other sessions in terminal text.
 *
 * A reference is a whole word that names exactly one known session: its full uuid,
 * an 8+ hex-digit prefix of the uuid, or its peer name (case-insensitive). The rules
 * follow the daemon's `/s/<ref>` resolver: a full uuid wins outright, otherwise every
 * session named by the word or whose uuid starts with it is a hit, and only a single
 * hit makes a link. Unlike `/s/`, a prefix needs 8 digits, so ordinary short words
 * that happen to be hex never count. Ambiguous and unknown words are left alone.
 *
 * A word is a run of `[A-Za-z0-9_.-]` bounded by anything else, so a name inside a
 * path or URL (`~/work/app-dd`, `https://x/app-dd`) is not a reference: the `/`
 * joins it to its neighbours. Sentence punctuation trailing a word is dropped.
 *
 * Pure; tested in session-refs.test.ts.
 */

export interface RefTarget {
  uuid: string;
  peer?: string | undefined;
}

export interface SessionRefIndex {
  /** Resolve one word to a session uuid, or null when it names none or several. */
  resolve(word: string): string | null;
}

export interface SessionRefMatch {
  /** Offsets into the scanned string, `end` exclusive. */
  start: number;
  end: number;
  text: string;
  uuid: string;
}

const MIN_PREFIX = 8;
const UUID_PREFIX_RE = /^[0-9a-f]{8}[0-9a-f-]*$/;
const FULL_UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/** Build an index over the known sessions. Rows sharing a uuid (a live pty and its
 *  transcript) count once. */
export function buildRefIndex(targets: Iterable<RefTarget>): SessionRefIndex {
  const uuids = new Set<string>();
  const byPeer = new Map<string, Set<string>>();
  for (const t of targets) {
    const uuid = t.uuid.toLowerCase();
    uuids.add(uuid);
    if (t.peer) {
      const key = t.peer.toLowerCase();
      let set = byPeer.get(key);
      if (!set) byPeer.set(key, (set = new Set()));
      set.add(uuid);
    }
  }
  const sorted = [...uuids].sort();

  /** uuids starting with `prefix`, stopping once two are found. */
  function prefixHits(prefix: string): string[] {
    let lo = 0;
    let hi = sorted.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (sorted[mid]! < prefix) lo = mid + 1;
      else hi = mid;
    }
    const out: string[] = [];
    for (let i = lo; i < sorted.length && out.length < 2 && sorted[i]!.startsWith(prefix); i++) {
      out.push(sorted[i]!);
    }
    return out;
  }

  return {
    resolve(word) {
      const w = word.toLowerCase();
      if (FULL_UUID_RE.test(w)) return uuids.has(w) ? w : null;
      const hits = new Set(byPeer.get(w) ?? []);
      if (w.length >= MIN_PREFIX && UUID_PREFIX_RE.test(w)) {
        for (const u of prefixHits(w)) hits.add(u);
      }
      return hits.size === 1 ? [...hits][0]! : null;
    },
  };
}

const WORD_RE = /[A-Za-z0-9_.-]+/g;
const TRAILING = /[.\-_]+$/;
const LEADING = /^[.\-_]+/;
/** Characters that glue a word to its neighbours into a path, URL or address. */
const JOINERS = new Set(["/", "\\", ":", "@", "~", "=", "#", "?", "&"]);

/** Every session reference in `text`, left to right. */
export function findSessionRefs(text: string, index: SessionRefIndex): SessionRefMatch[] {
  const out: SessionRefMatch[] = [];
  for (const m of text.matchAll(WORD_RE)) {
    let start = m.index;
    let end = start + m[0].length;
    const before = text[start - 1];
    const after = text[end];
    if ((before && JOINERS.has(before)) || (after && JOINERS.has(after) && after !== ":")) continue;
    let word = m[0];
    const lead = LEADING.exec(word)?.[0].length ?? 0;
    const trail = TRAILING.exec(word)?.[0].length ?? 0;
    word = word.slice(lead, word.length - trail);
    start += lead;
    end -= trail;
    if (word.length === 0) continue;
    // "app-dd:" in prose is the name and a colon; "host:8080" is an address.
    if (after === ":" && trail === 0 && /^[0-9/]/.test(text[end + 1] ?? "")) continue;
    const uuid = index.resolve(word);
    if (uuid) out.push({ start, end, text: word, uuid });
  }
  return out;
}
