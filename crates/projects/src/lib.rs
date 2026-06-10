//! eigen-projects: enumerate the Claude Code project directories under
//! `~/.claude/projects/`, recovering each project's original cwd from
//! its session JSONLs.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use thiserror::Error;

/// A new-session directory candidate: a path the author might start `claude` in,
/// tagged by whether it's a recent session cwd (`recent: true`) or merely an
/// unvisited immediate subdirectory of the configured code root (`recent: false`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub recent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// The on-disk name of the project subdirectory, e.g.
    /// `-home-rdmontgomery-projects-eigen`.
    pub dir_name: String,
    /// The original cwd as recovered from the project's JSONL files.
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

/// Walk a Claude Code projects directory (e.g. `~/.claude/projects/`) and
/// return one `Project` per subdirectory whose JSONL files record a cwd.
///
/// Subdirectories with no .jsonl files, or whose .jsonl files lack a cwd
/// field, are skipped silently — they're not currently usable.
pub fn enumerate_projects(projects_dir: &Path) -> Result<Vec<Project>> {
    let entries = fs::read_dir(projects_dir).map_err(|e| Error::Io {
        path: projects_dir.to_path_buf(),
        source: e,
    })?;

    let mut out: Vec<Project> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if let Some(cwd) = first_cwd_in_dir(&path)? {
            out.push(Project { dir_name, cwd });
        }
    }

    out.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    Ok(out)
}

/// Find the enumerated project whose recovered cwd equals `cwd`.
/// Returns Ok(None) if no project matches.
pub fn project_for_cwd(projects_dir: &Path, cwd: &Path) -> Result<Option<Project>> {
    let projects = enumerate_projects(projects_dir)?;
    Ok(projects.into_iter().find(|p| p.cwd == cwd))
}

/// Return the immediate subdirectories of `root`, sorted by path. Only
/// directories are returned; loose files are ignored. Errors if `root`
/// cannot be read (e.g. it does not exist).
pub fn immediate_subdirs(root: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(root).map_err(|e| Error::Io {
        path: root.to_path_buf(),
        source: e,
    })?;

    let mut out: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Merge recent session cwds with the immediate subdirs of the code root into
/// one candidate list: recents first (in order, de-duplicated), then any
/// subdir not already present as a recent, each tagged `recent: false`.
pub fn merge_candidates(recents: &[PathBuf], subdirs: &[PathBuf]) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: std::collections::HashSet<&PathBuf> = std::collections::HashSet::new();

    for path in recents {
        if seen.insert(path) {
            out.push(Candidate {
                path: path.clone(),
                recent: true,
            });
        }
    }
    for path in subdirs {
        if seen.insert(path) {
            out.push(Candidate {
                path: path.clone(),
                recent: false,
            });
        }
    }
    out
}

fn first_cwd_in_dir(dir: &Path) -> Result<Option<PathBuf>> {
    let entries = fs::read_dir(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    let mut jsonls: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            jsonls.push(path);
        }
    }
    // Stable order: alphabetic by filename so tests are deterministic.
    jsonls.sort();

    for jsonl in jsonls {
        if let Some(cwd) = first_cwd_in_jsonl(&jsonl)? {
            return Ok(Some(cwd));
        }
    }
    Ok(None)
}

fn first_cwd_in_jsonl(path: &Path) -> Result<Option<PathBuf>> {
    let file = fs::File::open(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line.map_err(|e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(cwd) = value.get("cwd").and_then(|v| v.as_str()) {
                return Ok(Some(PathBuf::from(cwd)));
            }
        }
    }
    Ok(None)
}
