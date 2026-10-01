//! eigenform-forest: discover and resolve Claude Code sessions across projects.
//!
//! v0.1 kills path-pasting: resolve a session by uuid (or unique prefix) machine-wide,
//! and list recent sessions per project. See
//! `docs/plans/2026-06-03-forest-crate-design.md`.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Utc};
use thiserror::Error;

/// First tail window read; doubles on escalation up to the whole file.
const TAIL_WINDOW: u64 = 64 * 1024;
/// Max chars kept from a last-prompt fallback title.
const TITLE_SNIPPET: usize = 60;

/// A session enriched with its recency and title (requires a tail read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    pub uuid: String,
    pub path: PathBuf,
    pub cwd: PathBuf,
    /// Last conversational timestamp, else the file's mtime.
    pub recency: DateTime<Utc>,
    /// Last `ai-title`, else a snippet of the last `last-prompt`.
    pub title: Option<String>,
}

/// A cheaply-enumerated session: filename uuid, path, and owning project cwd. No file
/// contents are read to build a stub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStub {
    pub uuid: String,
    pub path: PathBuf,
    pub cwd: PathBuf,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("no session matches `{0}`")]
    NotFound(String),
    #[error("ambiguous: {} sessions match", .0.len())]
    Ambiguous(Vec<SessionStub>),
    #[error(transparent)]
    Enumerate(#[from] Error),
}

