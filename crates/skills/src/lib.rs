//! eigenform-skills: scan and reason about Claude Code skill directories.

use std::collections::BTreeMap;
use std::fmt::Write as _;
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

/// Which level of the skill resolution hierarchy a skill came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer {
    Global,
    Plugin {
        name: String,
    },
    /// A project's local `.claude/skills/`. `project: None` means "the
    /// current cwd"; `project: Some(cwd)` is used when scanning multiple
    /// projects, so the rendered output can distinguish them.
    Repo {
        project: Option<PathBuf>,
    },
    Cwd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredSkill {
    pub layer: Layer,
    pub skill: Skill,
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
        let file_type = entry.file_type().map_err(|e| Error::Io {
            path: path.clone(),
            source: e,
        })?;

        let skill_path = if file_type.is_dir() {
            let candidate = path.join("SKILL.md");
            if candidate.exists() {
                candidate
            } else {
                continue;
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            path.clone()
        } else {
            continue;
        };

        let body = fs::read_to_string(&skill_path).map_err(|e| Error::Io {
            path: skill_path.clone(),
            source: e,
        })?;
        let fm = parse_frontmatter(&body, &skill_path)?;
        out.push(Skill {
            name: fm.name,
            description: fm.description,
            source_path: skill_path,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Like `scan_dir`, but tag every result with its layer.
/// Missing directories are treated as empty (no error).
pub fn scan_layered(layer: Layer, dir: &Path) -> Result<Vec<LayeredSkill>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let skills = scan_dir(dir)?;
    Ok(skills
        .into_iter()
        .map(|s| LayeredSkill {
            layer: layer.clone(),
            skill: s,
        })
        .collect())
}

/// Scan an ordered list of (layer, dir) roots and return the concatenation,
/// preserving input order. Missing directories are skipped silently.
pub fn scan_many(roots: &[(Layer, PathBuf)]) -> Result<Vec<LayeredSkill>> {
    let mut out = Vec::new();
    for (layer, dir) in roots {
        out.extend(scan_layered(layer.clone(), dir)?);
    }
    Ok(out)
}

/// The canonical skill resolution stack for a given `home` and `cwd`:
///   Global   = `<home>/.claude/skills/`
///   Plugin   = each `<home>/.claude/plugins/cache/<marketplace>/<plugin>/<version>/skills/`
///              (sorted by plugin name for determinism)
///   Repo     = `<cwd>/.claude/skills/`
///
/// Repo is included even if absent (so consumers see the slot in the override
/// stack). Plugin entries are only included for plugin trees that exist.
pub fn canonical_roots(home: &Path, cwd: &Path) -> Vec<(Layer, PathBuf)> {
    let mut roots: Vec<(Layer, PathBuf)> = Vec::new();
    roots.push((Layer::Global, home.join(".claude/skills")));

    let mut plugin_entries: Vec<(String, PathBuf)> = Vec::new();
    let cache = home.join(".claude/plugins/cache");
    if let Ok(marketplaces) = fs::read_dir(&cache) {
        for marketplace in marketplaces.flatten() {
            let mp_path = marketplace.path();
            if !mp_path.is_dir() {
                continue;
            }
            if let Ok(plugins) = fs::read_dir(&mp_path) {
                for plugin in plugins.flatten() {
                    let plugin_path = plugin.path();
                    if !plugin_path.is_dir() {
                        continue;
                    }
                    let plugin_name = match plugin_path.file_name().and_then(|n| n.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    if let Ok(versions) = fs::read_dir(&plugin_path) {
                        for version in versions.flatten() {
                            let skills_dir = version.path().join("skills");
                            if skills_dir.is_dir() {
                                plugin_entries.push((plugin_name.clone(), skills_dir));
                            }
                        }
                    }
                }
            }
        }
    }
    let marketplaces_root = home.join(".claude/plugins/marketplaces");
    if let Ok(mps) = fs::read_dir(&marketplaces_root) {
        for mp in mps.flatten() {
            let mp_path = mp.path();
            if !mp_path.is_dir() {
                continue;
            }
            for kind in ["external_plugins", "plugins"] {
                let kind_dir = mp_path.join(kind);
                let Ok(plugins) = fs::read_dir(&kind_dir) else {
                    continue;
                };
                for plugin in plugins.flatten() {
                    let plugin_path = plugin.path();
                    if !plugin_path.is_dir() {
                        continue;
                    }
                    let plugin_name = match plugin_path.file_name().and_then(|n| n.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    let skills_dir = plugin_path.join("skills");
                    if skills_dir.is_dir() {
                        plugin_entries.push((plugin_name, skills_dir));
                    }
                }
            }
        }
    }

    plugin_entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for (name, path) in plugin_entries {
        roots.push((Layer::Plugin { name }, path));
    }

    roots.push((Layer::Repo { project: None }, cwd.join(".claude/skills")));
    roots
}

/// Like `canonical_roots`, but emits one `Layer::Repo { project: Some(cwd) }`
/// entry per supplied project cwd. Global + plugin layers are included once
/// (they're shared across all projects). Repo entries appear in the supplied
/// order, after global and plugin.
pub fn all_projects_roots(home: &Path, project_cwds: &[PathBuf]) -> Vec<(Layer, PathBuf)> {
    // Use a throwaway cwd so we get the shared global+plugin entries,
    // then strip the dummy Repo at the end and re-emit one per project.
    let dummy = std::path::PathBuf::from("/");
    let mut shared = canonical_roots(home, &dummy);
    // Drop the trailing Repo { project: None } slot.
    if matches!(shared.last(), Some((Layer::Repo { project: None }, _))) {
        shared.pop();
    }
    for cwd in project_cwds {
        shared.push((
            Layer::Repo {
                project: Some(cwd.clone()),
            },
            cwd.join(".claude/skills"),
        ));
    }
    shared
}

/// Render a layered scan as a text tree: skills grouped by name (alphabetic),
/// contributions listed in precedence order, with a `WINS` marker on any
/// name that has more than one contribution.
pub fn render_tree(scan: &[LayeredSkill]) -> String {
    let mut out = String::from("SKILLS\n======\n\n");
    if scan.is_empty() {
        out.push_str("(no skills found)\n");
        return out;
    }
    let mut groups: BTreeMap<&str, Vec<&LayeredSkill>> = BTreeMap::new();
    for ls in scan {
        groups.entry(ls.skill.name.as_str()).or_default().push(ls);
    }
    for (name, contribs) in &groups {
        let non_plugin: Vec<&&LayeredSkill> = contribs
            .iter()
            .filter(|ls| !matches!(ls.layer, Layer::Plugin { .. }))
            .collect();
        let plugin_count = contribs.len() - non_plugin.len();
        // Plugins are namespaced (`plugin:<plug>:<skill>`) — multiple plugin
        // contributions to the same name coexist, they don't shadow.
        let namespaced = non_plugin.is_empty() && plugin_count > 1;
        if namespaced {
            let _ = writeln!(out, "{name}  (namespaced)");
        } else {
            let _ = writeln!(out, "{name}");
        }
        for ls in contribs {
            let _ = writeln!(
                out,
                "  [{}]  {}",
                layer_tag(&ls.layer),
                ls.skill.source_path.display()
            );
        }
        // Real shadowing only happens between non-namespaced (bare-name)
        // contributions: bundled, global, repo, cwd.
        if non_plugin.len() > 1 {
            let winner = non_plugin.last().unwrap();
            let _ = writeln!(out, "  -> WINS: [{}]", layer_tag(&winner.layer));
        }
        out.push('\n');
    }
    out
}

fn layer_tag(l: &Layer) -> String {
    match l {
        Layer::Global => "global".into(),
        Layer::Plugin { name } => format!("plugin:{name}"),
        Layer::Repo { project: None } => "repo".into(),
        Layer::Repo {
            project: Some(cwd),
        } => {
            let label = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?");
            format!("repo:{label}")
        }
        Layer::Cwd => "cwd".into(),
    }
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
