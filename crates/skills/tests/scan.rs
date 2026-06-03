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
