//! RED tests for layered scanning + override-stack resolution.

use std::fs;
use std::path::PathBuf;

use eigenform_skills::{scan_layered, scan_many, Layer};
use tempfile::tempdir;

fn write_skill(dir: &std::path::Path, fname: &str, name: &str) {
    fs::write(
        dir.join(fname),
        format!("---\nname: {name}\ndescription: {name} desc\n---\nbody\n"),
    )
    .unwrap();
}

#[test]
fn scan_layered_tags_skills_with_layer() {
    let dir = tempdir().unwrap();
    write_skill(dir.path(), "a.md", "a");

    let found = scan_layered(Layer::Global, dir.path()).unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].layer, Layer::Global);
    assert_eq!(found[0].skill.name, "a");
}

#[test]
fn scan_layered_distinguishes_plugin_layer_by_name() {
    let dir = tempdir().unwrap();
    write_skill(dir.path(), "b.md", "b");

    let layer = Layer::Plugin {
        name: "superpowers".into(),
    };
    let found = scan_layered(layer.clone(), dir.path()).unwrap();
    assert_eq!(found[0].layer, layer);
}

#[test]
fn scan_layered_missing_dir_returns_empty_not_error() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("nonexistent");

    let found = scan_layered(Layer::Repo { project: None }, &missing).unwrap();
    assert!(found.is_empty());
}

#[test]
fn scan_many_concatenates_in_supplied_order() {
    let g = tempdir().unwrap();
    let r = tempdir().unwrap();
    write_skill(g.path(), "x.md", "x"); // global
    write_skill(r.path(), "y.md", "y"); // repo

    let roots: Vec<(Layer, PathBuf)> = vec![
        (Layer::Global, g.path().to_path_buf()),
        (Layer::Repo { project: None }, r.path().to_path_buf()),
    ];
    let all = scan_many(&roots).unwrap();

    assert_eq!(all.len(), 2);
    assert_eq!(all[0].layer, Layer::Global);
    assert_eq!(all[0].skill.name, "x");
    assert_eq!(all[1].layer, Layer::Repo { project: None });
    assert_eq!(all[1].skill.name, "y");
}

#[test]
fn scan_many_handles_missing_dirs_gracefully() {
    let g = tempdir().unwrap();
    write_skill(g.path(), "x.md", "x");

    let roots: Vec<(Layer, PathBuf)> = vec![
        (Layer::Global, g.path().to_path_buf()),
        (Layer::Repo { project: None }, g.path().join("does/not/exist")),
    ];
    let all = scan_many(&roots).unwrap();

    assert_eq!(all.len(), 1);
    assert_eq!(all[0].layer, Layer::Global);
}
