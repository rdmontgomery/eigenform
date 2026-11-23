// Tests for links.ts — pure URL extraction from chat exchanges.
// Run: `node --test` (native TS via --experimental-strip-types in Node 22+).
import { test } from "node:test";
import assert from "node:assert/strict";
import { extractUrls, groupLinks, linkLabel, pullRequest, shortStamp, siteOf, type LinkEntry } from "./links.ts";
import type { Exchange } from "./turns.ts";

function ex(overrides: Partial<Exchange> & { n: number }): Exchange {
  return { user: "", ...overrides };
}

test("extractUrls: finds a URL in user text", () => {
  const got = extractUrls([ex({ n: 1, user: "check https://example.com/foo please" })]);
  assert.deepEqual(got, [{ url: "https://example.com/foo", turnNumber: 1, source: "user" }]);
});

test("extractUrls: finds a URL in assistant text", () => {
  const got = extractUrls([ex({ n: 1, assistant: "see https://docs.rs/tokio for details" })]);
  assert.deepEqual(got, [{ url: "https://docs.rs/tokio", turnNumber: 1, source: "claude" }]);
});

test("extractUrls: strips trailing sentence punctuation", () => {
  const got = extractUrls([ex({ n: 1, user: "Link: https://example.com/x." })]);
  assert.equal(got[0]!.url, "https://example.com/x");
});

test("extractUrls: strips trailing comma and parenthesis (unbalanced)", () => {
  const got = extractUrls([
    ex({ n: 1, user: "see https://example.com/a, and (https://example.com/b)" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), ["https://example.com/a", "https://example.com/b"]);
});

test("extractUrls: strips markdown emphasis trailing an inline link", () => {
  const got = extractUrls([
    ex({ n: 1, assistant: "opened **[PR #7](https://github.com/o/r/pull/7)** just now" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), ["https://github.com/o/r/pull/7"]);
});

test("extractUrls: strips markdown emphasis around a bare URL", () => {
  const got = extractUrls([
    ex({ n: 1, assistant: "see *https://example.com/a* and ~~https://example.com/b~~" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), [
    "https://example.com/a",
    "https://example.com/b",
  ]);
});

test("extractUrls: keeps a balanced paren inside an emphasized inline link", () => {
  const got = extractUrls([
    ex({ n: 1, assistant: "**[wiki](https://en.wikipedia.org/wiki/Foo_(bar))**" }),
  ]);
  assert.equal(got[0]!.url, "https://en.wikipedia.org/wiki/Foo_(bar)");
});

test("extractUrls: keeps a balanced closing paren that's part of the URL", () => {
  const got = extractUrls([
    ex({ n: 1, user: "wiki: https://en.wikipedia.org/wiki/Foo_(bar)" }),
  ]);
  assert.equal(got[0]!.url, "https://en.wikipedia.org/wiki/Foo_(bar)");
});

test("extractUrls: dedupes by URL, keeping the earliest turn", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/dup" }),
    ex({ n: 2, assistant: "https://example.com/dup again" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.turnNumber, 1);
  assert.equal(got[0]!.source, "user");
});

test("extractUrls: dedupes http vs https as the same link", () => {
  const got = extractUrls([
    ex({ n: 1, user: "http://example.com/a" }),
    ex({ n: 2, assistant: "https://example.com/a" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "http://example.com/a", "keeps the first-seen variant verbatim");
  assert.equal(got[0]!.turnNumber, 1);
});

test("extractUrls: dedupes case-insensitive host", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://GitHub.com/foo/bar" }),
    ex({ n: 2, assistant: "https://github.com/foo/bar" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "https://GitHub.com/foo/bar");
});

test("extractUrls: dedupes a trailing slash on the path", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/foo" }),
    ex({ n: 2, assistant: "https://example.com/foo/" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "https://example.com/foo");
});

test("extractUrls: dedupes an explicit default port", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com:443/foo" }),
    ex({ n: 2, assistant: "https://example.com/foo" }),
  ]);
  assert.equal(got.length, 1);
  assert.equal(got[0]!.url, "https://example.com:443/foo");
});

test("extractUrls: does NOT dedupe a root path's slash (nothing to strip)", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/" }),
    ex({ n: 2, assistant: "https://example.com" }),
  ]);
  assert.equal(got.length, 1, "both normalize to the same empty/root path");
});

test("extractUrls: does NOT dedupe differing query strings", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/x?tab=a" }),
    ex({ n: 2, assistant: "https://example.com/x?tab=b" }),
  ]);
  assert.equal(got.length, 2);
});

