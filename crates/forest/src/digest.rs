//! The session digest: everything the forest needs from a full transcript read, built in
//! one incremental pass and persisted to `state_dir/<session_id>.json`.
//!
//! Two products share the pass:
//!
//! - the activity **spark** (`output_tokens` per completed turn), and
//! - the **facets**: the GitHub repos, PRs, issues and git branches a session touched,
//!   split by how it touched them. `work` is what the session acted on: the repo its
//!   cwd belongs to, the branch it ran on, GitHub tool calls (`owner`/`repo`/
//!   `pullNumber` inputs), `gh pr …`/`gh issue …` commands, the PR URL a create printed,
//!   the repo a `git push` went to. `mention` is what was only talked about: GitHub URLs
//!   and `owner/repo#N` refs in user/assistant prose, and bare `#N` resolved against the
//!   session's own repo. A ref that is also `work` is not repeated under `mention`.
//!
//! Jira-style issue keys (`ABC-123`) are facets too. `work`: a key in the branch the
//! session ran on, in a `git commit`/`checkout`/`switch`/`worktree`/`push` or
//! `gh pr create`/`edit` command, the issue a Jira tool was called on (an
//! `issueIdOrKey`-style input), or the key a create tool returned. `mention`: a key in
//! prose, URLs included.
//!
//! Facets are strings `kind:value` with kind one of `repo` (`owner/repo`), `pr` / `issue`
//! (`owner/repo#N`), `ref` (`owner/repo#N` of unknown kind: a bare `#N`), `branch` and
//! `ticket` (`ABC-123`, as written: keys are upper case).
//! Repo keys are lowercased: GitHub is case-insensitive, and so is the search.
//!
//! JSONL transcripts are append-only, so the persisted state carries the byte offset it
//! has read to and a refresh parses only the new tail. A file that shrank was rewritten:
//! it is re-read from the start. Unchanged (mtime, len) is served without opening it.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Bump when the persisted shape or the extraction rules change: older state is rebuilt.
const VERSION: u64 = 3;
/// Most facets kept in state per side; bounds a pathological transcript.
const STATE_CAP: usize = 256;
/// Most facets reported per side (what travels in every forest push).
pub const FACET_CAP: usize = 64;
/// Most unanswered tool calls remembered across refreshes.
const PENDING_CAP: usize = 64;

/// What a session touched, by provenance. See the module docs for the vocabulary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facets {
    pub work: Vec<String>,
    pub mention: Vec<String>,
}

/// The spark and facets of one transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Digest {
    pub spark: Vec<u32>,
    pub facets: Facets,
}

/// How to read the result of a tool call whose output names what it acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Harvest {
    /// Shell output: GitHub URLs standing alone on a line (`gh pr create` prints the PR
    /// URL; `gh pr view` an `url:` line; `git push` `To <url>` and `remote:` lines).
    Lines,
    /// A structured result (a GitHub MCP create): the first PR/issue URL in it.
    First,
    /// A Jira create's result: the first issue key in it.
    Ticket,
}

impl Harvest {
    fn as_str(self) -> &'static str {
        match self {
            Harvest::Lines => "lines",
            Harvest::First => "first",
            Harvest::Ticket => "ticket",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "lines" => Some(Harvest::Lines),
            "first" => Some(Harvest::First),
            "ticket" => Some(Harvest::Ticket),
            _ => None,
        }
    }
}

/// The incremental parse state, persisted between refreshes.
#[derive(Debug, Clone, Default)]
struct State {
    offset: u64,
    spark: Vec<u32>,
    acc: u32,
    work: Vec<String>,
    mention: Vec<String>,
    /// cwd → the repo it belongs to (None: not a repo), resolved once per cwd.
    cwd_repos: BTreeMap<String, Option<String>>,
    /// The repo of the most recent line's cwd: what a bare `#N` or `gh pr view 5` means.
    cur_repo: Option<String>,
    /// Tool calls awaiting a result that names what they acted on.
    pending: Vec<(String, Harvest)>,
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if list.len() < STATE_CAP && !list.contains(&item) {
        list.push(item);
    }
}

impl State {
    fn work(&mut self, f: String) {
        push_unique(&mut self.work, f);
    }
    fn mention(&mut self, f: String) {
        push_unique(&mut self.mention, f);
    }

