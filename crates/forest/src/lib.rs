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
    /// Present iff a Fable→Opus guardrail downgrade was detected in this session.
    pub downgrade: Option<Downgrade>,
}

/// A detected Fable→Opus **guardrail** downgrade in a session transcript.
#[derive(Debug, Clone)]
pub struct Downgrade {
    /// The main-chain user turn whose response tripped the guardrail — the
    /// `fork_before` target. Forking before it drops the offending prompt so the
    /// user can restate it on a fresh Fable branch.
    pub offending_turn: String,
}

/// The guardrail-downgrade notice string Claude Code writes as a `<synthetic>`
/// assistant turn.
///
/// ⚠ SCRUBBED IN. No real guardrail sample exists in local transcripts yet — see
/// `notes/spikes/10-resume-model-derivation.md` (only session-limit + API-error
/// synthetics were observed). Capture the true string from the first live
/// occurrence and replace this one line, recording it with `claude --version` in
/// a new spike. Matching is a substring test, so a stable fragment is enough.
pub const GUARDRAIL_MARKER: &str = "switched this session to a safer model"; // PLACEHOLDER

/// Scan a session JSONL for the first guardrail downgrade. Returns the offending
/// user turn, or `None`. Pure: reads the file, no side effects.
///
/// A downgrade notice is a **main-chain** (`isSidechain:false`) `assistant` row
/// whose `message.model == "<synthetic>"` and whose text contains
/// [`GUARDRAIL_MARKER`]. Sidechain (subagent) turns are ignored — an Opus
/// subagent is benign, not a downgrade of your thread. The offending turn is the
/// last main-chain `user` turn *before* that notice.
pub fn detect_downgrade(jsonl_path: &Path) -> Option<Downgrade> {
    let text = fs::read_to_string(jsonl_path).ok()?;
    let mut last_user: Option<String> = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("isSidechain").and_then(|b| b.as_bool()).unwrap_or(false) {
            continue; // subagent / sidechain — never a downgrade of the main thread
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                if let Some(uuid) = v.get("uuid").and_then(|x| x.as_str()) {
                    last_user = Some(uuid.to_string());
                }
            }
            Some("assistant") => {
                let msg = v.get("message");
                let model = msg.and_then(|m| m.get("model")).and_then(|x| x.as_str());
                if model == Some("<synthetic>") && synthetic_text(msg).contains(GUARDRAIL_MARKER) {
                    return last_user.map(|offending_turn| Downgrade { offending_turn });
                }
            }
            _ => {}
        }
    }
    None
}