test("extractUrls: does NOT dedupe differing fragments", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://example.com/pull/22#issue-1" }),
    ex({ n: 2, assistant: "https://example.com/pull/22#issuecomment-2" }),
  ]);
  assert.equal(got.length, 2);
});

test("extractUrls: preserves order of first appearance across exchanges", () => {
  const got = extractUrls([
    ex({ n: 1, user: "first https://a.example.com" }),
    ex({ n: 2, assistant: "then https://b.example.com" }),
    ex({ n: 3, user: "then https://c.example.com" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), [
    "https://a.example.com",
    "https://b.example.com",
    "https://c.example.com",
  ]);
});

test("extractUrls: multiple URLs in one message", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://one.example.com and https://two.example.com" }),
  ]);
  assert.deepEqual(got.map((g) => g.url), ["https://one.example.com", "https://two.example.com"]);
});

test("extractUrls: ignores tool input/output entirely", () => {
  const got = extractUrls([
    ex({
      n: 1,
      user: "no link here",
      tool: { kind: "Bash", arg: "curl", delta: "", output: "https://leaked.example.com/from-output" },
    }),
  ]);
  assert.deepEqual(got, []);
});

test("extractUrls: no exchanges → empty list", () => {
  assert.deepEqual(extractUrls([]), []);
});

test("extractUrls: every link into one PR is one entry, opening on the PR", () => {
  const got = extractUrls([
    ex({ n: 1, user: "see https://github.com/acme/app/pull/7/files" }),
    ex({ n: 2, assistant: "https://github.com/acme/app/pull/7 and https://github.com/Acme/app/pull/7#discussion_r1" }),
  ]);
  assert.deepEqual(got, [{ url: "https://github.com/acme/app/pull/7", turnNumber: 1, source: "user" }]);
});

test("extractUrls: a Slack message with and without its thread query is one entry", () => {
  const got = extractUrls([
    ex({ n: 1, user: "https://acme.slack.com/archives/C0ABC/p1793289381860279" }),
    ex({ n: 2, user: "https://acme.slack.com/archives/C0ABC/p1793289381860279?thread_ts=1793289381.860279&cid=C0ABC" }),
    ex({ n: 3, user: "https://acme.slack.com/archives/C0ABC/p1793289381860280" }),
  ]);
  assert.equal(got.length, 2);
});

test("extractUrls: no URLs present → empty list", () => {
  const got = extractUrls([ex({ n: 1, user: "no links at all" })]);
  assert.deepEqual(got, []);
});

// ---------------------------------------------------------------------------
// linkLabel — rail display label, truncated to keep the differentiating tail
// ---------------------------------------------------------------------------

test("linkLabel: short host+path fits untouched", () => {
  assert.equal(linkLabel("https://example.com/foo"), "example.com/foo");
});

test("linkLabel: bare host (root path) has no trailing slash", () => {
  assert.equal(linkLabel("https://example.com/"), "example.com");
  assert.equal(linkLabel("https://example.com"), "example.com");
});

test("linkLabel: sibling links sharing a long prefix stay distinguishable", () => {
  const labels = [22, 23, 24, 25].map((n) => linkLabel(`https://example.com/acme/app/build/runs/${n}`));
  assert.equal(new Set(labels).size, 4, `expected 4 distinct labels, got ${JSON.stringify(labels)}`);
  for (const label of labels) assert.ok(label.length <= 36, `label too long: ${label}`);
});

