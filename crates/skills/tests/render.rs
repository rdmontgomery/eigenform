//! RED tests for the text projection of a layered skill scan.

use std::path::PathBuf;

use eigenform_skills::{render_tree, Layer, LayeredSkill, Skill};

fn sk(name: &str, layer: Layer, path: &str) -> LayeredSkill {
    LayeredSkill {
        layer,
        skill: Skill {
            name: name.into(),
            description: format!("{name} desc"),
            source_path: PathBuf::from(path),
        },
    }
}

#[test]
fn render_tree_empty_input_emits_only_header() {
    let out = render_tree(&[]);
    assert!(out.contains("SKILLS"));
    assert!(out.contains("(no skills found)"));
}

#[test]
fn render_tree_single_skill_shows_layer_and_path() {
    let scan = vec![sk("brainstorming", Layer::Global, "/h/brainstorming.md")];
    let out = render_tree(&scan);

    assert!(out.contains("brainstorming"));
    assert!(out.contains("[global]"));
    assert!(out.contains("/h/brainstorming.md"));
}

#[test]
fn render_tree_treats_multi_plugin_same_name_as_namespaced_not_shadowing() {
    // All-plugin contributions to the same name = three fully-qualified
    // namespaced skills (plugin:discord:access etc.). No shadowing,
    // no WINS marker.
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
    let out = render_tree(&scan);

    assert!(out.contains("[plugin:discord]"));
    assert!(out.contains("[plugin:imessage]"));
    assert!(out.contains("[plugin:telegram]"));
    assert!(
        !out.contains("WINS"),
        "plugin-only contributions are namespaced; no shadowing: {out}"
    );
    assert!(
        out.contains("(namespaced)"),
        "should annotate plugin-only co-existence: {out}"
    );
}

#[test]
fn render_tree_marks_shadowing_only_for_non_plugin_collisions() {
    // global + repo at the same name = real shadowing (only one is reachable
    // by the bare name `foo`). Repo wins by precedence.
    let scan = vec![
        sk("foo", Layer::Global, "/h/foo.md"),
        sk(
            "foo",
            Layer::Repo { project: None },
            "/r/foo.md",
        ),
    ];
    let out = render_tree(&scan);

    assert!(out.contains("WINS"));
    let winner_line = out
        .lines()
        .find(|l| l.contains("WINS"))
        .expect("expected a winner line");
    assert!(winner_line.contains("[repo]"));
}

#[test]
fn render_tree_mixed_plugin_and_non_plugin_marks_only_non_plugin_shadowing() {
    // global + plugin:foo + plugin:bar at the same name:
    //   - only one non-plugin contribution (global), so no shadowing among
    //     non-plugins.
    //   - plugins remain namespaced.
    //   - no WINS marker (only one bare-name reachable contribution).
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
    let out = render_tree(&scan);
    assert!(!out.contains("WINS"), "single non-plugin contribution = no shadowing: {out}");
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
    let out = render_tree(&scan);

    // Body contains all three contributions, in precedence order.
    let pos_g = out.find("[global]").unwrap();
    let pos_p = out.find("[plugin:superpowers]").unwrap();
    let pos_r = out.find("[repo]").unwrap();
    assert!(pos_g < pos_p);
    assert!(pos_p < pos_r);

    // Winner line names repo as the winner for `foo`.
    assert!(out.contains("WINS"));
    let winner_line = out
        .lines()
        .find(|l| l.contains("WINS"))
        .expect("expected a winner line");
    assert!(
        winner_line.contains("[repo]"),
        "winner line should name [repo]: {winner_line}"
    );
}

#[test]
fn render_tree_no_collision_does_not_emit_wins_marker() {
    let scan = vec![
        sk("alpha", Layer::Global, "/h/alpha.md"),
        sk("beta", Layer::Repo { project: None }, "/r/beta.md"),
    ];
    let out = render_tree(&scan);
    assert!(!out.contains("WINS"), "no collisions, no WINS marker: {out}");
}

#[test]
fn render_tree_skills_grouped_alphabetically() {
    let scan = vec![
        sk("zeta", Layer::Global, "/h/zeta.md"),
        sk("alpha", Layer::Global, "/h/alpha.md"),
        sk("mid", Layer::Repo { project: None }, "/r/mid.md"),
    ];
    let out = render_tree(&scan);
    let p_a = out.find("\nalpha\n").unwrap();
    let p_m = out.find("\nmid\n").unwrap();
    let p_z = out.find("\nzeta\n").unwrap();
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
    let out = render_tree(&scan);

    assert!(out.contains("[repo:proj-a]"), "render: {out}");
    assert!(out.contains("[repo:proj-b]"), "render: {out}");
    assert!(out.contains("WINS"));
}