    fn gh(&mut self, r: &GhRef, work: bool) {
        let mut add = |f: String| {
            if work {
                self.work(f)
            } else {
                self.mention(f)
            }
        };
        add(format!("repo:{}", r.repo));
        if let Some((kind, n)) = r.num {
            add(format!("{kind}:{}#{n}", r.repo));
        }
    }

    fn enter_cwd(&mut self, cwd: &str) {
        if !self.cwd_repos.contains_key(cwd) {
            if self.cwd_repos.len() >= STATE_CAP {
                return;
            }
            self.cwd_repos
                .insert(cwd.to_string(), repo_of_dir(Path::new(cwd)));
        }
        self.cur_repo = self.cwd_repos.get(cwd).cloned().flatten();
        if let Some(r) = self.cur_repo.clone() {
            self.work(format!("repo:{r}"));
        }
    }

    fn feed(&mut self, v: &Value) {
        if let Some(cwd) = v.get("cwd").and_then(Value::as_str) {
            self.enter_cwd(cwd);
        }
        if let Some(b) = v.get("gitBranch").and_then(Value::as_str) {
            if !b.is_empty() && b != "HEAD" {
                self.work(format!("branch:{b}"));
                for k in jira_keys(b) {
                    self.work(format!("ticket:{k}"));
                }
            }
        }
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let usage = v
                    .get("message")
                    .and_then(|m| m.get("usage"))
                    .or_else(|| v.get("usage"));
                if let Some(out) = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(Value::as_u64)
                {
                    self.acc = self.acc.saturating_add(out as u32);
                }
                for block in content_blocks(v) {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(Value::as_str) {
                                self.prose(t);
                            }
                        }
                        Some("tool_use") => self.tool_use(block),
                        _ => {}
                    }
                }
            }
            Some("system") if v.get("subtype").and_then(Value::as_str) == Some("turn_duration") => {
                self.spark.push(self.acc);
                self.acc = 0;
            }
            Some("user") => {
                let meta = v.get("isMeta").and_then(Value::as_bool) == Some(true);
                let content = v.get("message").and_then(|m| m.get("content"));
                if let Some(s) = content.and_then(Value::as_str) {
                    if !meta {
                        self.prose(s);
                    }
                }
                for block in content_blocks(v) {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") if !meta => {
                            if let Some(t) = block.get("text").and_then(Value::as_str) {
                                self.prose(t);
                            }
                        }
                        Some("tool_result") => self.tool_result(block),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Chat prose: everything it names is a mention.
    fn prose(&mut self, text: &str) {
        for k in jira_keys(text) {
            self.mention(format!("ticket:{k}"));
        }
        for r in github_refs(text) {
            self.gh(&r, false);
        }
        for (repo, n) in text_refs(text) {
            match repo {
                Some(repo) => self.mention(format!("ref:{repo}#{n}")),
                None => {
                    if let Some(repo) = self.cur_repo.clone() {
                        self.mention(format!("ref:{repo}#{n}"));
                    }
                }
            }
        }
    }

    fn tool_use(&mut self, block: &Value) {
        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
        let id = block.get("id").and_then(Value::as_str).unwrap_or("");
        let input = block.get("input");
        let lname = name.to_ascii_lowercase();
        let jira = lname.contains("jira") || lname.contains("atlassian");

        // A tool addressed at a GitHub repo (the GitHub MCP server, PR subscriptions…).
        let owner = input.and_then(|i| i.get("owner")).and_then(Value::as_str);
        let repo = input.and_then(|i| i.get("repo")).and_then(Value::as_str);
        if let (Some(owner), Some(repo)) = (owner, repo) {
            let repo = format!("{owner}/{repo}").to_ascii_lowercase();
            self.work(format!("repo:{repo}"));
            let num = |keys: &[&str]| {
                keys.iter().find_map(|k| {
                    let x = input?.get(*k)?;
                    x.as_u64().or_else(|| x.as_str()?.parse().ok())
                })
            };
            if let Some(n) = num(&["pullNumber", "pull_number", "prNumber", "pr_number"]) {
                self.work(format!("pr:{repo}#{n}"));
            } else if let Some(n) = num(&["issue_number", "issueNumber"]) {
                self.work(format!("issue:{repo}#{n}"));
            }
            if lname.contains("create_pull_request") {
                if let Some(head) = input.and_then(|i| i.get("head")).and_then(Value::as_str) {
                    self.work(format!("branch:{head}"));
                }
            }
        }
        let creates =
            lname.contains("create") && (lname.contains("pull") || lname.contains("issue"));
        if (creates || lname.contains("issue_write")) && !jira && !id.is_empty() {
            self.await_result(id, Harvest::First);
        }

        // A Jira tool: the issue it acted on, or (a create) the one it returns.
        for k in ["issueIdOrKey", "issueKey", "issue_key", "issueIdOrKeyOrUrl"] {
            if let Some(v) = input.and_then(|i| i.get(k)).and_then(Value::as_str) {
                for key in jira_keys(v) {
                    self.work(format!("ticket:{key}"));
                }
            }
        }
        if jira && lname.contains("create") && !id.is_empty() {
            self.await_result(id, Harvest::Ticket);
        }

        // A fetched URL is talked about, not acted on.
        if let Some(url) = input.and_then(|i| i.get("url")).and_then(Value::as_str) {
            for r in github_refs(url) {
                self.gh(&r, false);
            }
        }

        if name == "Bash" {
            if let Some(cmd) = input.and_then(|i| i.get("command")).and_then(Value::as_str) {
                self.bash(cmd, id);
            }
        }
    }

    fn bash(&mut self, cmd: &str, id: &str) {
        let mut harvest = false;
        for seg in shell_segments(cmd) {
            if names_work(&seg) {
                for k in jira_keys(&seg.join(" ")) {
                    self.work(format!("ticket:{k}"));
                }
            }
            if let Some(call) = gh_call(&seg) {
                let repo = call.repo.clone().or_else(|| self.cur_repo.clone());
                if let Some(repo) = &repo {
                    self.work(format!("repo:{repo}"));
                }
                if let Some(r) = &call.url {
                    self.gh(r, true);
                } else if let (Some(n), Some(repo)) = (call.num, &repo) {
                    self.work(format!("{}:{repo}#{n}", call.noun));
                }
                // No number given: the output names the PR (create prints it; view and
                // friends act on the current branch's PR and print its url).
                if call.num.is_none() && call.url.is_none() && call.verb != "list" {
                    harvest = true;
                }
            } else if is_git_push(&seg) {
                if let Some(repo) = self.cur_repo.clone() {
                    self.work(format!("repo:{repo}"));
                }
                harvest = true;
            }
        }
        if harvest && !id.is_empty() {
            self.await_result(id, Harvest::Lines);
        }
    }

    fn await_result(&mut self, id: &str, how: Harvest) {
        if self.pending.len() >= PENDING_CAP {
            self.pending.remove(0);
        }
        self.pending.push((id.to_string(), how));
    }

    fn tool_result(&mut self, block: &Value) {
        let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
            return;
        };
        let Some(i) = self.pending.iter().position(|(p, _)| p == id) else {
            return;
        };
        let (_, how) = self.pending.remove(i);
        let text = result_text(block.get("content"));
        match how {
            Harvest::Lines => {
                for line in text.lines() {
                    let mut l = line.trim();
                    for prefix in ["remote:", "url:", "To "] {
                        if let Some(rest) = l.strip_prefix(prefix) {
                            l = rest.trim();
                        }
                    }
                    if l.starts_with("http") && !l.contains(char::is_whitespace) {
                        for r in github_refs(l) {
                            self.gh(&r, true);
                        }
                    }
                }
            }
            Harvest::First => {
                if let Some(r) = github_refs(&text).into_iter().find(|r| r.num.is_some()) {
                    self.gh(&r, true);
                }
            }
            Harvest::Ticket => {
                if let Some(k) = jira_keys(&text).into_iter().next() {
                    self.work(format!("ticket:{k}"));
                }
            }
        }
    }

    fn facets(&self) -> Facets {
        let key = |f: &str| -> Option<String> {
            let (kind, rest) = f.split_once(':')?;
            matches!(kind, "pr" | "issue" | "ref").then(|| rest.to_string())
        };
        let worked: HashSet<String> = self.work.iter().filter_map(|f| key(f)).collect();
        // A ref of known kind beats a bare one for the same number.
        let typed: HashSet<String> = self
            .mention
            .iter()
            .filter(|f| !f.starts_with("ref:"))
            .filter_map(|f| key(f))
            .collect();
        let mention = self
            .mention
            .iter()
            .filter(|f| !self.work.contains(f))
            .filter(|f| match key(f) {
                Some(k) => !(worked.contains(&k) || f.starts_with("ref:") && typed.contains(&k)),
                None => true,
            })
            .take(FACET_CAP)
            .cloned()
            .collect();
        Facets {
            work: self.work.iter().take(FACET_CAP).cloned().collect(),
            mention,
        }
    }

    fn digest(&self) -> Digest {
        Digest {
            spark: self.spark.clone(),
            facets: self.facets(),
        }
    }

    /// Parse whatever the file holds past `offset`. A shrunken file starts over.
    fn advance(&mut self, path: &Path) {
        let Ok(mut f) = fs::File::open(path) else {
            return;
        };
        let Ok(len) = f.metadata().map(|m| m.len()) else {
            return;
        };
        if len < self.offset {
            *self = State::default();
        }
        if len == self.offset || f.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        if f.take(len - self.offset).read_to_end(&mut buf).is_err() {
            return;
        }
        let mut consumed = 0usize;
        for line in buf.split_inclusive(|&b| b == b'\n') {
            let terminated = line.last() == Some(&b'\n');
            let body = line.strip_suffix(b"\n").unwrap_or(line);
            match serde_json::from_slice::<Value>(body) {
                Ok(v) => self.feed(&v),
                // An unterminated tail that doesn't parse is a write in progress: leave
                // it for the next refresh. (A truncated object is never valid JSON.)
                Err(_) if !terminated => break,
                Err(_) => {}
            }
            consumed += line.len();
        }
        self.offset += consumed as u64;
    }

    fn to_json(&self, stamp: (i64, u64)) -> Value {
        let total: u64 = self.spark.iter().map(|&x| x as u64).sum();
        serde_json::json!({
            "v": VERSION,
            "source_mtime": stamp.0,
            "source_len": stamp.1,
            "offset": self.offset,
            "spark": self.spark,
            "total": total,
            "acc": self.acc,
            "work": self.work,
            "mention": self.mention,
            "cwd_repos": self.cwd_repos,
            "cur_repo": self.cur_repo,
            "pending": self.pending.iter().map(|(id, h)| [id.as_str(), h.as_str()]).collect::<Vec<_>>(),
        })
    }

    fn from_json(v: &Value) -> Option<State> {
        if v.get("v").and_then(Value::as_u64) != Some(VERSION) {
            return None;
        }
        let strings = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(State {
            offset: v.get("offset")?.as_u64()?,
            spark: v
                .get("spark")?
                .as_array()?
                .iter()
                .filter_map(|x| x.as_u64().map(|n| n as u32))
                .collect(),
            acc: v.get("acc").and_then(Value::as_u64).unwrap_or(0) as u32,
            work: strings("work"),
            mention: strings("mention"),
            cwd_repos: v
                .get("cwd_repos")
                .and_then(Value::as_object)
                .map(|o| {
                    o.iter()
                        .map(|(k, r)| (k.clone(), r.as_str().map(String::from)))
                        .collect()
                })
                .unwrap_or_default(),
            cur_repo: v.get("cur_repo").and_then(Value::as_str).map(String::from),
            pending: v
                .get("pending")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|p| {
                            let id = p.get(0)?.as_str()?;
                            Some((id.to_string(), Harvest::parse(p.get(1)?.as_str()?)?))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

fn content_blocks(v: &Value) -> impl Iterator<Item = &Value> {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// A tool_result's text: a string, or the text of its blocks.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Digest a whole transcript, uncached.
pub fn session_digest(jsonl_path: &Path) -> Digest {
    let mut s = State::default();
    s.advance(jsonl_path);
    s.digest()
}

/// [`session_digest`] cached in `state_dir/<session_id>.json`. An unchanged (mtime, len)
/// is served from the cache without opening the transcript; a grown one parses only the
/// appended lines. The state survives restarts and is shared with the CLI.
pub fn cached_digest(state_dir: &Path, session_id: &str, jsonl_path: &Path) -> Digest {
    let stamp = crate::mtime_millis(jsonl_path);
    let state_path = state_dir.join(format!("{session_id}.json"));
    let stored = fs::read_to_string(&state_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let fresh = |v: &Value| {
        stamp.is_some_and(|(mtime, len)| {
            v.get("source_mtime").and_then(Value::as_i64) == Some(mtime)
                && v.get("source_len").and_then(Value::as_u64) == Some(len)
        })
    };

    let mut state = stored.as_ref().and_then(State::from_json);
    if let (Some(s), Some(v)) = (&state, &stored) {
        if fresh(v) {
            return s.digest();
        }
    }
    let s = state.get_or_insert_with(State::default);
    s.advance(jsonl_path);
    if let Some(stamp) = stamp {
        let _ = fs::create_dir_all(state_dir);
        write_atomic(&state_path, &s.to_json(stamp).to_string());
    }
    s.digest()
}

/// Write via a sibling temp file and rename, so the daemon and the CLI never read each
/// other's half-written state (a torn read would only cost a rebuild, but still).
fn write_atomic(path: &Path, body: &str) {
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    if fs::write(&tmp, body).is_ok() && fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

// ---------------------------------------------------------------------------
// GitHub refs in text
// ---------------------------------------------------------------------------

/// A GitHub repo, optionally narrowed to one PR or issue.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GhRef {
    /// `owner/repo`, lowercased.
    repo: String,
    /// `("pr" | "issue", number)`.
    num: Option<(&'static str, u64)>,
}

/// First path segments of github.com that are site pages, not owners.
const RESERVED_OWNERS: &[&str] = &[
    "about",
    "account",
    "apps",
    "codespaces",
    "collections",
    "contact",
    "customer-stories",
    "enterprise",
    "explore",
    "features",
    "issues",
    "login",
    "marketplace",
    "new",
    "notifications",
    "orgs",
    "pricing",
    "pulls",
    "readme",
    "search",
    "security",
    "settings",
    "site",
    "sponsors",
    "topics",
    "trending",
    "users",
];

fn is_owner_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}
fn is_repo_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')
}

/// Every `https://github.com/<owner>/<repo>[/pull|issues/<n>]` in `text`. Other hosts
/// (api., gist., raw.) are not repo pages and are skipped.
fn github_refs(text: &str) -> Vec<GhRef> {
    const HOST: &str = "github.com/";
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(HOST) {
        let at = from + pos;
        from = at + HOST.len();
        // The host proper: after a scheme, `www.`, or a word boundary (a scheme-less
        // `github.com/o/r` in prose), never a subdomain like `api.` or `gist.`.
        let pre = &lower[..at];
        let pre = pre.strip_suffix("www.").unwrap_or(pre);
        let boundary = pre.ends_with("://")
            || pre.bytes().last().is_none_or(|b| {
                !(b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'/' | b'@'))
            });
        if !boundary {
            continue;
        }
        if let Some(r) = parse_repo_path(&lower[from..]) {
            out.push(r);
        }
    }
    out
}

/// `<owner>/<repo>[/pull|pulls|issues/<n>]…` at the start of `path` (already lowercase).
fn parse_repo_path(path: &str) -> Option<GhRef> {
    let b = path.as_bytes();
    let owner_end = b.iter().position(|&c| !is_owner_byte(c)).unwrap_or(b.len());
    if owner_end == 0 || b.get(owner_end) != Some(&b'/') {
        return None;
    }
    let owner = &path[..owner_end];
    if RESERVED_OWNERS.contains(&owner) {
        return None;
    }
    let rest = &path[owner_end + 1..];
    let rb = rest.as_bytes();
    let repo_end = rb
        .iter()
        .position(|&c| !is_repo_byte(c))
        .unwrap_or(rb.len());
    let mut repo = &rest[..repo_end];
    repo = repo.strip_suffix(".git").unwrap_or(repo);
    repo = repo.trim_end_matches('.');
    if repo.is_empty() {
        return None;
    }
    let repo_key = format!("{owner}/{repo}");
    let tail = &rest[repo_end..];
    let num = tail.strip_prefix('/').and_then(|t| {
        let (kind, after) = t.split_once('/')?;
        let kind = match kind {
            "pull" | "pulls" => "pr",
            "issues" => "issue",
            _ => return None,
        };
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        (digits > 0).then(|| after[..digits].parse().ok().map(|n| (kind, n)))?
    });
    Some(GhRef {
        repo: repo_key,
        num,
    })
}

/// `owner/repo#N` and bare `#N` refs in prose, outside URLs. Bare refs are taken only
/// outside code spans (`#333` in a stylesheet is not issue 333); a qualified ref reads the
/// same in backticks, which is how it is usually written.
fn text_refs(text: &str) -> Vec<(Option<String>, u64)> {
    let mut out = Vec::new();
    for (src, bare) in [(text.to_string(), false), (strip_code(text), true)] {
        for tok in src.split_whitespace() {
            if tok.contains("://") {
                continue;
            }
            let tok = tok.trim_matches(|c: char| "()[]{}<>,.;:!?\"'*_~`".contains(c));
            let Some((repo, num)) = tok.split_once('#') else {
                continue;
            };
            if num.is_empty() || num.len() > 6 || !num.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(n) = num.parse::<u64>() else { continue };
            if repo.is_empty() {
                if bare {
                    out.push((None, n));
                }
            } else if !bare {
                let Some((o, r)) = repo.split_once('/') else {
                    continue;
                };
                let ok = !o.is_empty()
                    && !r.is_empty()
                    && o.bytes().all(is_owner_byte)
                    && r.bytes().all(is_repo_byte);
                if ok {
                    out.push((Some(repo.to_ascii_lowercase()), n));
                }
            }
        }
    }
    out
}

/// Upper-case prefixes of `ABC-123` shapes that are standards and encodings, not issues.
const NOT_PROJECTS: &[&str] = &[
    "AES", "ANSI", "CUDA", "CVE", "ECMA", "GPT", "HTTP", "IEEE", "IPV", "ISO", "PEP", "RFC", "RSA",
    "SHA", "SSL", "TLS", "UTF", "WCAG",
];

/// Jira-style keys (`ABC-123`: an upper-case project of 2–10 letters or digits starting
/// with a letter, a dash, a number) standing as their own token, deduplicated in order.
fn jira_keys(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let starts = b[i].is_ascii_uppercase() && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        if !starts {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < b.len() && (b[j].is_ascii_uppercase() || b[j].is_ascii_digit()) {
            j += 1;
        }
        let project = &text[i..j];
        let mut k = j + 1;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        let ok = (2..=10).contains(&project.len())
            && b.get(j) == Some(&b'-')
            && (2..=8).contains(&(k - j))
            && (k == b.len() || !b[k].is_ascii_alphanumeric())
            && !NOT_PROJECTS.contains(&project);
        if ok {
            let key = text[i..k].to_string();
            if !out.contains(&key) {
                out.push(key);
            }
            i = k;
        } else {
            i = j.max(i + 1);
        }
    }
    out
}

/// A command whose arguments name the work it is for: a commit message, a branch
/// being made or pushed, a PR's title or body.
fn names_work(words: &[String]) -> bool {
    let w = |n: usize| words.get(n).map(String::as_str);
    match w(0) {
        Some("git") => matches!(
            w(1),
            Some("commit" | "checkout" | "switch" | "worktree" | "push" | "branch")
        ),
        Some("gh") => w(1) == Some("pr") && matches!(w(2), Some("create" | "edit")),
        _ => false,
    }
}

/// Blank out fenced blocks and inline code: `#333` in a stylesheet is not issue 333.
fn strip_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            out.push('\n');
            continue;
        }
        if fenced {
            out.push('\n');
            continue;
        }
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
                out.push(' ');
            } else {
                out.push(if in_code { ' ' } else { c });
            }
        }
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Shell commands
// ---------------------------------------------------------------------------

/// Split a command line into simple commands (on `&&`, `||`, `;`, `|`, newlines),
/// each a list of words with quotes removed. Not a shell parser: enough to find `gh`.
fn shell_segments(cmd: &str) -> Vec<Vec<String>> {
    let mut segs = vec![Vec::new()];
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();
    let end_word = |segs: &mut Vec<Vec<String>>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            segs.last_mut().unwrap().push(std::mem::take(word));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    in_word = true;
                }
                ';' | '|' | '&' | '\n' => {
                    end_word(&mut segs, &mut word, &mut in_word);
                    if matches!(c, '|' | '&') && chars.peek() == Some(&c) {
                        chars.next();
                    }
                    segs.push(Vec::new());
                }
                c if c.is_whitespace() => end_word(&mut segs, &mut word, &mut in_word),
                c => {
                    word.push(c);
                    in_word = true;
                }
            },
        }
    }
    end_word(&mut segs, &mut word, &mut in_word);
    segs.retain(|s| !s.is_empty());
    segs
}

