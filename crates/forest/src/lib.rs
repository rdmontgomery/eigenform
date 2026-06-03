//! eigen-forest: discover and resolve Claude Code sessions across projects.
//!
//! v0.1 kills path-pasting: resolve a session by uuid (or unique prefix) machine-wide,
//! and list recent sessions per project. See
//! `docs/plans/2026-06-03-forest-crate-design.md`.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

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
    let cwd_by_dir: HashMap<String, PathBuf> = eigen_projects::enumerate_projects(projects_dir)
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
        .filter(|r| cutoff.map_or(true, |c| r.recency >= c))
        .collect();

    // Recent-first; the CLI/render emits newest-at-bottom by reversing.
    refs.sort_by(|a, b| b.recency.cmp(&a.recency));
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

struct Tail {
    last_timestamp: Option<String>,
    title: Option<String>,
}

/// Read the tail of a session file (byte-stream, escalating) for the last timestamped
/// row and a title. Returns empties if the file is unreadable.
fn peek_tail(path: &Path) -> Tail {
    let Ok(mut file) = fs::File::open(path) else {
        return Tail { last_timestamp: None, title: None };
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);

    let mut window = TAIL_WINDOW;
    loop {
        let start = len.saturating_sub(window);
        let mut buf = Vec::new();
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut buf).is_err() {
            return Tail { last_timestamp: None, title: None };
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
    for line in lines {
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
            _ => {}
        }
    }
    Tail {
        last_timestamp,
        title: ai_title.or(last_prompt),
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
