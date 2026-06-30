//! eigenform-inspect: one unified config model with many faces.
//!
//! aleph's `inspect` was a single rich data model rendered three ways; eigenform
//! had skills and memory as independent crates each rendered as a one-shot text
//! dump. This crate is the unified model — it walks the Claude Code resolution
//! layers (Global → Plugin → Repo) and, for each, captures the skills and memory
//! contributed there, every entry annotated with a byte size and a token
//! estimate. The model is data only; projecting it to text / json / html lives in
//! `eigenform-render`, the same multi-renderer pattern the session view uses.
//!
//! Two entry points:
//!   - [`collect`] — one resolution context (a cwd). Shadowing (which skill WINS
//!     across global/plugin/repo) is meaningful here and is computed.
//!   - [`collect_all_projects`] — an inventory across every recorded project.
//!     Cross-project shadowing is undefined (project A's repo can't shadow B's),
//!     so it is not computed; this is a flat inventory.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use eigenform_skills::{Layer, LayeredSkill};

/// The full unified config inventory: an ordered list of resolution layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectData {
    pub layers: Vec<InspectLayer>,
}

impl InspectData {
    /// Total token estimate across every layer.
    pub fn tokens(&self) -> usize {
        self.layers.iter().map(InspectLayer::tokens).sum()
    }
}

/// One resolution layer (e.g. `global`, `plugin:foo`, `repo`, `repo:eigenform`)
/// with the skills and memory it contributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectLayer {
    /// Short label, e.g. `global` / `plugin:foo` / `repo:eigenform`.
    pub label: String,
    pub skills: Vec<SkillItem>,
    pub memory: Vec<MemoryItem>,
}