test("linkLabel: truncates from the front, keeping the host and whole trailing segments", () => {
  assert.equal(linkLabel("https://example.com/acme/application/build/runs/22"), "example.com…/build/runs/22");
});

test("linkLabel: without the host, shows the path alone", () => {
  assert.equal(linkLabel("https://example.com/acme/app", false), "/acme/app");
  assert.equal(linkLabel("https://example.com/", false), "/");
  assert.equal(
    linkLabel("https://example.com/acme/application/build/runs/22/attempts/3", false),
    "…/build/runs/22/attempts/3",
  );
});

test("linkLabel: names GitHub PRs, issues, files and repos", () => {
  assert.equal(linkLabel("https://github.com/acme/app/pull/22"), "app#22");
  assert.equal(linkLabel("https://github.com/acme/app/issues/4"), "app#4");
  assert.equal(linkLabel("https://github.com/acme/app/blob/main/docs/guide.md"), "app · guide.md");
  assert.equal(linkLabel("https://github.com/acme/app"), "acme/app");
});

test("linkLabel: names a Jira-style ticket by its key", () => {
  assert.equal(linkLabel("https://acme.atlassian.net/browse/APP-548"), "APP-548");
  assert.equal(linkLabel("https://acme.atlassian.net/browse/APP-548", false), "APP-548");
});

test("linkLabel: a Slack message shows when it was posted, not its id", () => {
  const when = shortStamp(new Date(1793289381 * 1000));
  const url = "https://acme.slack.com/archives/C0ABC123/p1793289381860279";
  assert.equal(linkLabel(url), `Slack message · ${when}`);
  assert.equal(linkLabel(url, false), `message · ${when}`);
  assert.equal(
    linkLabel(`${url}?thread_ts=1793280000.000100&cid=C0ABC123`, false),
    `reply · ${when}`,
  );
});

test("linkLabel: a single path segment longer than the whole budget falls back to raw tail", () => {
  const url = `https://example.com/${"x".repeat(60)}`;
  const label = linkLabel(url);
  assert.ok(label.startsWith("example.com…"));
  assert.ok(label.length <= 36);
});

test("linkLabel: query string and fragment count toward the path length", () => {
  const label = linkLabel("https://example.com/pull/22?tab=files#issuecomment-99999999");
  assert.equal(label, "example.com…es#issuecomment-99999999");
  assert.ok(label.length <= 36);
});

// ---------------------------------------------------------------------------
// Sources and grouping
// ---------------------------------------------------------------------------

test("extractUrls: a subagent's links are tagged subagent, at the launching turn", () => {
  const got = extractUrls([
    ex({ n: 1, user: "look at https://example.com/a" }),
    ex({
      n: 2,
      assistant: "delegating; see https://example.com/b",
      tool: {
        kind: "Agent",
        arg: "",
        delta: "",
        subagent: {
          agentType: "general-purpose",
          description: null,
          exchanges: [
            ex({
              n: 1,
              user: "check https://docs.example.org/x",
              assistant: "found https://example.com/a and https://github.com/acme/app/pull/12",
            }),
          ],
        },
      },
    }),
  ]);
  assert.deepEqual(got, [
    { url: "https://example.com/a", turnNumber: 1, source: "user" },
    { url: "https://example.com/b", turnNumber: 2, source: "claude" },
    { url: "https://docs.example.org/x", turnNumber: 2, source: "subagent" },
    { url: "https://github.com/acme/app/pull/12", turnNumber: 2, source: "subagent" },
  ]);
});

test("pullRequest and siteOf", () => {
  assert.equal(pullRequest("https://github.com/acme/app/pull/12"), "acme/app#12");
  assert.equal(pullRequest("https://github.com/acme/app/pull/12/files#diff-1"), "acme/app#12");
  assert.equal(pullRequest("https://github.com/acme/app/issues/12"), null);
  assert.equal(pullRequest("https://example.com/acme/app/pull/12"), null);
  assert.equal(siteOf("https://WWW.Example.com/x"), "example.com");
  assert.equal(siteOf("http://localhost:4317/s/app-dd"), "localhost");
});