/// A `gh pr <verb> …` / `gh issue <verb> …` invocation.
#[derive(Debug)]
struct GhCall {
    noun: &'static str,
    verb: String,
    num: Option<u64>,
    url: Option<GhRef>,
    repo: Option<String>,
}

fn gh_call(words: &[String]) -> Option<GhCall> {
    let gh = words.iter().position(|w| w == "gh")?;
    let noun = match words.get(gh + 1)?.as_str() {
        "pr" => "pr",
        "issue" => "issue",
        _ => return None,
    };
    let verb = words.get(gh + 2).cloned().unwrap_or_default();
    let mut call = GhCall {
        noun,
        verb,
        num: None,
        url: None,
        repo: None,
    };
    let mut args = words[(gh + 3).min(words.len())..].iter();
    while let Some(a) = args.next() {
        if a == "-R" || a == "--repo" {
            call.repo = args.next().and_then(|r| repo_arg(r));
        } else if let Some(r) = a.strip_prefix("--repo=") {
            call.repo = repo_arg(r);
        } else if a.starts_with('-') {
            continue;
        } else if call.verb != "create" && call.num.is_none() && call.url.is_none() {
            if a.bytes().all(|b| b.is_ascii_digit()) && !a.is_empty() {
                call.num = a.parse().ok();
            } else if let Some(r) = github_refs(a).into_iter().find(|r| r.num.is_some()) {
                call.url = Some(r);
            }
        }
    }
    if let (Some(url), None) = (&call.url, &call.repo) {
        call.repo = Some(url.repo.clone());
    }
    Some(call)
}

