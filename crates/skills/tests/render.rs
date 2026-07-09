//! Tests for the text projection of a layered skill scan.

use std::path::PathBuf;

use eigenform_skills::{render_tree, Layer, LayeredSkill, RenderOpts, Skill};

fn sk(name: &str, layer: Layer, path: &str) -> LayeredSkill {
    LayeredSkill {
        layer,
        skill: Skill {
            name: name.into(),
            description: format!("{name} desc"),
            source_path: PathBuf::from(path),
            size: 0,
            tokens: 0,
        },
    }
}

fn render(scan: &[LayeredSkill]) -> String {
    render_tree(scan, &RenderOpts::default())
}

#[test]
fn render_tree_empty_input_emits_one_summary_line() {
    let out = render(&[]);
    assert_eq!(out.lines().count(), 1, "empty scan is a single line: {out}");
    assert!(out.contains("none found"));
}

#[test]
fn render_tree_summary_counts_names_and_sources() {
    let scan = vec![
        sk("alpha", Layer::Global, "/h/alpha.md"),
        sk("beta", Layer::Global, "/h/beta.md"),
        sk(
            "beta",
            Layer::Plugin { name: "p".into() },
            "/p/beta.md",
        ),
    ];
    let out = render(&scan);
    let summary = out.lines().next().unwrap();
    assert!(summary.contains("2 skills"), "summary: {summary}");
    assert!(summary.contains("3 sources"), "summary: {summary}");
}

#[test]
fn render_tree_summary_omits_sources_when_equal_to_names() {
    let scan = vec![sk("alpha", Layer::Global, "/h/alpha.md")];
    let out = render(&scan);
    let summary = out.lines().next().unwrap();
    assert!(summary.contains("1 skill"), "summary: {summary}");
    assert!(!summary.contains("sources"), "summary: {summary}");
}

#[test]
fn render_tree_note_leads_the_summary() {
    let scan = vec![sk("alpha", Layer::Global, "/h/alpha.md")];
    let opts = RenderOpts {
        note: Some("41 projects".into()),
        ..RenderOpts::default()
    };
    let out = render_tree(&scan, &opts);
    assert!(
        out.lines().next().unwrap().starts_with("41 projects · "),
        "summary: {out}"
    );
}

#[test]
fn render_tree_single_skill_shows_layer_and_path() {
    let scan = vec![sk("brainstorming", Layer::Global, "/h/brainstorming.md")];
    let out = render(&scan);

    assert!(out.contains("brainstorming"));
    assert!(out.contains("global"));
    assert!(out.contains("/h/brainstorming.md"));
}

#[test]
fn render_tree_treats_multi_plugin_same_name_as_namespaced_not_shadowing() {
    // All-plugin contributions to the same name = three fully-qualified
    // namespaced skills (plugin:discord:access etc.). No shadowing,
    // no wins marker.
    let scan = vec![
        sk(
            "access",
            Layer::Plugin {
                name: "discord".into(),
            },
            "/p/discord/access.md",
        ),
        sk(
            "access",
            Layer::Plugin {
                name: "imessage".into(),
            },
            "/p/imessage/access.md",
        ),
        sk(
            "access",
            Layer::Plugin {
                name: "telegram".into(),
            },
            "/p/telegram/access.md",
        ),
    ];
    let out = render(&scan);

    assert!(out.contains("plugin:discord"));
    assert!(out.contains("plugin:imessage"));
    assert!(out.contains("plugin:telegram"));
    assert!(
        !out.contains("wins"),
        "plugin-only contributions are namespaced; no shadowing: {out}"
    );
    assert!(
        out.contains("namespaced · 3 plugins"),
        "should annotate plugin-only co-existence: {out}"
    );
}

#[test]
fn render_tree_marks_shadowing_only_for_non_plugin_collisions() {
    // global + repo at the same name = real shadowing (only one is reachable
    // by the bare name `foo`). Repo wins by precedence, called out on the header.
    let scan = vec![
        sk("foo", Layer::Global, "/h/foo.md"),
        sk(
            "foo",
            Layer::Repo { project: None },
            "/r/foo.md",
        ),
    ];
    let out = render(&scan);

    let header = out
        .lines()
        .find(|l| l.starts_with("foo"))
        .expect("expected a header line for foo");
    assert!(header.contains("repo wins"), "header: {header}");
}

#[test]
fn render_tree_mixed_plugin_and_non_plugin_marks_only_non_plugin_shadowing() {
    // global + plugin:foo + plugin:bar at the same name:
    //   - only one non-plugin contribution (global), so no shadowing among
    //     non-plugins.
    //   - plugins remain namespaced.
    //   - no wins marker (only one bare-name reachable contribution).
    let scan = vec![
        sk("hat", Layer::Global, "/h/hat.md"),
        sk(
            "hat",
            Layer::Plugin {
                name: "foo".into(),
            },
            "/p/foo/hat.md",
        ),
        sk(
            "hat",
            Layer::Plugin {
                name: "bar".into(),
            },
            "/p/bar/hat.md",
        ),
    ];
    let out = render(&scan);
    assert!(!out.contains("wins"), "single non-plugin contribution = no shadowing: {out}");
    assert!(out.contains("3 sources"), "multi-source, non-shadowing note: {out}");
}