/// Concatenate the text blocks of an assistant `message.content` (string or array form).
fn synthetic_text(msg: Option<&serde_json::Value>) -> String {
    let Some(content) = msg.and_then(|m| m.get("content")) else { return String::new() };
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|x| x.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// Is a process alive? `/proc/<pid>` on Linux/WSL (this project's target).
pub fn is_pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
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

/// [`detect_downgrade`] cached to `state_dir/<session_id>.downgrade.json`, keyed by the
/// JSONL's (mtime, len) — the same on-change discipline as [`cached_spark`]. A downgrade is
/// an immutable property of a transcript's content, so once the file stops changing the full
/// scan runs at most once. Avoids re-reading every transcript on every forest snapshot.
pub fn cached_downgrade(state_dir: &Path, session_id: &str, jsonl_path: &Path) -> Option<Downgrade> {
    let stamp = mtime_millis(jsonl_path);
    let state_path = state_dir.join(format!("{session_id}.downgrade.json"));

    if let Some((mtime, len)) = stamp {
        if let Ok(text) = fs::read_to_string(&state_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let same = v.get("source_mtime").and_then(|x| x.as_i64()) == Some(mtime)
                    && v.get("source_len").and_then(|x| x.as_u64()) == Some(len);
                if same {
                    // Cached: `offending_turn` is a string when a downgrade was found, null otherwise.
                    return v.get("offending_turn")
                        .and_then(|x| x.as_str())
                        .map(|s| Downgrade { offending_turn: s.to_string() });
                }
            }
        }
    }

    let found = detect_downgrade(jsonl_path);
    if let Some((mtime, len)) = stamp {
        let _ = fs::create_dir_all(state_dir);
        let doc = serde_json::json!({
            "source_mtime": mtime,
            "source_len": len,
            "offending_turn": found.as_ref().map(|d| d.offending_turn.clone()),
        });
        let _ = fs::write(&state_path, doc.to_string());
    }
    found
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
    // sessionId → cwd, for the processes that are actually alive.
    let mut live: HashMap<String, PathBuf> = HashMap::new();
    if let Ok(entries) = fs::read_dir(sessions_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&p) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let pid = v.get("pid").and_then(|x| x.as_u64()).map(|x| x as u32);
            let sid = v.get("sessionId").and_then(|x| x.as_str()).map(str::to_string);
            let cwd = v.get("cwd").and_then(|x| x.as_str()).map(PathBuf::from);
            if let (Some(pid), Some(sid)) = (pid, sid) {
                if alive(pid) {
                    live.insert(sid, cwd.unwrap_or_default());
                }
            }
        }
    }

    let recents = list(projects_dir, Scope::AllProjects, None, now).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<LiveSession> = Vec::new();
    for r in &recents {
        seen.insert(r.uuid.clone());
        let is_live = live.contains_key(&r.uuid);
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
            downgrade: cached_downgrade(state_dir, &r.uuid, &r.path),
        });
    }
    // Live sessions whose JSONL hasn't landed yet (brand-new): show them anyway.
    for (sid, cwd) in &live {
        if seen.contains(sid) {
            continue;
        }
        out.push(LiveSession {
            uuid: sid.clone(),
            title: None,
            cwd: cwd.clone(),
            recency: now,
            live: true,
            state: SessionState::Working,
            spark: Vec::new(),
            downgrade: None,
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

#[cfg(test)]
mod downgrade_tests {
    use super::*;
    use std::io::Write;

    fn tmp_jsonl(body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    // user(offending) -> synthetic guardrail notice -> opus assistant
    fn guardrail_fixture() -> String {
        [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"benign question"},"sessionId":"s"}"#.to_string(),
            r#"{"type":"assistant","isSidechain":false,"uuid":"a1","message":{"model":"claude-fable-5","role":"assistant","content":[{"type":"text","text":"ok"}]},"sessionId":"s"}"#.to_string(),
            r#"{"type":"system","isSidechain":false,"subtype":"turn_duration","uuid":"sys1","sessionId":"s"}"#.to_string(),
            r#"{"type":"user","isSidechain":false,"uuid":"u2","message":{"role":"user","content":"the offending prompt"},"sessionId":"s"}"#.to_string(),
            format!(r#"{{"type":"assistant","isSidechain":false,"uuid":"synth","message":{{"model":"<synthetic>","role":"assistant","content":[{{"type":"text","text":"{GUARDRAIL_MARKER}"}}]}},"sessionId":"s"}}"#),
            r#"{"type":"assistant","isSidechain":false,"uuid":"a2","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"reply"}]},"sessionId":"s"}"#.to_string(),
        ].join("\n") + "\n"
    }

    #[test]
    fn fires_on_guardrail_marker_targeting_offending_user_turn() {
        let f = tmp_jsonl(&guardrail_fixture());
        let d = detect_downgrade(f.path()).expect("should fire");
        assert_eq!(d.offending_turn, "u2"); // last main-chain user turn before the marker
    }

    #[test]
    fn session_limit_fallback_does_not_fire() {
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"go"},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"synth","message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"You've hit your session limit · resets 4pm"}]},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"a2","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"reply"}]},"sessionId":"s"}"#,
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }

    #[test]
    fn opus_subagent_sidechain_does_not_fire() {
        // A guardrail-marker synthetic on a SIDECHAIN must be ignored: it's a
        // subagent's notice, not a downgrade of the main thread. Deleting the
        // isSidechain guard would make this fire — so it genuinely pins it.
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"go"},"sessionId":"s"}"#.to_string(),
            format!(r#"{{"type":"assistant","isSidechain":true,"uuid":"synthsub","message":{{"model":"<synthetic>","role":"assistant","content":[{{"type":"text","text":"{GUARDRAIL_MARKER}"}}]}},"sessionId":"s"}}"#),
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }

    #[test]
    fn sidechain_user_turn_is_not_the_offending_turn() {
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"real","message":{"role":"user","content":"the real prompt"},"sessionId":"s"}"#.to_string(),
            r#"{"type":"user","isSidechain":true,"uuid":"subuser","message":{"role":"user","content":"subagent prompt"},"sessionId":"s"}"#.to_string(),
            format!(r#"{{"type":"assistant","isSidechain":false,"uuid":"synth","message":{{"model":"<synthetic>","role":"assistant","content":[{{"type":"text","text":"{GUARDRAIL_MARKER}"}}]}},"sessionId":"s"}}"#),
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        assert_eq!(detect_downgrade(f.path()).unwrap().offending_turn, "real");
    }

    #[test]
    fn always_opus_session_does_not_fire() {
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"go"},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"a1","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"reply"}]},"sessionId":"s"}"#,
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }

    #[test]
    fn cached_downgrade_caches_a_hit_and_writes_state_file() {
        let f = tmp_jsonl(&guardrail_fixture());
        let state = tempfile::tempdir().unwrap();
        let id = "sess-hit";

        let first = cached_downgrade(state.path(), id, f.path());
        assert_eq!(first.unwrap().offending_turn, "u2");
        assert!(
            state.path().join(format!("{id}.downgrade.json")).exists(),
            "cache state file must be written"
        );
        // Second call (now served from the state file) yields the same result.
        let second = cached_downgrade(state.path(), id, f.path());
        assert_eq!(second.unwrap().offending_turn, "u2");
    }

    #[test]
    fn cached_downgrade_caches_a_miss_as_null() {
        let body = [
            r#"{"type":"user","isSidechain":false,"uuid":"u1","message":{"role":"user","content":"go"},"sessionId":"s"}"#,
            r#"{"type":"assistant","isSidechain":false,"uuid":"a1","message":{"model":"claude-fable-5","role":"assistant","content":[{"type":"text","text":"reply"}]},"sessionId":"s"}"#,
        ].join("\n") + "\n";
        let f = tmp_jsonl(&body);
        let state = tempfile::tempdir().unwrap();
        let id = "sess-clean";

        assert!(cached_downgrade(state.path(), id, f.path()).is_none());
        let state_file = state.path().join(format!("{id}.downgrade.json"));
        assert!(state_file.exists(), "a miss must still write the cache file");
        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
        assert!(doc["offending_turn"].is_null(), "a miss caches null");
        // Second call still None (served from cache).
        assert!(cached_downgrade(state.path(), id, f.path()).is_none());
    }

    #[test]
    fn marker_with_no_prior_user_turn_does_not_fire() {
        let body = format!(
            r#"{{"type":"assistant","isSidechain":false,"uuid":"synth","message":{{"model":"<synthetic>","role":"assistant","content":[{{"type":"text","text":"{GUARDRAIL_MARKER}"}}]}},"sessionId":"s"}}"#
        ) + "\n";
        let f = tmp_jsonl(&body);
        assert!(detect_downgrade(f.path()).is_none());
    }
}
