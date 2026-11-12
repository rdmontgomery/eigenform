# Workers and feeds — design

**Date:** 2026-11-12 · **Status:** step 1 (the feed) being built

## The reframe

eigenform started as a control surface over Claude Code, and Codex workers already stretched that. The reason to keep going is attention. eigenform is where the operator spends the day, so anything that wants their judgment should show up there, not in a second app they stop opening once they stop building it.

The target picture has four layers, and eigenform hosts the screen for all of them:

| layer | what it does | examples |
|---|---|---|
| System 1 | cheap, local decisions and recall from one embedding and a dot product | a compiled classifier, embedding recall over the session forest |
| recontextualize | gather what's relevant and reshape it into something a model can act on | a briefing, a review request with its PR, a forest search |
| System 2 | reasoning models doing the work | Claude Code ptys, Codex workers, a salon of personas |
| durable spine | waits, schedules and approval gates that outlive a browser | an external orchestrator (for the author, Stead on Restate) |

eigenform doesn't become the spine. Durable workflows stay in whatever runs them; eigenform shows what they need and passes the operator's answer back.

## The promise, restated

The old line was "the daemon reads `~/.claude`; it never calls an API". That was a proxy for the real promise: **nothing leaves the machine**. From this design on:

- The daemon may make HTTP calls, but only to loopback addresses (`127.0.0.1`, `::1`, `localhost`). A configured URL with any other host is never fetched; it shows as a feed with an error. Those calls ignore proxy environment variables and follow no redirects, so a loopback request can't be routed off the machine.
- A local embedding server (llama.cpp, Ollama, TEI) counts as loopback when it listens there, so System 1 tools fit inside the promise.
- The pty engine rule is unchanged: interactive `claude`/`codex` only, launched from the app.

## Step 1: feeds and the "Needs you" rail section

A **feed** is a local HTTP endpoint a provider serves. eigenform reads every configured feed, lists the items in a rail section headed **Needs you**, and sends an item's actions back to its provider. eigenform knows nothing about any provider: the contract below is all there is.

### Configuration

`~/.eigenform/feeds.json`:

```json
[
  { "name": "stead", "url": "http://127.0.0.1:8090/local/feed" }
]
```

`name` is unique and `[a-z0-9-]+`. `url` must be `http://` on a loopback host. Without the file, no feeds are configured and the rail section is not shown. A file that doesn't parse, or breaks the name rule, answers `{ "feeds": [], "error": "feeds.json: …" }`, and the rail shows that line. The file is read on every `/api/feeds` request, so editing it needs no restart.

### The feed document (provider → eigenform)

`GET <url>` answers `200` with:

```json
{
  "items": [
    {
      "id": "review:helloextend/foo#12",
      "kind": "review",
      "title": "helloextend/foo#12",
      "detail": "#xad-team-pr",
      "link": "https://github.com/helloextend/foo/pull/12",
      "at": "2026-11-12T15:04:00Z",
      "actions": [
        { "label": "Start review", "post": "/api/ReviewRequests/me/launch", "body": { "key": "helloextend/foo#12" } },
        { "label": "Dismiss", "post": "/api/ReviewRequests/me/dismiss", "body": { "key": "helloextend/foo#12" } }
      ]
    }
  ]
}
```

| field | required | meaning |
|---|---|---|
| `id` | yes | stable within the provider while the item is open |
| `kind` | yes | a short label, shown as a chip |
| `title` | yes | one line |
| `detail` | no | one more line of plain text |
| `link` | no | an `http(s)` URL; clicking the title opens it in a new tab |
| `at` | no | RFC 3339, when the item started needing the operator; drives "3h ago" and the sort |
| `actions` | no | buttons, in order |
| `actions[].label` | yes | button text |
| `actions[].post` | yes | an absolute path (`/…`, not `//…`), resolved against the feed URL's origin |
| `actions[].body` | no | JSON sent as the request body; `{}` when absent |

All text is rendered with `textContent`, never as HTML. An item missing `id`, `kind` or `title` is dropped on its own, so a single bad item doesn't empty the feed. An invalid optional field (a non-http(s) `link`, an `at` that isn't RFC 3339) or an invalid action is dropped from its item, which keeps the rest; action indices count the actions that survived. Unknown fields are ignored, so providers can add fields before eigenform reads them.

### Daemon routes (browser → daemon)

- `GET /api/feeds` fetches every configured feed concurrently, with a 3 s timeout each, and answers `{ "feeds": [ { "name", "ok", "error"?, "items": [...] } ] }`. A down provider is `ok: false` with a short error and no items. It never fails the whole response. The daemon keeps the last good snapshot of each feed in memory.
- `POST /api/feeds/act` takes `{ "feed", "item", "action" }`, where `action` is the index into that item's `actions`. The daemon looks the action up in its snapshot and POSTs `body` to `post` on the feed's origin (`content-type: application/json`, 30 s timeout). It answers `{ "ok": true, "status" }`, or a 502 carrying the provider's status and the first 300 bytes of its error. An unknown feed, item or index is a 409 ("refresh and try again").

The browser never names a URL or a path. It can only trigger an action the provider advertised, so a page that reaches the daemon can't turn it into a proxy for arbitrary localhost POSTs. Like the plan gate's decision route, `/api/feeds/act` requires a local `Origin` and a JSON content type.

### The rail section

- It sits above the session list and is hidden when no feeds are configured.
- The heading carries the open-item count. Items are sorted by `at`, oldest first, because what has waited longest goes on top.
- Each row shows the kind chip, the title (a link when `link` is set), the detail, the age and the action buttons. A pressed button disables itself until the action answers, then the feed is refetched. A failure shows inline on the row.
- An unreachable feed shows one dim line ("stead unreachable") rather than disappearing, so a dead provider is visible.
- It polls every 30 s and again when the window regains focus.

### What step 1 doesn't do

- No push. A provider can't notify eigenform yet; polling is enough to learn whether the section gets used.
- No staged prompt as an action. A provider that wants a session calls `POST /api/pty` and `/api/focus` itself, as Stead does today.
- No approve/reject in the rail. Approving without reading the proposal defeats a gate, so a gate's feed item links to its own page until there's a pane worth approving from.

## Later steps (sketched)

1. **Worker kinds.** Generalize the rail's row from "Claude session, maybe Codex" to a worker kind with five operations: list, transcript, reach, launch and focus. Claude ptys and Codex threads are the first two kinds. A salon room and an orchestrator's workflow run are the next candidates, each read from a provider over the same loopback rule.
2. **A pane for a feed item.** The artifact pane already renders sandboxed HTML. A provider could serve an item's detail view there, so approving happens beside the proposal.
3. **Semantic recall over the forest.** The README's "indexed by … semantics" is still unbuilt. An embedding index (one embedding per turn, a dot product at query time, a token budget for the hand-over) fits the loopback rule with a local embedder, and gives resumption a "what did I already say about this" search.
4. **System 1 routing.** A compiled judge can answer "does this need a model, and which one" before anything spends tokens, and defer when unsure. That's the cheap front door for items arriving through feeds.