/// `-R owner/repo` (or `host/owner/repo`, or a URL) → `owner/repo`.
fn repo_arg(r: &str) -> Option<String> {
    if r.contains("://") {
        return github_refs(r).into_iter().next().map(|g| g.repo);
    }
    remote_key(r)
}

fn is_git_push(words: &[String]) -> bool {
    words
        .iter()
        .position(|w| w == "git")
        .is_some_and(|g| words[g + 1..].iter().any(|w| w == "push"))
}

// ---------------------------------------------------------------------------
// The repo a directory belongs to
// ---------------------------------------------------------------------------

/// `owner/repo` of the git checkout containing `dir`, from its `origin` remote (else
/// the first remote). Reads `.git/config` directly: no `git` process. Follows the
/// `gitdir:` pointer of a worktree or submodule to the config that holds the remotes.
pub(crate) fn repo_of_dir(dir: &Path) -> Option<String> {
    let mut d = Some(dir);
    while let Some(p) = d {
        let dotgit = p.join(".git");
        if dotgit.is_dir() {
            return remote_of_gitdir(&dotgit);
        }
        if dotgit.is_file() {
            let text = fs::read_to_string(&dotgit).ok()?;
            let gd = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
            let gd = if gd.is_absolute() { gd } else { p.join(gd) };
            let common = fs::read_to_string(gd.join("commondir"))
                .map(|c| gd.join(c.trim()))
                .unwrap_or(gd);
            return remote_of_gitdir(&common);
        }
        d = p.parent();
    }
    None
}

