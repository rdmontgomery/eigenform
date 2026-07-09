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
    /// Size of the source file in bytes.
    pub size: usize,
    /// Rough token estimate for the source file (see [`estimate_tokens`]).
    pub tokens: usize,
}

/// A coarse token estimate: ~4 bytes/token, the rule of thumb for English text
/// under the Claude tokenizer. Deliberately cheap and dependency-free — this is
/// a budgeting aid for a context-surgery tool, not a billing oracle.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Format a token count for display: `~N tok` under 1k, `~N.Nk tok` above. The
/// `~` is a standing reminder that this is an estimate, not a measured count.
pub fn fmt_tokens(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("~{:.1}k tok", tokens as f64 / 1000.0)
    } else {
        format!("~{tokens} tok")
    }
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
            size: body.len(),
            tokens: estimate_tokens(&body),
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

/// Presentation options for [`render_tree`].
#[derive(Debug, Default, Clone)]
pub struct RenderOpts {
    /// Maximum line width; lines are truncated/elided to fit, never wrapped.
    /// `0` means "no limit".
    pub width: usize,
    /// When set, paths under this prefix render as `~/…`.
    pub home: Option<PathBuf>,
    /// Extra context prepended to the summary line (e.g. `41 projects`).
    pub note: Option<String>,
}

impl RenderOpts {
    fn width_or_max(&self) -> usize {
        if self.width == 0 { usize::MAX } else { self.width }
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

/// Shorten a path for display: the home prefix becomes `~`, and if the result
/// still exceeds `max` chars it is left-elided (`…/tail/kept/whole`) — the tail
/// is what distinguishes one skill file from another.
fn display_path(path: &Path, home: Option<&Path>, max: usize) -> String {
    let mut s = path.display().to_string();
    if let Some(home) = home {
        if let Ok(rest) = path.strip_prefix(home) {
            s = format!("~/{}", rest.display());
        }
    }
    if s.chars().count() <= max {
        return s;
    }
    // Keep the last `max - 1` chars, then snap forward to a path boundary.
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(max.saturating_sub(1));
    let tail: String = chars[start..].iter().collect();
    let snapped = tail.split_once('/').map(|(_, rest)| rest.to_string()).unwrap_or(tail);
    format!("…/{snapped}")
}

/// Render a layered scan as text: a summary line, then skills grouped by name
/// (alphabetic), each a name header (tokens + resolution note), a one-line
/// truncated description, and its source path(s). Multi-source names list one
/// aligned `layer  tokens  path` row per contribution; shadowing is called out
/// as `<layer> wins` on the header. Lines fit `opts.width`; nothing wraps.
pub fn render_tree(scan: &[LayeredSkill], opts: &RenderOpts) -> String {
    let width = opts.width_or_max();
    let home = opts.home.as_deref();
    let mut out = String::new();

    let mut groups: BTreeMap<&str, Vec<&LayeredSkill>> = BTreeMap::new();
    for ls in scan {
        groups.entry(ls.skill.name.as_str()).or_default().push(ls);
    }

    // Summary: `<note> · N skills · M sources · ~T tok` (sources only when it
    // differs from the name count; note only when the caller supplied one).
    let mut summary = Vec::new();
    if let Some(note) = &opts.note {
        summary.push(note.clone());
    }
    summary.push(format!("{} skill{}", groups.len(), if groups.len() == 1 { "" } else { "s" }));
    if scan.len() != groups.len() {
        summary.push(format!("{} sources", scan.len()));
    }
    if scan.is_empty() {
        summary.push("none found".to_string());
    } else {
        let total: usize = scan.iter().map(|ls| ls.skill.tokens).sum();
        summary.push(fmt_tokens(total));
    }
    let _ = writeln!(out, "{}", summary.join(" · "));
    if scan.is_empty() {
        return out;
    }
    out.push('\n');

    for (name, contribs) in &groups {
        let non_plugin: Vec<&&LayeredSkill> = contribs
            .iter()
            .filter(|ls| !matches!(ls.layer, Layer::Plugin { .. }))
            .collect();
        let plugin_count = contribs.len() - non_plugin.len();
        // Plugins are namespaced (`plugin:<plug>:<skill>`) — multiple plugin
        // contributions to the same name coexist, they don't shadow. Real
        // shadowing only happens between bare-name (non-plugin) contributions.
        let namespaced = non_plugin.is_empty() && plugin_count > 1;
        let group_tokens: usize = contribs.iter().map(|ls| ls.skill.tokens).sum();

        // Header: name, total tokens, and how the name resolves.
        let note = if contribs.len() == 1 {
            layer_tag(&contribs[0].layer)
        } else if namespaced {
            format!("namespaced · {} plugins", plugin_count)
        } else if non_plugin.len() > 1 {
            format!("{} wins", layer_tag(&non_plugin.last().unwrap().layer))
        } else {
            format!("{} sources", contribs.len())
        };
        let _ = writeln!(out, "{name}  {} · {note}", fmt_tokens(group_tokens));

        // One description line under the name (the winning contribution's, or
        // the sole one) — why a reader can tell skills apart at a glance.
        if let Some(desc) = contribs.last().map(|ls| ls.skill.description.as_str()) {
            if !desc.is_empty() {
                let _ = writeln!(out, "  {}", truncate_line(desc, width.saturating_sub(2)));
            }
        }

        if contribs.len() == 1 {
            // Sole source: the layer is already on the header, tokens equal the
            // total — only the path carries new information.
            let path = display_path(&contribs[0].skill.source_path, home, width.saturating_sub(2));
            let _ = writeln!(out, "  {path}");
        } else {
            let tag_w = contribs.iter().map(|ls| layer_tag(&ls.layer).chars().count()).max().unwrap_or(0);
            let tok_w = contribs.iter().map(|ls| fmt_tokens(ls.skill.tokens).chars().count()).max().unwrap_or(0);
            for ls in contribs {
                let tag = layer_tag(&ls.layer);
                let tok = fmt_tokens(ls.skill.tokens);
                let path_max = width.saturating_sub(2 + tag_w + 2 + tok_w + 2);
                let path = display_path(&ls.skill.source_path, home, path_max);
                let _ = writeln!(out, "  {tag:<tag_w$}  {tok:<tok_w$}  {path}");
            }
        }
    }
    out
}

/// A short human label for a layer, e.g. `global`, `plugin:foo`, `repo`,
/// `repo:eigenform`. Used by the text tree and by the unified inspect model.
pub fn layer_label(l: &Layer) -> String {
    layer_tag(l)
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
