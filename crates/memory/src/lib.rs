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
    /// Size of the source file in bytes.
    pub size: usize,
    /// Rough token estimate for the source file (see [`estimate_tokens`]).
    pub tokens: usize,
}

/// A coarse token estimate: ~4 bytes/token, the rule of thumb for English text
/// under the Claude tokenizer. Deliberately cheap and dependency-free — a
/// budgeting aid, not a billing oracle. Mirrors `eigenform_skills::estimate_tokens`.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Format a token count for display: `~N tok` under 1k, `~N.Nk tok` above.
pub fn fmt_tokens(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("~{:.1}k tok", tokens as f64 / 1000.0)
    } else {
        format!("~{tokens} tok")
    }
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
            size: body.len(),
            tokens: estimate_tokens(&body),
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

/// Truncate `s` to at most `width` display chars, ending in `…` when cut.
/// Whitespace is collapsed first so multi-line descriptions read as one line.
pub fn truncate_line(s: &str, width: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= width {
        return flat;
    }
    let kept: String = flat.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// One-line summary for a set of entries: `5 entries · ~3.1k tok`.
pub fn summarize(entries: &[MemoryEntry]) -> String {
    if entries.is_empty() {
        return "no memory entries".to_string();
    }
    let total: usize = entries.iter().map(|e| e.tokens).sum();
    format!(
        "{} entr{} · {}",
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" },
        fmt_tokens(total)
    )
}

/// Render one project's memory as text: a `label · summary` line, then kind
/// groups in the known precedence order (feedback → project → reference →
/// user → other), one line per entry (name, tokens, truncated description).
/// Lines are fitted to `width` chars (`0` = no limit); descriptions never wrap.
pub fn render_memory_tree(label: &str, entries: &[MemoryEntry], width: usize) -> String {
    let width = if width == 0 { usize::MAX } else { width };
    let mut out = String::new();
    let _ = writeln!(out, "{label} · {}", summarize(entries));
    if entries.is_empty() {
        return out;
    }
    let mut groups: BTreeMap<&MemoryKind, Vec<&MemoryEntry>> = BTreeMap::new();
    for e in entries {
        groups.entry(&e.kind).or_default().push(e);
    }
    for (kind, items) in &groups {
        let group_tokens: usize = items.iter().map(|e| e.tokens).sum();
        let _ = writeln!(out, "  {}  {}", kind.as_tag(), fmt_tokens(group_tokens));
        for e in items {
            let head = format!("    {}  {}", e.name, fmt_tokens(e.tokens));
            let line = if e.description.is_empty() {
                head
            } else {
                let budget = width.saturating_sub(head.chars().count() + 3);
                format!("{head} — {}", truncate_line(&e.description, budget))
            };
            let _ = writeln!(out, "{line}");
        }
    }
    out
}
