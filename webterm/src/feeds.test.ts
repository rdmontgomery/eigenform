// Tests for feeds.ts — row order, age, and link safety for the "Needs you" section.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { downFeeds, feedRows, formatAge, safeLink } from "./feeds.ts";
import type { Feed, FeedItem } from "./feeds.ts";

function item(id: string, at?: string): FeedItem {
  return { id, kind: "review", title: id, actions: [], ...(at ? { at } : {}) };
}

test("feedRows: oldest first across feeds, undated last, down feeds skipped", () => {
  const feeds: Feed[] = [
    { name: "a", ok: true, items: [item("new", "2026-11-12T12:00:00Z"), item("undated")] },
    { name: "b", ok: true, items: [item("old", "2026-11-11T12:00:00Z")] },
    { name: "c", ok: false, error: "HTTP 500", items: [item("ghost", "2026-01-01T00:00:00Z")] },
  ];
  assert.deepEqual(
    feedRows(feeds).map((r) => `${r.feed}:${r.item.id}`),
    ["b:old", "a:new", "a:undated"],
  );
  assert.deepEqual(downFeeds(feeds).map((f) => f.name), ["c"]);
});

test("formatAge: minutes, hours, days, and nothing for a missing or bad time", () => {
  const now = Date.parse("2026-11-12T12:00:00Z");
  assert.equal(formatAge("2026-11-12T11:59:40Z", now), "now");
  assert.equal(formatAge("2026-11-12T11:48:00Z", now), "12m");
  assert.equal(formatAge("2026-11-12T09:00:00Z", now), "3h");
  assert.equal(formatAge("2026-11-10T11:00:00Z", now), "2d");
  assert.equal(formatAge("2026-11-12T13:00:00Z", now), "now", "a future time is now");
  assert.equal(formatAge(undefined, now), "");
  assert.equal(formatAge("soon", now), "");
});

test("safeLink: only http(s)", () => {
  assert.equal(safeLink("https://github.com/o/r/pull/1"), "https://github.com/o/r/pull/1");
  assert.equal(safeLink("http://127.0.0.1:8090/"), "http://127.0.0.1:8090/");
  assert.equal(safeLink("javascript:alert(1)"), null);
  assert.equal(safeLink("data:text/html,x"), null);
  assert.equal(safeLink("not a url"), null);
  assert.equal(safeLink(undefined), null);
});