/// Enumerate every session under `projects_dir/<project>/<uuid>.jsonl`, attaching each
/// project's recovered cwd. Reads no session contents.
pub fn enumerate_session_stubs(projects_dir: &Path) -> Result<Vec<SessionStub>> {
    let cwd_by_dir: HashMap<String, PathBuf> = eigenform_projects::enumerate_projects(projects_dir)
        .map(|projects| {
            projects
                .into_iter()
                .map(|p| (p.dir_name, p.cwd))
                .collect()
        })
        .unwrap_or_default();

    let entries = fs::read_dir(projects_dir).map_err(|e| Error::Io {
        path: projects_dir.to_path_buf(),
        source: e,
    })?;

    let mut out = Vec::new();
    for project in entries.flatten() {
        let pdir = project.path();
        if !pdir.is_dir() {
            continue;
        }
        let dir_name = match pdir.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let cwd = cwd_by_dir
            .get(&dir_name)
            .cloned()
            .unwrap_or_else(|| decode_dir_name(&dir_name));

        let files = fs::read_dir(&pdir).map_err(|e| Error::Io {
            path: pdir.clone(),
            source: e,
        })?;
        for f in files.flatten() {
            let path = f.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) {
                out.push(SessionStub {
                    uuid: uuid.to_string(),
                    path: path.clone(),
                    cwd: cwd.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// A discovered async subagent transcript, spawned from a parent session via the Agent
/// tool. `agent_type`/`description` come from the sibling `.meta.json` when present and
/// parseable — their absence degrades gracefully rather than hiding the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentStub {
    pub agent_id: String,
    pub path: PathBuf,
    pub agent_type: Option<String>,
    pub description: Option<String>,
}

/// Enumerate the subagent transcripts a session spawned, from
/// `<session_path stem>/subagents/agent-<id>.jsonl` (+ sibling `.meta.json`). Reads no
/// jsonl contents. Returns empty if the subagents dir doesn't exist.
pub fn enumerate_subagents(session_path: &Path) -> Vec<SubagentStub> {
    let Some(stem) = session_path.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let subagents_dir = session_path.with_file_name(stem).join("subagents");
    let Ok(entries) = fs::read_dir(&subagents_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(agent_id) = file_stem.strip_prefix("agent-") else {
            continue;
        };

        let meta_path = subagents_dir.join(format!("agent-{agent_id}.meta.json"));
        let meta: Option<serde_json::Value> = fs::read_to_string(&meta_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());

        out.push(SubagentStub {
            agent_id: agent_id.to_string(),
            path,
            agent_type: meta
                .as_ref()
                .and_then(|m| m.get("agentType"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            description: meta
                .as_ref()
                .and_then(|m| m.get("description"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }
    out
}

/// Resolve a session `query` (full uuid or unique prefix) to its path, machine-wide.
pub fn resolve(projects_dir: &Path, query: &str) -> std::result::Result<PathBuf, ResolveError> {
    Ok(resolve_stub(projects_dir, query)?.path)
}

/// Like [`resolve`], but returns the full [`SessionStub`] (uuid, path, and cwd).
pub fn resolve_stub(
    projects_dir: &Path,
    query: &str,
) -> std::result::Result<SessionStub, ResolveError> {
    let stubs = enumerate_session_stubs(projects_dir)?;

    if let Some(exact) = stubs.iter().find(|s| s.uuid == query) {
        return Ok(exact.clone());
    }
    let matches: Vec<SessionStub> = stubs
        .into_iter()
        .filter(|s| s.uuid.starts_with(query))
        .collect();
    match matches.len() {
        0 => Err(ResolveError::NotFound(query.to_string())),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => Err(ResolveError::Ambiguous(matches)),
    }
}

/// Which sessions `list` considers.
#[derive(Debug, Clone)]
pub enum Scope {
    /// Only sessions belonging to the project at this cwd.
    Project(PathBuf),
    /// Every session on the machine.
    AllProjects,
}

/// List sessions, scoped and windowed, sorted recent-first.
pub fn list(
    projects_dir: &Path,
    scope: Scope,
    since: Option<chrono::Duration>,
    now: DateTime<Utc>,
) -> Result<Vec<SessionRef>> {
    let stubs = enumerate_session_stubs(projects_dir)?;
    let cutoff = since.map(|d| now - d);

    let mut refs: Vec<SessionRef> = stubs
        .iter()
        .filter(|s| match &scope {
            Scope::AllProjects => true,
            Scope::Project(cwd) => &s.cwd == cwd,
        })
        .map(session_ref)
        .filter(|r| cutoff.is_none_or(|c| r.recency >= c))
        .collect();

    // Recent-first; the CLI/render emits newest-at-bottom by reversing.
    refs.sort_by_key(|b| std::cmp::Reverse(b.recency));
    Ok(refs)
}

/// Enrich a stub by tail-peeking its file for recency and title.
pub fn session_ref(stub: &SessionStub) -> SessionRef {
    let tail = peek_tail(&stub.path);
    let recency = tail
        .last_timestamp
        .as_deref()
        .and_then(parse_ts)
        .unwrap_or_else(|| mtime_of(&stub.path));
    SessionRef {
        uuid: stub.uuid.clone(),
        path: stub.path.clone(),
        cwd: stub.cwd.clone(),
        recency,
        title: tail.title,
    }
}

/// A session's process state, corroborated from disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Live process, a turn in flight (last prompt not yet closed).
    Working,
    /// Live process, last turn complete — awaiting your input.
    Ready,
    /// No live process; history.
    Recent,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Working => "working",
            SessionState::Ready => "ready",
            SessionState::Recent => "recent",
        }
    }
    fn rank(self) -> u8 {
        match self {
            SessionState::Ready => 0,
            SessionState::Working => 1,
            SessionState::Recent => 2,
        }
    }
}

/// A Forest row: a session enriched with liveness, process state, and activity spark.
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub uuid: String,
    pub title: Option<String>,
    pub cwd: PathBuf,
    pub recency: DateTime<Utc>,
    pub live: bool,
    pub state: SessionState,
    /// Per-turn output-token counts (the activity sparkline). Empty until metrics exist.
    pub spark: Vec<u32>,
    /// Launched non-interactively (`claude -p`, an Agent SDK host) — see [`is_headless`].
    pub headless: bool,
    /// The live process's pid, from its session claim. None for dead sessions.
    pub pid: Option<u32>,
}

/// Is a process alive? `/proc/<pid>` on Linux/WSL (this project's target).
pub fn is_pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// A process's start time (clock ticks since boot, `/proc/<pid>/stat` field 22) — the
/// value Claude Code records as `procStart` in its session claim. None off-Linux or when
/// the process is gone.
pub fn proc_start_of(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `comm` (field 2) is parenthesised and may contain spaces; fields resume after the
    // last ')' at field 3, so starttime (22) is the 20th token of the remainder.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19).map(str::to_string)
}

/// Is this entrypoint a non-interactive launch? `claude -p` writes `sdk-cli`; the Agent
/// SDKs write `sdk-ts` / `sdk-py`. The interactive TUI writes `cli`.
pub fn is_headless(entrypoint: &str) -> bool {
    entrypoint.starts_with("sdk-")
}

/// How much to trust a `sessions/<pid>.json` claim that a session is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimHealth {
    /// The pid is alive and (where recorded) its start time matches the claim.
    Alive,
    /// No such process — Claude exited without cleaning up its claim.
    Dead,
    /// The pid is alive but belongs to a different, later process (pid reuse): the claim
    /// is a ghost that a bare pid check would wrongly report as live.
    Reused,
}

impl ClaimHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimHealth::Alive => "alive",
            ClaimHealth::Dead => "dead",
            ClaimHealth::Reused => "reused",
        }
    }
}

/// One `~/.claude/sessions/<pid>.json`: Claude Code's own claim that a session is running.
#[derive(Debug, Clone)]
pub struct Claim {
    pub pid: u32,
    pub session_id: String,
    pub cwd: Option<PathBuf>,
    /// Epoch milliseconds.
    pub started_at: Option<i64>,
    pub kind: Option<String>,
    pub entrypoint: Option<String>,
    /// Claude's own busy/idle status, when it records one.
    pub status: Option<String>,
    pub name: Option<String>,
    pub path: PathBuf,
    pub health: ClaimHealth,
}

/// Read every session claim, judging each against the real process table.
pub fn read_claims(sessions_dir: &Path) -> Vec<Claim> {
    read_claims_with(sessions_dir, is_pid_alive, proc_start_of)
}

/// [`read_claims`] with injected liveness + start-time probes (for deterministic tests).
/// A claim without a recorded `procStart`, or whose process start can't be read, is
/// judged on the pid alone.
pub fn read_claims_with(
    sessions_dir: &Path,
    alive: impl Fn(u32) -> bool,
    start_of: impl Fn(u32) -> Option<String>,
) -> Vec<Claim> {
    let Ok(entries) = fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    let str_of = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let mut out = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let Some(pid) = v.get("pid").and_then(|x| x.as_u64()).map(|x| x as u32) else { continue };
        let Some(session_id) = str_of(&v, "sessionId") else { continue };
        let health = if !alive(pid) {
            ClaimHealth::Dead
        } else {
            match (str_of(&v, "procStart"), start_of(pid)) {
                (Some(claimed), Some(actual)) if claimed != actual => ClaimHealth::Reused,
                _ => ClaimHealth::Alive,
            }
        };
        out.push(Claim {
            pid,
            session_id,
            cwd: str_of(&v, "cwd").map(PathBuf::from),
            started_at: v.get("startedAt").and_then(|x| x.as_i64()),
            kind: str_of(&v, "kind"),
            entrypoint: str_of(&v, "entrypoint"),
            status: str_of(&v, "status"),
            name: str_of(&v, "name"),
            path: p,
            health,
        });
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.started_at.unwrap_or(0)));
    out
}