impl InspectLayer {
    /// Total token estimate for this layer (skills + memory).
    pub fn tokens(&self) -> usize {
        self.skills.iter().map(|s| s.tokens).sum::<usize>()
            + self.memory.iter().map(|m| m.tokens).sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillItem {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub size: usize,
    pub tokens: usize,
    /// Whether this contribution is the one that wins resolution for its name.
    /// Plugin skills are namespaced and always win in their namespace. A bare-name
    /// skill wins iff it is the highest-precedence non-plugin contribution. In
    /// all-projects mode shadowing is undefined, so every item is marked `wins`.
    pub wins: bool,
    /// Whether this name resolves through plugin namespacing (`plugin:<p>:<name>`)
    /// rather than bare-name shadowing — i.e. several plugins, no bare-name claimant.
    pub namespaced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryItem {
    pub name: String,
    pub description: String,
    /// The memory `type` tag (feedback / project / reference / user / …).
    pub kind: String,
    pub path: PathBuf,
    pub size: usize,
    pub tokens: usize,
}

/// Errors collecting the inventory. Skills frontmatter is strictly parsed (a
/// malformed SKILL.md is surfaced, not hidden); memory parse failures are skipped
/// by the memory scanner itself.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Skills(#[from] eigenform_skills::Error),
    #[error(transparent)]
    Memory(#[from] eigenform_memory::Error),
    #[error(transparent)]
    Projects(#[from] eigenform_projects::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Collect the unified inventory for a single resolution context: the canonical
/// skill stack (`<home>/.claude/skills`, plugin trees, `<cwd>/.claude/skills`)
/// plus the auto-memory recorded for the project at `cwd`. Shadowing is computed.
pub fn collect(home: &Path, cwd: &Path) -> Result<InspectData> {
    let roots = eigenform_skills::canonical_roots(home, cwd);
    let all = eigenform_skills::scan_many(&roots)?;
    let shadow = ShadowMap::compute(&all);

    let mut layers: Vec<InspectLayer> = Vec::new();
    for (layer, dir) in &roots {
        let scanned = eigenform_skills::scan_layered(layer.clone(), dir)?;
        layers.push(InspectLayer {
            label: eigenform_skills::layer_label(layer),
            skills: scanned.iter().map(|ls| shadow.item(ls)).collect(),
            memory: Vec::new(),
        });
    }

    // Attach this project's auto-memory to the repo layer (the cwd context).
    // A missing projects dir just means no memory has been recorded — not an error.
    let projects_dir = home.join(".claude/projects");
    if projects_dir.exists() {
        let memory = memory_for_cwd(&projects_dir, cwd)?;
        if !memory.is_empty() {
            attach_memory(&mut layers, &Layer::Repo { project: None }, &roots, memory);
        }
    }

    Ok(InspectData { layers })
}

/// Collect a flat inventory across every recorded project: shared global + plugin
/// layers once, then one `repo:<project>` layer per project carrying that
/// project's skills and memory. Cross-project shadowing is undefined, so every
/// skill is marked `wins` (plugin namespacing is still resolved).
pub fn collect_all_projects(home: &Path) -> Result<InspectData> {
    let projects_dir = home.join(".claude/projects");
    let projects = if projects_dir.exists() {
        eigenform_projects::enumerate_projects(&projects_dir)?
    } else {
        Vec::new()
    };
    let cwds: Vec<PathBuf> = projects.iter().map(|p| p.cwd.clone()).collect();
    let roots = eigenform_skills::all_projects_roots(home, &cwds);

    // No cross-project shadowing: only plugin namespacing is context-independent.
    let all = eigenform_skills::scan_many(&roots)?;
    let shadow = ShadowMap::compute_namespacing_only(&all);

    let mut layers: Vec<InspectLayer> = Vec::new();
    for (layer, dir) in &roots {
        let scanned = eigenform_skills::scan_layered(layer.clone(), dir)?;
        layers.push(InspectLayer {
            label: eigenform_skills::layer_label(layer),
            skills: scanned.iter().map(|ls| shadow.item(ls)).collect(),
            memory: Vec::new(),
        });
    }

    // Attach each project's memory to its repo layer.
    for project in &projects {
        let memory_dir = projects_dir.join(&project.dir_name).join("memory");
        let entries = eigenform_memory::scan_memory_dir(&memory_dir)?;
        if entries.is_empty() {
            continue;
        }
        let target = Layer::Repo {
            project: Some(project.cwd.clone()),
        };
        attach_memory(&mut layers, &target, &roots, into_memory_items(entries));
    }

    Ok(InspectData { layers })
}

/// Resolve the memory entries for the project whose recovered cwd is `cwd`.
/// Returns an empty Vec if no project matches (no error — the cwd may simply
/// never have run Claude Code).
fn memory_for_cwd(projects_dir: &Path, cwd: &Path) -> Result<Vec<MemoryItem>> {
    let Some(project) = eigenform_projects::project_for_cwd(projects_dir, cwd)? else {
        return Ok(Vec::new());
    };
    let memory_dir = projects_dir.join(&project.dir_name).join("memory");
    Ok(into_memory_items(eigenform_memory::scan_memory_dir(&memory_dir)?))
}

fn into_memory_items(entries: Vec<eigenform_memory::MemoryEntry>) -> Vec<MemoryItem> {
    entries
        .into_iter()
        .map(|e| MemoryItem {
            name: e.name,
            description: e.description,
            kind: e.kind.as_tag().to_string(),
            path: e.source_path,
            size: e.size,
            tokens: e.tokens,
        })
        .collect()
}

/// Attach `memory` to the layer in `layers` matching `target`. The layer is
/// located by its label (derived from `target`); `roots` is unused beyond
/// confirming the target is a real slot. If the layer is missing (no skills dir
/// created it), one is appended so the memory is never silently dropped.
fn attach_memory(
    layers: &mut Vec<InspectLayer>,
    target: &Layer,
    _roots: &[(Layer, PathBuf)],
    memory: Vec<MemoryItem>,
) {
    let label = eigenform_skills::layer_label(target);
    if let Some(layer) = layers.iter_mut().find(|l| l.label == label) {
        layer.memory = memory;
    } else {
        layers.push(InspectLayer {
            label,
            skills: Vec::new(),
            memory,
        });
    }
}

/// Precomputed shadowing: which `(name → winning source path)` and which names
/// resolve via plugin namespacing. Mirrors the logic in `skills::render_tree` so
/// the tree and the unified model agree on who wins.
struct ShadowMap {
    /// name → source path of the winning bare-name (non-plugin) contribution.
    winners: HashMap<String, PathBuf>,
    /// names that resolve via plugin namespacing (all-plugin, more than one).
    namespaced: BTreeSet<String>,
}

impl ShadowMap {
    fn compute(all: &[LayeredSkill]) -> ShadowMap {
        let mut by_name: HashMap<&str, Vec<&LayeredSkill>> = HashMap::new();
        for ls in all {
            by_name.entry(ls.skill.name.as_str()).or_default().push(ls);
        }
        let mut winners = HashMap::new();
        let mut namespaced = BTreeSet::new();
        for (name, contribs) in &by_name {
            let non_plugin: Vec<&&LayeredSkill> = contribs
                .iter()
                .filter(|ls| !matches!(ls.layer, Layer::Plugin { .. }))
                .collect();
            let plugin_count = contribs.len() - non_plugin.len();
            if non_plugin.is_empty() && plugin_count > 1 {
                namespaced.insert((*name).to_string());
            }
            // The highest-precedence bare-name contribution wins (scan order is
            // global → plugin → repo, so the last non-plugin is the winner).
            if let Some(winner) = non_plugin.last() {
                winners.insert((*name).to_string(), winner.skill.source_path.clone());
            }
        }
        ShadowMap { winners, namespaced }
    }

    /// Namespacing without shadowing: every bare-name contribution is its own
    /// winner (used for the all-projects inventory, where cross-project
    /// shadowing is undefined).
    fn compute_namespacing_only(all: &[LayeredSkill]) -> ShadowMap {
        let mut by_name: HashMap<&str, Vec<&LayeredSkill>> = HashMap::new();
        for ls in all {
            by_name.entry(ls.skill.name.as_str()).or_default().push(ls);
        }
        let mut namespaced = BTreeSet::new();
        for (name, contribs) in &by_name {
            let non_plugin = contribs
                .iter()
                .filter(|ls| !matches!(ls.layer, Layer::Plugin { .. }))
                .count();
            let plugin_count = contribs.len() - non_plugin;
            if non_plugin == 0 && plugin_count > 1 {
                namespaced.insert((*name).to_string());
            }
        }
        ShadowMap {
            winners: HashMap::new(),
            namespaced,
        }
    }

    /// Project a `LayeredSkill` into a `SkillItem`, stamping its `wins` /
    /// `namespaced` flags from the precomputed maps.
    fn item(&self, ls: &LayeredSkill) -> SkillItem {
        let is_plugin = matches!(ls.layer, Layer::Plugin { .. });
        // Plugins always win in their namespace. A bare-name skill wins iff it is
        // the recorded winner for its name. With no winner recorded (namespacing-
        // only mode), bare names default to winning — there is nothing to shadow.
        let wins = is_plugin
            || match self.winners.get(&ls.skill.name) {
                Some(path) => *path == ls.skill.source_path,
                None => true,
            };
        SkillItem {
            name: ls.skill.name.clone(),
            description: ls.skill.description.clone(),
            path: ls.skill.source_path.clone(),
            size: ls.skill.size,
            tokens: ls.skill.tokens,
            wins,
            namespaced: self.namespaced.contains(&ls.skill.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Write a SKILL.md with the given name/description into `dir/<name>/SKILL.md`.
    fn write_skill(dir: &Path, name: &str, desc: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\nbody\n"),
        )
        .unwrap();
    }

    /// Write a memory entry into `projects_dir/<dir_name>/memory/<name>.md`, and a
    /// session JSONL recording `cwd` so the project is discoverable.
    fn write_project(projects_dir: &Path, dir_name: &str, cwd: &Path) {
        let proj = projects_dir.join(dir_name);
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            proj.join("session.jsonl"),
            format!("{{\"cwd\":\"{}\"}}\n", cwd.display()),
        )
        .unwrap();
    }

    fn write_memory(projects_dir: &Path, dir_name: &str, name: &str, kind: &str) {
        let mem = projects_dir.join(dir_name).join("memory");
        fs::create_dir_all(&mem).unwrap();
        fs::write(
            mem.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: a {kind} note\ntype: {kind}\n---\nbody\n"),
        )
        .unwrap();
    }

    #[test]
    fn collect_marks_repo_skill_as_winner_over_global() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        write_skill(&home.path().join(".claude/skills"), "review", "global review");
        write_skill(&cwd.path().join(".claude/skills"), "review", "repo review");

        let data = collect(home.path(), cwd.path()).unwrap();

        let global = data.layers.iter().find(|l| l.label == "global").unwrap();
        let repo = data.layers.iter().find(|l| l.label == "repo").unwrap();
        let g = &global.skills[0];
        let r = &repo.skills[0];
        assert_eq!(g.name, "review");
        assert!(!g.wins, "global is shadowed by repo");
        assert!(r.wins, "repo wins");
        assert!(g.tokens > 0, "token estimate is populated");
    }

    #[test]
    fn collect_attaches_project_memory_to_repo_layer() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let projects_dir = home.path().join(".claude/projects");
        write_project(&projects_dir, "proj", cwd.path());
        write_memory(&projects_dir, "proj", "auth-flow", "project");

        let data = collect(home.path(), cwd.path()).unwrap();
        let repo = data.layers.iter().find(|l| l.label == "repo").unwrap();
        assert_eq!(repo.memory.len(), 1);
        assert_eq!(repo.memory[0].name, "auth-flow");
        assert_eq!(repo.memory[0].kind, "project");
        assert!(repo.memory[0].tokens > 0);
    }

    #[test]
    fn total_tokens_sum_over_layers() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        write_skill(&home.path().join(".claude/skills"), "a", "skill a");
        write_skill(&cwd.path().join(".claude/skills"), "b", "skill b");

        let data = collect(home.path(), cwd.path()).unwrap();
        let layer_sum: usize = data.layers.iter().map(|l| l.tokens()).sum();
        assert_eq!(data.tokens(), layer_sum);
        assert!(data.tokens() > 0);
    }

    #[test]
    fn all_projects_does_not_shadow_across_projects() {
        let home = tempfile::tempdir().unwrap();
        let projects_dir = home.path().join(".claude/projects");
        // Two projects each with a same-named repo skill — neither shadows the other.
        let cwd_a = tempfile::tempdir().unwrap();
        let cwd_b = tempfile::tempdir().unwrap();
        write_project(&projects_dir, "a", cwd_a.path());
        write_project(&projects_dir, "b", cwd_b.path());
        write_skill(&cwd_a.path().join(".claude/skills"), "dup", "from a");
        write_skill(&cwd_b.path().join(".claude/skills"), "dup", "from b");

        let data = collect_all_projects(home.path()).unwrap();
        let dup_items: Vec<&SkillItem> = data
            .layers
            .iter()
            .flat_map(|l| l.skills.iter())
            .filter(|s| s.name == "dup")
            .collect();
        assert_eq!(dup_items.len(), 2);
        assert!(dup_items.iter().all(|s| s.wins), "no cross-project shadowing");
    }
}