fn remote_of_gitdir(gitdir: &Path) -> Option<String> {
    let config = fs::read_to_string(gitdir.join("config")).ok()?;
    let mut section: Option<String> = None;
    let mut remotes: Vec<(String, String)> = Vec::new();
    for line in config.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            section = l
                .strip_prefix("[remote \"")
                .and_then(|s| s.strip_suffix("\"]"))
                .map(String::from);
        } else if let (Some(name), Some((k, v))) = (&section, l.split_once('=')) {
            if k.trim() == "url" {
                remotes.push((name.clone(), v.trim().to_string()));
            }
        }
    }
    let url = remotes
        .iter()
        .find(|(n, _)| n == "origin")
        .or(remotes.first())
        .map(|(_, u)| u)?;
    remote_key(url)
}

/// A remote URL's last two path segments, `owner/repo`, lowercased. Handles
/// `https://host/o/r(.git)`, `git@host:o/r.git`, `ssh://git@host/o/r` and proxy forms
/// like `http://proxy@127.0.0.1:port/git/o/r`.
fn remote_key(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches('/');
    let u = u.strip_suffix(".git").unwrap_or(u);
    let path = if let Some((_, rest)) = u.split_once("://") {
        rest.split_once('/').map(|(_, p)| p)?
    } else if let Some((host, p)) = u.split_once(':') {
        if host.contains('/') {
            return None;
        }
        p
    } else {
        u
    };
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let [.., owner, repo] = segs.as_slice() else {
        return None;
    };
    Some(format!("{owner}/{repo}").to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_keys() {
        for (url, want) in [
            (
                "https://github.com/rdmontgomery/eigenform",
                "rdmontgomery/eigenform",
            ),
            ("https://github.com/Owner/Repo.git", "owner/repo"),
            ("git@github.com:owner/repo.git", "owner/repo"),
            ("ssh://git@github.com/owner/repo", "owner/repo"),
            ("http://proxy@127.0.0.1:41823/git/owner/repo", "owner/repo"),
        ] {
            assert_eq!(remote_key(url).as_deref(), Some(want), "{url}");
        }
        assert_eq!(remote_key("https://github.com/"), None);
    }

    #[test]
    fn github_urls() {
        let refs = github_refs(
            "see https://github.com/O/R/pull/56, https://github.com/o/r/issues/7#issuecomment-1 \
             and https://github.com/o/r/pull/new/branch; not https://api.github.com/repos/x/y \
             nor https://github.com/apps/claude nor https://github.com/solo \
             nor gist.github.com/u/abc, but `github.com/a/b/pull/2`",
        );
        assert_eq!(
            refs,
            vec![
                GhRef {
                    repo: "o/r".into(),
                    num: Some(("pr", 56))
                },
                GhRef {
                    repo: "o/r".into(),
                    num: Some(("issue", 7))
                },
                GhRef {
                    repo: "o/r".into(),
                    num: None
                },
                GhRef {
                    repo: "a/b".into(),
                    num: Some(("pr", 2))
                },
            ]
        );
    }

    #[test]
    fn prose_refs_skip_code_and_urls() {
        let t = "Opened Owner/Repo#12 (fixes #3). `color: #333` `a/b#5`\n```\n#444\n```\nhttps://x.y/a/b#9";
        assert_eq!(
            text_refs(t),
            vec![
                (Some("owner/repo".into()), 12),
                (Some("a/b".into()), 5),
                (None, 3)
            ]
        );
    }

    #[test]
    fn jira_key_shapes() {
        assert_eq!(
            jira_keys(
                "feat/ABC-1234-thing, see (ABC-9) and https://x.atlassian.net/browse/OPS-77."
            ),
            ["ABC-1234", "ABC-9", "OPS-77"]
        );
        assert!(
            jira_keys("UTF-8 SHA-256 ISO-8601 RFC-3339 abc-12 X-1 ABCDEFGHIJK-1 AB-1x A1-22")
                .contains(&"A1-22".to_string())
        );
        assert_eq!(
            jira_keys("UTF-8 SHA-256 ISO-8601 RFC-3339 abc-12 X-1 ABCDEFGHIJK-1 AB-1x"),
            Vec::<String>::new()
        );
        assert_eq!(jira_keys("ABC-1 ABC-1"), ["ABC-1"]);
    }

    #[test]
    fn gh_commands() {
        let segs = shell_segments(
            "cd x && gh pr view 56 -R o/r --json url | jq . ; gh pr create --title \"fix 9\"",
        );
        let calls: Vec<_> = segs.iter().filter_map(|s| gh_call(s)).collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            (calls[0].num, calls[0].repo.as_deref()),
            (Some(56), Some("o/r"))
        );
        assert_eq!((calls[1].verb.as_str(), calls[1].num), ("create", None));
        let merge =
            gh_call(&shell_segments("gh pr merge https://github.com/a/b/pull/4 --squash")[0])
                .unwrap();
        assert_eq!(merge.repo.as_deref(), Some("a/b"));
        assert_eq!(merge.url.and_then(|u| u.num), Some(("pr", 4)));
        assert!(is_git_push(
            &shell_segments("git -C dir push -u origin b")[0]
        ));
    }
}
