//! Where skills live on a real filesystem.

use std::fs;

use eigenform_skills::{canonical_roots, Layer};
use tempfile::tempdir;

#[test]
fn canonical_roots_includes_global_skills_dir() {
    let home = tempdir().unwrap();
    let cwd = tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude/skills")).unwrap();

    let roots = canonical_roots(home.path(), cwd.path());

    assert!(roots
        .iter()
        .any(|(l, p)| matches!(l, Layer::Global) && p == &home.path().join(".claude/skills")));
}

#[test]
fn canonical_roots_includes_repo_skills_dir_even_if_absent() {
    let home = tempdir().unwrap();
    let cwd = tempdir().unwrap();

    let roots = canonical_roots(home.path(), cwd.path());

    assert!(roots
        .iter()
        .any(|(l, p)| matches!(l, Layer::Repo { project: None }) && p == &cwd.path().join(".claude/skills")));
}

#[test]
fn canonical_roots_enumerates_each_plugin_version() {
    let home = tempdir().unwrap();
    let cwd = tempdir().unwrap();

    let pluga = home
        .path()
        .join(".claude/plugins/cache/marketplace-x/superpowers/4.0.3/skills");
    let plugb = home
        .path()
        .join(".claude/plugins/cache/marketplace-y/helper-tool/0.1.0/skills");
    fs::create_dir_all(&pluga).unwrap();
    fs::create_dir_all(&plugb).unwrap();

    let roots = canonical_roots(home.path(), cwd.path());

    let plugin_entries: Vec<_> = roots
        .iter()
        .filter_map(|(l, p)| match l {
            Layer::Plugin { name } => Some((name.clone(), p.clone())),
            _ => None,
        })
        .collect();

    assert!(plugin_entries
        .iter()
        .any(|(n, p)| n == "superpowers" && p == &pluga));
    assert!(plugin_entries
        .iter()
        .any(|(n, p)| n == "helper-tool" && p == &plugb));
    assert_eq!(plugin_entries.len(), 2);
}

#[test]
fn canonical_roots_ordered_global_then_plugins_then_repo() {
    let home = tempdir().unwrap();
    let cwd = tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude/skills")).unwrap();
    fs::create_dir_all(
        home.path()
            .join(".claude/plugins/cache/mp/superpowers/1.0.0/skills"),
    )
    .unwrap();

    let roots = canonical_roots(home.path(), cwd.path());

    // Find indices of the three layer kinds.
    let global_idx = roots
        .iter()
        .position(|(l, _)| matches!(l, Layer::Global))
        .unwrap();
    let plugin_idx = roots
        .iter()
        .position(|(l, _)| matches!(l, Layer::Plugin { .. }))
        .unwrap();
    let repo_idx = roots
        .iter()
        .position(|(l, _)| matches!(l, Layer::Repo { project: None }))
        .unwrap();

    assert!(global_idx < plugin_idx);
    assert!(plugin_idx < repo_idx);
}

#[test]
fn canonical_roots_finds_marketplaces_external_plugins_layout() {
    let home = tempdir().unwrap();
    let cwd = tempdir().unwrap();

    // ~/.claude/plugins/marketplaces/<mp>/external_plugins/<plugin>/skills/
    let p = home
        .path()
        .join(".claude/plugins/marketplaces/claude-plugins-official/external_plugins/discord/skills");
    fs::create_dir_all(&p).unwrap();

    let roots = canonical_roots(home.path(), cwd.path());
    assert!(roots
        .iter()
        .any(|(l, dir)| matches!(l, Layer::Plugin { name } if name == "discord") && dir == &p));
}

#[test]
fn canonical_roots_finds_marketplaces_plugins_layout() {
    let home = tempdir().unwrap();
    let cwd = tempdir().unwrap();

    // ~/.claude/plugins/marketplaces/<mp>/plugins/<plugin>/skills/
    let p = home.path().join(
        ".claude/plugins/marketplaces/claude-plugins-official/plugins/frontend-design/skills",
    );
    fs::create_dir_all(&p).unwrap();

    let roots = canonical_roots(home.path(), cwd.path());
    assert!(roots
        .iter()
        .any(|(l, dir)| matches!(l, Layer::Plugin { name } if name == "frontend-design")
            && dir == &p));
}
