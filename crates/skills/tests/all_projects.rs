use std::fs;
use std::path::PathBuf;

use eigenform_skills::{all_projects_roots, Layer};
use tempfile::tempdir;

#[test]
fn all_projects_roots_emits_one_repo_entry_per_project_cwd() {
    let home = tempdir().unwrap();
    let cwds = vec![
        PathBuf::from("/tmp/proj-a"),
        PathBuf::from("/tmp/proj-b"),
    ];

    let roots = all_projects_roots(home.path(), &cwds);

    let repo_entries: Vec<_> = roots
        .iter()
        .filter_map(|(l, p)| match l {
            Layer::Repo { project: Some(cwd) } => Some((cwd.clone(), p.clone())),
            _ => None,
        })
        .collect();

    assert_eq!(repo_entries.len(), 2);
    assert!(repo_entries
        .iter()
        .any(|(cwd, p)| cwd == &PathBuf::from("/tmp/proj-a")
            && p == &PathBuf::from("/tmp/proj-a/.claude/skills")));
    assert!(repo_entries
        .iter()
        .any(|(cwd, p)| cwd == &PathBuf::from("/tmp/proj-b")
            && p == &PathBuf::from("/tmp/proj-b/.claude/skills")));
}

#[test]
fn all_projects_roots_includes_global_and_plugin_layers_once() {
    let home = tempdir().unwrap();
    fs::create_dir_all(home.path().join(".claude/skills")).unwrap();
    fs::create_dir_all(
        home.path()
            .join(".claude/plugins/cache/mp/somethingelse/1.0.0/skills"),
    )
    .unwrap();

    let cwds = vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")];
    let roots = all_projects_roots(home.path(), &cwds);

    let global_count = roots
        .iter()
        .filter(|(l, _)| matches!(l, Layer::Global))
        .count();
    let plugin_count = roots
        .iter()
        .filter(|(l, _)| matches!(l, Layer::Plugin { .. }))
        .count();

    assert_eq!(global_count, 1);
    assert_eq!(plugin_count, 1);
}

#[test]
fn all_projects_roots_drops_dummy_current_repo_slot() {
    let home = tempdir().unwrap();
    let cwds: Vec<PathBuf> = vec![];

    let roots = all_projects_roots(home.path(), &cwds);

    assert!(
        roots
            .iter()
            .all(|(l, _)| !matches!(l, Layer::Repo { project: None })),
        "no current-cwd Repo slot should leak in: {:?}",
        roots
    );
}