/// The `entrypoint` a session transcript was written under (`cli`, `sdk-cli`, …), from
/// the first rows that carry one. Reads only the head of the file, once: a found value is
/// memoized per path (it never changes), so the forest's 3s tick doesn't re-read heads.
pub fn session_entrypoint(path: &Path) -> Option<String> {
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(hit) = seen.lock().ok().and_then(|m| m.get(path).cloned()) {
        return Some(hit);
    }
    let found = read_entrypoint(path)?;
    if let Ok(mut m) = seen.lock() {
        m.insert(path.to_path_buf(), found.clone());
    }
    Some(found)
}

fn read_entrypoint(path: &Path) -> Option<String> {
    let mut buf = Vec::new();
    fs::File::open(path).ok()?.take(TAIL_WINDOW).read_to_end(&mut buf).ok()?;
    String::from_utf8_lossy(&buf).lines().find_map(|line| {
        let v = serde_json::from_str::<serde_json::Value>(line).ok()?;
        v.get("entrypoint").and_then(|x| x.as_str()).map(str::to_string)
    })
}

/// Whether a session's last turn has closed (ready) vs is in flight (working), from a
/// cheap tail-peek. Used to badge live sessions.
pub fn session_complete(path: &Path) -> bool {
    peek_tail(path).complete
}

