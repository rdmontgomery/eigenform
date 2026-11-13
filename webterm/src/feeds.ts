// Pure logic for the rail's "Needs you" section: the shape `/api/feeds` answers, the
// order rows show in, and how an item's age reads. DOM lives in rail-feeds.ts.

export interface FeedAction {
  label: string;
}

export interface FeedItem {
  id: string;
  kind: string;
  title: string;
  detail?: string;
  link?: string;
  at?: string;
  actions: FeedAction[];
}

export interface Feed {
  name: string;
  ok: boolean;
  error?: string;
  items: FeedItem[];
}

export interface FeedsResponse {
  feeds: Feed[];
  error?: string;
}

export interface FeedRow {
  feed: string;
  item: FeedItem;
}

/** Every open item, oldest `at` first (what has waited longest on top); undated last. */
export function feedRows(feeds: Feed[]): FeedRow[] {
  const rows = feeds.flatMap((f) => (f.ok ? f.items.map((item) => ({ feed: f.name, item })) : []));
  const t = (r: FeedRow) => {
    const ms = r.item.at ? Date.parse(r.item.at) : NaN;
    return Number.isNaN(ms) ? Infinity : ms;
  };
  return rows
    .map((r, i) => ({ r, i }))
    .sort((a, b) => t(a.r) - t(b.r) || a.i - b.i)
    .map(({ r }) => r);
}

/** Feeds that answered nothing usable, for the dim "<name> unreachable" lines. */
export function downFeeds(feeds: Feed[]): Feed[] {
  return feeds.filter((f) => !f.ok);
}

/** "now", "12m", "3h", "2d" — how long an item has waited. */
export function formatAge(at: string | undefined, now: number): string {
  if (!at) return "";
  const ms = Date.parse(at);
  if (Number.isNaN(ms)) return "";
  const min = Math.floor(Math.max(0, now - ms) / 60_000);
  if (min < 1) return "now";
  if (min < 60) return `${min}m`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** The link only when it is http(s); anything else (javascript:, data:) is no link. */
export function safeLink(url: string | undefined): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    return u.protocol === "https:" || u.protocol === "http:" ? u.href : null;
  } catch {
    return null;
  }
}
