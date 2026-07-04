use std::path::PathBuf;

use eigenform_memory::{render_memory_tree, MemoryEntry, MemoryKind};

fn entry(name: &str, kind: MemoryKind, path: &str, desc: &str) -> MemoryEntry {
    MemoryEntry {
        name: name.into(),
        description: desc.into(),
        kind,
        source_path: PathBuf::from(path),
        size: 0,
        tokens: 0,
    }
}

#[test]
fn render_groups_by_kind_in_known_order() {
    let entries = vec![
        entry("z-user", MemoryKind::User, "/u.md", "u"),
        entry("a-fb", MemoryKind::Feedback, "/f.md", "f"),
        entry("c-proj", MemoryKind::Project, "/p.md", "p"),
        entry("d-ref", MemoryKind::Reference, "/r.md", "r"),
    ];
    let out = render_memory_tree(&entries);

    let p_f = out.find("[feedback]").unwrap();
    let p_p = out.find("[project]").unwrap();
    let p_r = out.find("[reference]").unwrap();
    let p_u = out.find("[user]").unwrap();
    assert!(p_f < p_p);
    assert!(p_p < p_r);
    assert!(p_r < p_u);
}

#[test]
fn render_includes_each_entry_name_and_description() {
    let entries = vec![entry(
        "engine invocations",
        MemoryKind::Feedback,
        "/h/feedback_engine_invocations.md",
        "Never run claude -p without authorization",
    )];
    let out = render_memory_tree(&entries);

    assert!(out.contains("engine invocations"));
    assert!(out.contains("Never run claude -p without authorization"));
}

#[test]
fn render_empty_input_says_no_memory() {
    let out = render_memory_tree(&[]);
    assert!(out.contains("MEMORY"));
    assert!(out.contains("(no memory entries found)"));
}

#[test]
fn render_omits_section_for_unused_kinds() {
    let entries = vec![entry(
        "only-feedback",
        MemoryKind::Feedback,
        "/f.md",
        "x",
    )];
    let out = render_memory_tree(&entries);

    assert!(out.contains("[feedback]"));
    assert!(!out.contains("[project]"));
    assert!(!out.contains("[reference]"));
    assert!(!out.contains("[user]"));
}
