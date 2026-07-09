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

fn render(entries: &[MemoryEntry]) -> String {
    render_memory_tree("~/proj", entries, 0)
}

#[test]
fn render_groups_by_kind_in_known_order() {
    let entries = vec![
        entry("z-user", MemoryKind::User, "/u.md", "u"),
        entry("a-fb", MemoryKind::Feedback, "/f.md", "f"),
        entry("c-proj", MemoryKind::Project, "/p.md", "p"),
        entry("d-ref", MemoryKind::Reference, "/r.md", "r"),
    ];
    let out = render(&entries);

    let p_f = out.find("\n  feedback  ").unwrap();
    let p_p = out.find("\n  project  ").unwrap();
    let p_r = out.find("\n  reference  ").unwrap();
    let p_u = out.find("\n  user  ").unwrap();
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
    let out = render(&entries);

    assert!(out.contains("engine invocations"));
    assert!(out.contains("Never run claude -p without authorization"));
}

#[test]
fn render_leads_with_label_and_entry_count() {
    let entries = vec![
        entry("a", MemoryKind::Feedback, "/a.md", "x"),
        entry("b", MemoryKind::User, "/b.md", "y"),
    ];
    let out = render(&entries);
    let summary = out.lines().next().unwrap();
    assert!(summary.starts_with("~/proj · "), "summary: {summary}");
    assert!(summary.contains("2 entries"), "summary: {summary}");
}

#[test]
fn render_empty_input_is_a_single_line() {
    let out = render(&[]);
    assert_eq!(out, "~/proj · no memory entries\n");
}

#[test]
fn render_omits_section_for_unused_kinds() {
    let entries = vec![entry(
        "only-feedback",
        MemoryKind::Feedback,
        "/f.md",
        "x",
    )];
    let out = render(&entries);

    assert!(out.contains("feedback"));
    assert!(!out.contains("project"));
    assert!(!out.contains("reference"));
    assert!(!out.contains("user"));
}

#[test]
fn render_puts_each_entry_on_one_line() {
    let entries = vec![entry(
        "engine invocations",
        MemoryKind::Feedback,
        "/f.md",
        "Never run claude -p without authorization",
    )];
    let out = render(&entries);
    let line = out
        .lines()
        .find(|l| l.contains("engine invocations"))
        .expect("entry line present");
    assert!(
        line.contains("Never run claude -p without authorization"),
        "description rides the entry line: {line}"
    );
}

#[test]
fn render_truncates_descriptions_to_width_without_wrapping() {
    let entries = vec![entry(
        "verbose",
        MemoryKind::Feedback,
        "/v.md",
        &"word ".repeat(100),
    )];
    let out = render_memory_tree("~/proj", &entries, 60);
    for line in out.lines() {
        assert!(
            line.chars().count() <= 60,
            "line exceeds width: {line:?} ({} chars)",
            line.chars().count()
        );
    }
    assert!(out.contains('…'), "long description is truncated: {out}");
}
