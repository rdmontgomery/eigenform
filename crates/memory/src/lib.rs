//! eigenform-memory: scan and reason about Claude Code auto-memory directories
//! (`~/.claude/projects/<escaped-cwd>/memory/`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Kinds of memory entry, mirroring the `type` field in frontmatter.
/// Unknown values are preserved in `Other(...)` rather than dropped.
///
/// Ordering of the known variants here is significant: it's the precedence
/// used by `scan_memory_dir` when sorting results.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryKind {
    Feedback,
    Project,
    Reference,
    User,
    Other(String),
}

impl MemoryKind {
    pub fn as_tag(&self) -> &str {
        match self {
            MemoryKind::Feedback => "feedback",
            MemoryKind::Project => "project",
            MemoryKind::Reference => "reference",
            MemoryKind::User => "user",
            MemoryKind::Other(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    pub kind: MemoryKind,
    pub source_path: PathBuf,
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

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(rename = "type")]
    kind: String,
}

/// Scan a single memory directory. Missing directories return an empty Vec,
/// not an error. Files without YAML frontmatter, and the conventional
/// `MEMORY.md` index file, are skipped silently.
pub fn scan_memory_dir(dir: &Path) -> Result<Vec<MemoryEntry>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("MEMORY.md") {
            continue;
        }
        let body = fs::read_to_string(&path).map_err(|e| Error::Io {
            path: path.clone(),
            source: e,
        })?;
        let Some(fm) = parse_frontmatter(&body) else {
            continue;
        };
        out.push(MemoryEntry {
            name: fm.name,
            description: fm.description,
            kind: classify(&fm.kind),
            source_path: path,
        });
    }

    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn parse_frontmatter(body: &str) -> Option<Frontmatter> {
    let rest = body.strip_prefix("---\n")?;
    let (yaml, _) = rest.split_once("\n---")?;
    serde_yaml::from_str(yaml).ok()
}

fn classify(kind: &str) -> MemoryKind {
    match kind {
        "feedback" => MemoryKind::Feedback,
        "project" => MemoryKind::Project,
        "reference" => MemoryKind::Reference,
        "user" => MemoryKind::User,
        other => MemoryKind::Other(other.to_string()),
    }
}

/// Render the per-project memory tree as text: grouped by kind in the
/// known precedence order (feedback → project → reference → user → other),
/// with entry names + descriptions under each section.
pub fn render_memory_tree(entries: &[MemoryEntry]) -> String {
    let mut out = String::from("MEMORY\n======\n\n");
    if entries.is_empty() {
        out.push_str("(no memory entries found)\n");
        return out;
    }
    let mut groups: BTreeMap<&MemoryKind, Vec<&MemoryEntry>> = BTreeMap::new();
    for e in entries {
        groups.entry(&e.kind).or_default().push(e);
    }
    for (kind, items) in &groups {
        let _ = writeln!(out, "[{}]", kind.as_tag());
        for e in items {
            let _ = writeln!(out, "  {}", e.name);
            let _ = writeln!(out, "    {}", e.description);
        }
        out.push('\n');
    }
    out
}