/// The activity sparkline: `output_tokens` summed per completed turn (assistant messages
/// accumulate; a `turn_duration` system row closes a turn and pushes its total). Requires
/// a full read — use [`cached_spark`] for the persisted, parse-on-change form.
pub fn session_spark(jsonl_path: &Path) -> Vec<u32> {
    let Ok(text) = fs::read_to_string(jsonl_path) else {
        return Vec::new();
    };
    let mut spark = Vec::new();
    let mut acc: u32 = 0;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                let usage = v
                    .get("message")
                    .and_then(|m| m.get("usage"))
                    .or_else(|| v.get("usage"));
                if let Some(out) = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|x| x.as_u64())
                {
                    acc = acc.saturating_add(out as u32);
                }
            }
            Some("system")
                if v.get("subtype").and_then(|s| s.as_str()) == Some("turn_duration") =>
            {
                spark.push(acc);
                acc = 0;
            }
            _ => {}
        }
    }
    spark
}

fn mtime_millis(path: &Path) -> Option<(i64, u64)> {
    let m = fs::metadata(path).ok()?;
    let millis = m
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((millis, m.len()))
}

/// [`session_spark`] cached to `state_dir/<session_id>.json`, keyed by the JSONL's
/// (mtime, len). A static transcript is parsed once; the cache (eigenform's `~/.eigenform/state`)
/// survives restarts and is shared with the CLI.
pub fn cached_spark(state_dir: &Path, session_id: &str, jsonl_path: &Path) -> Vec<u32> {
    let stamp = mtime_millis(jsonl_path);
    let state_path = state_dir.join(format!("{session_id}.json"));

    if let Some((mtime, len)) = stamp {
        if let Ok(text) = fs::read_to_string(&state_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let same = v.get("source_mtime").and_then(|x| x.as_i64()) == Some(mtime)
                    && v.get("source_len").and_then(|x| x.as_u64()) == Some(len);
                if same {
                    if let Some(arr) = v.get("spark").and_then(|x| x.as_array()) {
                        return arr.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect();
                    }
                }
            }
        }
    }

    let spark = session_spark(jsonl_path);
    if let Some((mtime, len)) = stamp {
        let _ = fs::create_dir_all(state_dir);
        let total: u64 = spark.iter().map(|&x| x as u64).sum();
        let doc = serde_json::json!({
            "source_mtime": mtime,
            "source_len": len,
            "spark": spark,
            "total": total,
        });
        let _ = fs::write(&state_path, doc.to_string());
    }
    spark
}

/// The live Forest: corroborate `~/.claude/sessions/<pid>.json` (process liveness) with
/// the project JSONLs (state, title, recency). The source of truth is the filesystem —
/// reconstructed on demand — so it survives a daemon that wasn't running when sessions
/// started, and a dead pid's stale session file is simply ignored (the pid check is the GC).
pub fn live_forest(
    projects_dir: &Path,
    sessions_dir: &Path,
    state_dir: &Path,
    now: DateTime<Utc>,
) -> Vec<LiveSession> {
    live_forest_with(projects_dir, sessions_dir, state_dir, now, is_pid_alive)
}

/// [`live_forest`] with an injected liveness predicate (for deterministic tests).
pub fn live_forest_with(
    projects_dir: &Path,
    sessions_dir: &Path,
    state_dir: &Path,
    now: DateTime<Utc>,
    alive: impl Fn(u32) -> bool,
) -> Vec<LiveSession> {
    // sessionId → its live claim. A dead pid's stale claim is simply ignored (the pid
    // check is the GC), and so is a reused pid's ghost claim (the procStart check).
    let mut live: HashMap<String, Claim> = HashMap::new();
    for c in read_claims_with(sessions_dir, alive, proc_start_of) {
        if c.health == ClaimHealth::Alive {
            live.insert(c.session_id.clone(), c);
        }
    }

    let recents = list(projects_dir, Scope::AllProjects, None, now).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<LiveSession> = Vec::new();
    for r in &recents {
        seen.insert(r.uuid.clone());
        let claim = live.get(&r.uuid);
        let is_live = claim.is_some();
        let state = if is_live {
            if session_complete(&r.path) {
                SessionState::Ready
            } else {
                SessionState::Working
            }
        } else {
            SessionState::Recent
        };
        out.push(LiveSession {
            uuid: r.uuid.clone(),
            title: r.title.clone(),
            cwd: r.cwd.clone(),
            recency: r.recency,
            live: is_live,
            state,
            spark: cached_spark(state_dir, &r.uuid, &r.path),
            headless: claim_headless(claim) || session_entrypoint(&r.path).is_some_and(|e| is_headless(&e)),
            pid: claim.map(|c| c.pid),
        });
    }
    // Live sessions whose JSONL hasn't landed yet (brand-new): show them anyway.
    for (sid, c) in &live {
        if seen.contains(sid) {
            continue;
        }
        out.push(LiveSession {
            uuid: sid.clone(),
            title: None,
            cwd: c.cwd.clone().unwrap_or_default(),
            recency: now,
            live: true,
            state: SessionState::Working,
            spark: Vec::new(),
            headless: claim_headless(Some(c)),
            pid: Some(c.pid),
        });
    }

    out.sort_by(|a, b| {
        a.state
            .rank()
            .cmp(&b.state.rank())
            .then(b.recency.cmp(&a.recency))
    });
    out
}

