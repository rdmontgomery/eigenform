// cooling.ts — the cache-economy model. The KV/prompt cache is ONE unit governed
// by a clock, not by position in the document: the whole prefix is warm right after
// a leaf write and COOLS over the TTL (default 5 min). Let it lapse and the next
// message re-prices the entire prefix at full input cost. The Furnace's heat IS
// this state. What varies by turn is only the STRUCTURAL fork — how many turns drop
// and, if cold when you commit, how long a prefix the new branch must re-warm.
//
// Pure + dependency-free so it can be unit-tested and later swapped for real
// telemetry without touching the view.

export const TTL_DEFAULT = 300; // prompt-cache lifetime, seconds (5 min)
export const TTL_EXTENDED = 3600; // 1-hour cache option
export const CTX_FULL = 118000; // tokens of cached prefix at the leaf
export const SEED_IDLE = 268; // start at 4:28 idle → 0:32 to cold (the dramatic open)
export const TOTAL_TURNS = 137; // the sample session runs to 137 turns

export type CacheLabel = "hot" | "warm" | "cooling" | "cold";

export interface CacheReading {
  temp: number; // 1 hot → 0 cold
  remaining: number; // seconds until cold
  cold: boolean;
  idle: number;
  ttl: number;
  reWarmFull: number; // tokens re-read in full when cold
  label: CacheLabel;
}

export interface ForkReading {
  n: number;
  drops: number; // turns discarded downstream (already spent — sunk)
  prefix: number; // re-warm size IF cold (tokens of surviving prefix)
  leaf: boolean;
  reWarm: number; // tokens actually re-warmed under the live cache state
  cold: boolean;
  temp: number;
}

// cumulative prefix tokens up to (and including) turn k
export function prefixTokensTo(k: number, total = TOTAL_TURNS): number {
  return Math.round(CTX_FULL * (k / total));
}

export function dropsAt(n: number, total = TOTAL_TURNS): number {
  return Math.max(0, total - n);
}

// the live cache state from the idle clock
export function cacheReading(idle: number, ttl = TTL_DEFAULT): CacheReading {
  const temp = Math.max(0, Math.min(1, 1 - idle / ttl)); // 1 hot → 0 cold
  const remaining = Math.max(0, ttl - idle);
  const cold = idle >= ttl;
  let label: CacheLabel;
  if (cold) label = "cold";
  else if (temp > 0.66) label = "hot";
  else if (temp > 0.33) label = "warm";
  else label = "cooling";
  return { temp, remaining, cold, idle, ttl, reWarmFull: CTX_FULL, label };
}

// the cost of forking at turn n, GIVEN the live cache state. Warm → re-process only
// the edit; cold → re-warm the surviving prefix (shorter the earlier you fork).
export function forkReading(n: number, cache: CacheReading, total = TOTAL_TURNS): ForkReading {
  const drops = dropsAt(n, total);
  const prefix = prefixTokensTo(n, total);
  const leaf = n >= total;
  const reWarm = leaf
    ? 0
    : cache.cold
      ? prefix
      : Math.round(prefix * (1 - cache.temp) * 0.18);
  return { n, drops, prefix, leaf, reWarm, cold: cache.cold, temp: cache.temp };
}

export function fmtClock(s: number): string {
  s = Math.max(0, Math.round(s));
  const m = Math.floor(s / 60);
  const ss = String(s % 60).padStart(2, "0");
  return `${m}:${ss}`;
}

export function fmtK(tok: number): string {
  if (tok >= 1000) return `${(tok / 1000).toFixed(tok >= 10000 ? 0 : 1)}k`;
  return `${tok}`;
}

// temperature → a CSS color-mix string referencing the theme's reserved poles, so it
// adapts to the active theme. Hot = amber (furnace lit, cache warm, cheap); cold =
// slate-blue (furnace out, re-warm at full price). Emitting concrete percentages (no
// calc()) keeps it portable; one string write per tick drives the whole cascade.
export function tempColor(temp: number): string {
  const p = Math.max(0, Math.min(100, Math.round(temp * 100)));
  return `color-mix(in oklab, var(--cold) ${100 - p}%, var(--amber) ${p}%)`;
}
