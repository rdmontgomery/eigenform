//! eigen-projects: enumerate the Claude Code project directories under
//! `~/.claude/projects/`, recovering each project's original cwd from
//! its session JSONLs.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use thiserror::Error;

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