fn claim_headless(c: Option<&Claim>) -> bool {
    c.and_then(|c| c.entrypoint.as_deref()).is_some_and(is_headless)
}

struct Tail {
    last_timestamp: Option<String>,
    title: Option<String>,
    /// Did the last turn close? Ready iff the last `turn_duration` row follows the last
    /// `user` row (a turn completed after the latest prompt). A trailing user prompt with
    /// no close means a turn is in flight (working). No user row in the window → assume
    /// idle/ready. Trailing bridge/title/mode metadata rows are ignored.
    complete: bool,
}

/// Read the tail of a session file (byte-stream, escalating) for the last timestamped
/// row and a title. Returns empties if the file is unreadable.
fn peek_tail(path: &Path) -> Tail {
    let Ok(mut file) = fs::File::open(path) else {
        return Tail { last_timestamp: None, title: None, complete: true };
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);

    let mut window = TAIL_WINDOW;
    loop {
        let start = len.saturating_sub(window);
        let mut buf = Vec::new();
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut buf).is_err() {
            return Tail { last_timestamp: None, title: None, complete: true };
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines: Vec<&str> = text.split('\n').filter(|l| !l.is_empty()).collect();
        // If we didn't start at the file head, the first line is likely a partial row.
        if start > 0 && !lines.is_empty() {
            lines.remove(0);
        }

        let tail = scan_tail(&lines);
        // Found a timestamp, or we've already read the whole file — done either way.
        if tail.last_timestamp.is_some() || start == 0 {
            return tail;
        }
        window = window.saturating_mul(2);
    }
}

/// Scan complete lines (in file order) for the last timestamped row, the last ai-title,
/// and a last-prompt fallback title.
fn scan_tail(lines: &[&str]) -> Tail {
    let mut last_timestamp = None;
    let mut ai_title = None;
    let mut last_prompt = None;
    let mut last_user = None;
    let mut last_close = None;
    for (i, line) in lines.iter().enumerate() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(ts) = value.get("timestamp").and_then(|t| t.as_str()) {
            last_timestamp = Some(ts.to_string());
        }
        match value.get("type").and_then(|t| t.as_str()) {
            Some("ai-title") => {
                if let Some(t) = value.get("aiTitle").and_then(|t| t.as_str()) {
                    ai_title = Some(t.to_string());
                }
            }
            Some("last-prompt") => {
                if let Some(p) = value.get("lastPrompt").and_then(|t| t.as_str()) {
                    last_prompt = Some(snippet(p));
                }
            }
            Some("user") => last_user = Some(i),
            Some("system")
                if value.get("subtype").and_then(|s| s.as_str()) == Some("turn_duration") =>
            {
                last_close = Some(i);
            }
            _ => {}
        }
    }
    let complete = match (last_user, last_close) {
        (Some(u), Some(c)) => c >= u,
        (Some(_), None) => false,
        (None, _) => true,
    };
    Tail {
        last_timestamp,
        title: ai_title.or(last_prompt),
        complete,
    }
}

fn snippet(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= TITLE_SNIPPET {
        s
    } else {
        format!("{}…", s.chars().take(TITLE_SNIPPET).collect::<String>())
    }
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn mtime_of(path: &Path) -> DateTime<Utc> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now())
}

/// Decode a project dir name (`-home-me-proj`) back to a best-effort cwd. Lossy for paths
/// containing `-`; only used when a project's cwd couldn't be recovered from its JSONLs.
fn decode_dir_name(dir_name: &str) -> PathBuf {
    PathBuf::from(dir_name.replace('-', "/"))
}
