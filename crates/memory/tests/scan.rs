use std::fs;
use std::path::PathBuf;

use eigenform_memory::{scan_memory_dir, MemoryKind};
use tempfile::tempdir;

fn write_entry(dir: &std::path::Path, fname: &str, name: &str, kind: &str) -> PathBuf {
    let path = dir.join(fname);
    fs::write(
        &path,
        format!(
            "---\nname: {name}\ndescription: about {name}\ntype: {kind}\n---\n\nbody\n"
        ),
    )
    .unwrap();
    path
}

#[test]
fn scan_returns_single_memory_entry() {
    let dir = tempdir().unwrap();
    let path = write_entry(dir.path(), "user_role.md", "user role", "user");

    let entries = scan_memory_dir(dir.path()).unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "user role");
    assert_eq!(entries[0].description, "about user role");
    assert_eq!(entries[0].kind, MemoryKind::User);
    assert_eq!(entries[0].source_path, path);
}

#[test]
fn scan_recognises_all_four_known_kinds() {
    let dir = tempdir().unwrap();
    write_entry(dir.path(), "f.md", "f", "feedback");
    write_entry(dir.path(), "p.md", "p", "project");
    write_entry(dir.path(), "r.md", "r", "reference");
    write_entry(dir.path(), "u.md", "u", "user");

    let kinds: Vec<MemoryKind> = scan_memory_dir(dir.path())
        .unwrap()
        .into_iter()
        .map(|m| m.kind)
        .collect();

    assert!(kinds.contains(&MemoryKind::Feedback));
    assert!(kinds.contains(&MemoryKind::Project));
    assert!(kinds.contains(&MemoryKind::Reference));
    assert!(kinds.contains(&MemoryKind::User));
}

#[test]
fn scan_categorises_unknown_kind_as_other() {
    let dir = tempdir().unwrap();
    write_entry(dir.path(), "weird.md", "weird", "weird-kind");

    let entries = scan_memory_dir(dir.path()).unwrap();
    assert_eq!(entries[0].kind, MemoryKind::Other("weird-kind".into()));
}

#[test]
fn scan_skips_memory_md_index_file() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("MEMORY.md"),
        "- [a](a.md) - hook\n- [b](b.md) - hook\n",
    )
    .unwrap();
    write_entry(dir.path(), "a.md", "a", "feedback");

    let entries = scan_memory_dir(dir.path()).unwrap();
    let names: Vec<&str> = entries.iter().map(|m| m.name.as_str()).collect();

    assert_eq!(names, vec!["a"]);
}

#[test]
fn scan_skips_files_without_frontmatter() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("loose.md"), "no frontmatter at all").unwrap();
    write_entry(dir.path(), "valid.md", "valid", "user");

    let entries = scan_memory_dir(dir.path()).unwrap();
    let names: Vec<&str> = entries.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["valid"]);
}

#[test]
fn scan_missing_dir_returns_empty_not_error() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("memory");
    let entries = scan_memory_dir(&missing).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn scan_sorts_by_kind_then_name() {
    let dir = tempdir().unwrap();
    write_entry(dir.path(), "u_z.md", "zelda", "user");
    write_entry(dir.path(), "u_a.md", "annie", "user");
    write_entry(dir.path(), "f_a.md", "alpha", "feedback");

    let entries = scan_memory_dir(dir.path()).unwrap();
    let labels: Vec<(MemoryKind, String)> =
        entries.into_iter().map(|m| (m.kind, m.name)).collect();

    // Order: feedback < project < reference < user; ties broken by name.
    assert_eq!(
        labels,
        vec![
            (MemoryKind::Feedback, "alpha".into()),
            (MemoryKind::User, "annie".into()),
            (MemoryKind::User, "zelda".into()),
        ]
    );
}
