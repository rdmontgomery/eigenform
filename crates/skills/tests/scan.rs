//! RED tests: each defines a behavior we want from `eigen_skills`.
//! Order roughly mirrors implementation order.

use std::fs;
use tempfile::tempdir;

#[test]
fn scan_returns_single_skill_from_directory_with_one_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("my-skill.md");
    fs::write(
        &path,
        "---\nname: my-skill\ndescription: a test skill\n---\n\nbody here",
    )
    .unwrap();

    let found = eigen_skills::scan_dir(dir.path()).unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "my-skill");
    assert_eq!(found[0].description, "a test skill");
    assert_eq!(found[0].source_path, path);
}

#[test]
fn scan_returns_multiple_skills_sorted_by_name() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("zeta.md"),
        "---\nname: zeta\ndescription: z\n---\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("alpha.md"),
        "---\nname: alpha\ndescription: a\n---\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("mid.md"),
        "---\nname: mid\ndescription: m\n---\n",
    )
    .unwrap();

    let names: Vec<String> = eigen_skills::scan_dir(dir.path())
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();

    assert_eq!(names, vec!["alpha", "mid", "zeta"]);
}

#[test]
fn scan_finds_skill_in_subdir_skill_md_layout() {
    let dir = tempdir().unwrap();
    let sub = dir.path().join("brainstorming");
    fs::create_dir(&sub).unwrap();
    let skill_path = sub.join("SKILL.md");
    fs::write(
        &skill_path,
        "---\nname: brainstorming\ndescription: nested\n---\nbody",
    )
    .unwrap();

    let found = eigen_skills::scan_dir(dir.path()).unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "brainstorming");
    assert_eq!(found[0].source_path, skill_path);
}

#[test]
fn scan_mixes_flat_and_nested_layouts_in_one_dir() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("flat.md"),
        "---\nname: flat\ndescription: f\n---\n",
    )
    .unwrap();
    let sub = dir.path().join("nested");
    fs::create_dir(&sub).unwrap();
    fs::write(
        sub.join("SKILL.md"),
        "---\nname: nested\ndescription: n\n---\n",
    )
    .unwrap();

    let names: Vec<String> = eigen_skills::scan_dir(dir.path())
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();

    assert_eq!(names, vec!["flat", "nested"]);
}

#[test]
fn scan_ignores_random_non_skill_files_and_dirs() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("README.txt"), "not a skill").unwrap();
    fs::write(
        dir.path().join("real.md"),
        "---\nname: real\ndescription: r\n---\n",
    )
    .unwrap();
    let empty_sub = dir.path().join("empty");
    fs::create_dir(&empty_sub).unwrap();
    let stray_sub = dir.path().join("stray");
    fs::create_dir(&stray_sub).unwrap();
    fs::write(stray_sub.join("notes.md"), "no frontmatter").unwrap();

    let names: Vec<String> = eigen_skills::scan_dir(dir.path())
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();

    assert_eq!(names, vec!["real"]);
}
