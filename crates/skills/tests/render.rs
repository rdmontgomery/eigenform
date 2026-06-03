//! RED tests for the text projection of a layered skill scan.

use std::path::PathBuf;

use eigen_skills::{render_tree, Layer, LayeredSkill, Skill};

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
fn render_tree_collides_two_layers_marks_last_as_winner() {
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
        sk("foo", Layer::Repo, "/r/foo.md"),
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
        sk("beta", Layer::Repo, "/r/beta.md"),
    ];
    let out = render_tree(&scan);
    assert!(!out.contains("WINS"), "no collisions, no WINS marker: {out}");
}

#[test]
fn render_tree_skills_grouped_alphabetically() {
    let scan = vec![
        sk("zeta", Layer::Global, "/h/zeta.md"),
        sk("alpha", Layer::Global, "/h/alpha.md"),
        sk("mid", Layer::Repo, "/r/mid.md"),
    ];
    let out = render_tree(&scan);
    let p_a = out.find("\nalpha\n").unwrap();
    let p_m = out.find("\nmid\n").unwrap();
    let p_z = out.find("\nzeta\n").unwrap();
    assert!(p_a < p_m);
    assert!(p_m < p_z);
}
