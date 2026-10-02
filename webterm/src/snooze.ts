/**
 * snooze.ts — pure helpers for tab snooze: duration parsing, preset wake times,
 * "wakes in …" formatting, and picking the due snoozes. No DOM; tested with
 * node --test. The daemon stores snoozes (GET/POST/DELETE /api/snoozes); the
 * shell closes the tab, then reopens it with a flash once it is due.
 */

import type { TabDescriptor } from "./shell-helpers.ts";

/** One stored snooze, as GET /api/snoozes returns it. Times are epoch ms. */
export interface Snooze {
  id: string;
  until: number;
  snoozedAt: number;
  tab: TabDescriptor;
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

const UNIT_MS: Record<string, number> = {
  s: 1000, sec: 1000, secs: 1000, second: 1000, seconds: 1000,
  m: MIN, min: MIN, mins: MIN, minute: MIN, minutes: MIN,
  h: HOUR, hr: HOUR, hrs: HOUR, hour: HOUR, hours: HOUR,
  d: DAY, day: DAY, days: DAY,
  w: 7 * DAY, wk: 7 * DAY, week: 7 * DAY, weeks: 7 * DAY,
};

/**
 * Parse a typed duration into milliseconds: "30m", "45 min", "2h", "1h30m",
 * "3 days", "1w". A bare number means minutes. Returns null for anything that
 * doesn't parse or comes to zero.
 */
export function parseDuration(text: string): number | null {
  const s = text.trim().toLowerCase();
  if (!s) return null;
  if (/^\d+(\.\d+)?$/.test(s)) {
    const ms = Number(s) * MIN;
    return ms > 0 ? Math.round(ms) : null;
  }
  const re = /(\d+(?:\.\d+)?)\s*([a-z]+)\s*,?\s*/y;
  let total = 0;
  let pos = 0;
  while (pos < s.length) {
    re.lastIndex = pos;
    const m = re.exec(s);
    if (!m) return null;
    const unit = UNIT_MS[m[2]!];
    if (unit === undefined) return null;
    total += Number(m[1]) * unit;
    pos = re.lastIndex;
  }
  return total > 0 ? Math.round(total) : null;
}

/** A snooze preset: a label and how to compute its wake time from now. */
export interface SnoozePreset {
  label: string;
  until: (now: number) => number;
}

/** Tomorrow at local `hour`:00. */
export function tomorrowAt(now: number, hour: number): number {
  const d = new Date(now);
  d.setDate(d.getDate() + 1);
  d.setHours(hour, 0, 0, 0);
  return d.getTime();
}

export const SNOOZE_PRESETS: SnoozePreset[] = [
  { label: "30 minutes", until: (now) => now + 30 * MIN },
  { label: "1 hour", until: (now) => now + HOUR },
  { label: "3 hours", until: (now) => now + 3 * HOUR },
  { label: "Tomorrow 9am", until: (now) => tomorrowAt(now, 9) },
  { label: "3 days", until: (now) => now + 3 * DAY },
  { label: "1 week", until: (now) => now + 7 * DAY },
];

/** "in 25m", "in 2h 10m", "in 3d 4h", or "now" once due. */
export function formatWake(until: number, now: number): string {
  const left = until - now;
  if (left <= 0) return "now";
  const mins = Math.ceil(left / MIN);
  if (mins < 60) return `in ${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) {
    const m = mins % 60;
    return m ? `in ${hours}h ${m}m` : `in ${hours}h`;
  }
  const days = Math.floor(hours / 24);
  const h = hours % 24;
  return h ? `in ${days}d ${h}h` : `in ${days}d`;
}

/** Snoozes whose wake time has arrived, soonest first. */
export function dueSnoozes(list: Snooze[], now: number): Snooze[] {
  return list.filter((s) => s.until <= now).sort((a, b) => a.until - b.until);
}

/** The descriptor fields worth persisting with a snooze (no transient seedInput). */
export function snoozableTab(desc: TabDescriptor): TabDescriptor {
  const { seedInput: _seed, ...rest } = desc;
  return rest;
}