#[test]
fn render_tree_collides_two_non_plugin_layers_marks_last_as_winner() {
    // scan_many's output order = layer precedence order: global is first, repo is last.
    let scan = vec![
        sk("foo", Layer::Global, "/h/foo.md"),
        sk(
            "foo",
            Layer::Plugin {
                name: "superpowers".into(),
            },
            "/h/plug/foo.md",
        ),
        sk("foo", Layer::Repo { project: None }, "/r/foo.md"),
    ];
    let out = render(&scan);

    // Body contains all three contributions, in precedence order.
    let pos_g = out.find("global").unwrap();
    let pos_p = out.find("plugin:superpowers").unwrap();
    let pos_r = out.find("\n  repo ").unwrap();
    assert!(pos_g < pos_p);
    assert!(pos_p < pos_r);

    // Header names repo as the winner for `foo`.
    let header = out
        .lines()
        .find(|l| l.starts_with("foo"))
        .expect("expected a header line for foo");
    assert!(header.contains("repo wins"), "header: {header}");
}

#[test]
fn render_tree_no_collision_does_not_emit_wins_marker() {
    let scan = vec![
        sk("alpha", Layer::Global, "/h/alpha.md"),
        sk("beta", Layer::Repo { project: None }, "/r/beta.md"),
    ];
    let out = render(&scan);
    assert!(!out.contains("wins"), "no collisions, no wins marker: {out}");
}

#[test]
fn render_tree_skills_grouped_alphabetically() {
    let scan = vec![
        sk("zeta", Layer::Global, "/h/zeta.md"),
        sk("alpha", Layer::Global, "/h/alpha.md"),
        sk("mid", Layer::Repo { project: None }, "/r/mid.md"),
    ];
    let out = render(&scan);
    // Name headers carry a trailing token estimate, e.g. `alpha  ~0 tok · global`.
    let p_a = out.find("\nalpha  ").unwrap();
    let p_m = out.find("\nmid  ").unwrap();
    let p_z = out.find("\nzeta  ").unwrap();
    assert!(p_a < p_m);
    assert!(p_m < p_z);
}

#[test]
fn render_tree_tags_named_projects_distinctly() {
    let scan = vec![
        sk(
            "drift",
            Layer::Repo {
                project: Some(PathBuf::from("/tmp/proj-a")),
            },
            "/tmp/proj-a/.claude/skills/drift.md",
        ),
        sk(
            "drift",
            Layer::Repo {
                project: Some(PathBuf::from("/tmp/proj-b")),
            },
            "/tmp/proj-b/.claude/skills/drift.md",
        ),
    ];
    let out = render(&scan);

    assert!(out.contains("repo:proj-a"), "render: {out}");
    assert!(out.contains("repo:proj-b"), "render: {out}");
    assert!(out.contains("repo:proj-b wins"), "render: {out}");
}

#[test]
fn render_tree_truncates_descriptions_to_width_without_wrapping() {
    let mut skill = sk("verbose", Layer::Global, "/h/verbose.md");
    skill.skill.description = "word ".repeat(100);
    let out = render_tree(
        &[skill],
        &RenderOpts {
            width: 60,
            ..RenderOpts::default()
        },
    );
    for line in out.lines() {
        assert!(
            line.chars().count() <= 60,
            "line exceeds width: {line:?} ({} chars)",
            line.chars().count()
        );
    }
    assert!(out.contains('…'), "long description is truncated: {out}");
}

#[test]
fn render_tree_shortens_home_paths_and_elides_long_ones() {
    let long = "/Users/rick/.claude/plugins/marketplaces/claude-plugins-official/external_plugins/discord/skills/access/SKILL.md";
    let skill = sk("access", Layer::Plugin { name: "discord".into() }, long);
    let out = render_tree(
        &[skill],
        &RenderOpts {
            width: 72,
            home: Some(PathBuf::from("/Users/rick")),
            note: None,
        },
    );
    assert!(!out.contains("/Users/rick"), "home is shortened: {out}");
    let path_line = out
        .lines()
        .find(|l| l.contains("SKILL.md"))
        .expect("path line present");
    assert!(path_line.chars().count() <= 72, "path line fits width: {path_line}");
    assert!(
        path_line.contains("…/") && path_line.ends_with("discord/skills/access/SKILL.md"),
        "long path is left-elided keeping the tail: {path_line}"
    );
}
