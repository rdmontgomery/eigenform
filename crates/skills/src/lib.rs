//! eigen-skills: scan and reason about Claude Code skill directories.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
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
    #[error("missing frontmatter in {path}")]
    MissingFrontmatter { path: PathBuf },
    #[error("invalid yaml frontmatter in {path}: {source}")]
    InvalidFrontmatter {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
}

pub fn scan_dir(dir: &Path) -> Result<Vec<Skill>> {
    let mut out = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| Error::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let body = fs::read_to_string(&path).map_err(|e| Error::Io {
            path: path.clone(),
            source: e,
        })?;
        let fm = parse_frontmatter(&body, &path)?;
        out.push(Skill {
            name: fm.name,
            description: fm.description,
            source_path: path,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn parse_frontmatter(body: &str, path: &Path) -> Result<Frontmatter> {
    let rest = body
        .strip_prefix("---\n")
        .ok_or_else(|| Error::MissingFrontmatter {
            path: path.to_path_buf(),
        })?;
    let (yaml, _body) = rest
        .split_once("\n---")
        .ok_or_else(|| Error::MissingFrontmatter {
            path: path.to_path_buf(),
        })?;
    serde_yaml::from_str(yaml).map_err(|e| Error::InvalidFrontmatter {
        path: path.to_path_buf(),
        source: e,
    })
}