const L = (url: string, source: LinkEntry["source"], n = 1): LinkEntry => ({ url, turnNumber: n, source });
const LINKS: LinkEntry[] = [
  L("https://docs.example.org/guide", "user", 1),
  L("https://github.com/acme/app/pull/12", "claude", 2),
  L("https://example.com/one", "claude", 3),
  L("https://github.com/acme/app/issues/4", "subagent", 4),
  L("https://example.com/two", "subagent", 5),
  L("https://github.com/acme/web/pull/7", "user", 6),
];
const shape = (gs: ReturnType<typeof groupLinks>) =>
  gs.map((g) => [g.label, g.entries.map((e) => e.url.replace(/^https:\/\//, ""))]);

test("groupLinks by site: pull requests first, then sites by size, newest first within", () => {
  assert.deepEqual(shape(groupLinks(LINKS, "site")), [
    ["Pull requests", ["github.com/acme/web/pull/7", "github.com/acme/app/pull/12"]],
    ["example.com", ["example.com/two", "example.com/one"]],
    ["docs.example.org", ["docs.example.org/guide"]],
    ["github.com", ["github.com/acme/app/issues/4"]],
  ]);
});

test("groupLinks by site: artifact hosts, then artifact pages, then the rest; GitHub and Slack last", () => {
  const links = [
    L("https://acme.slack.com/archives/C0ABC/p1793289381860279", "user"),
    L("https://acme.slack.com/archives/C0ABC/p1793289381860280", "user"),
    L("https://acme.slack.com/archives/C0ABC/p1793289381860281", "user"),
    L("https://github.com/acme/app/blob/main/README.md", "user"),
    L("https://github.com/acme/app/blob/main/LICENSE", "user"),
    L("https://claude.ai/artifact/abc", "claude"),
    L("https://claude.ai/artifact/def", "claude"),
    L("https://artifacts.example.com/a/one", "claude"),
    L("https://docs.example.org/guide", "user"),
    L("https://github.com/acme/app/pull/12", "claude"),
  ];
  assert.deepEqual(
    groupLinks(links, "site").map((g) => g.label),
    ["Pull requests", "artifacts.example.com", "claude.ai", "docs.example.org", "github.com", "acme.slack.com"],
  );
});

test("groupLinks by source: you, Claude, subagents, each with its PRs first", () => {
  assert.deepEqual(shape(groupLinks(LINKS, "source")), [
    ["Pasted by you", ["github.com/acme/web/pull/7", "docs.example.org/guide"]],
    ["From Claude", ["github.com/acme/app/pull/12", "example.com/one"]],
    ["From subagents", ["example.com/two", "github.com/acme/app/issues/4"]],
  ]);
});

test("groupLinks filters by every word, any case, and drops emptied groups", () => {
  assert.deepEqual(shape(groupLinks(LINKS, "site", "ACME pull")), [
    ["Pull requests", ["github.com/acme/web/pull/7", "github.com/acme/app/pull/12"]],
  ]);
  assert.deepEqual(shape(groupLinks(LINKS, "source", "example.com")), [
    ["From Claude", ["example.com/one"]],
    ["From subagents", ["example.com/two"]],
  ]);
  assert.deepEqual(groupLinks(LINKS, "site", "nothing-matches"), []);
});

test("groupLinks gives each group a stable id", () => {
  assert.deepEqual(
    groupLinks(LINKS, "site").map((g) => g.key),
    ["pr", "site:example.com", "site:docs.example.org", "site:github.com"],
  );
  assert.deepEqual(
    groupLinks(LINKS, "source").map((g) => g.key),
    ["source:user", "source:claude", "source:subagent"],
  );
});
