//! The text projection: a View tree rendered with box-drawing connectors.

use eigen_render::{render_text, Node, View};

#[test]
fn render_text_draws_tree_connectors_and_glyphs() {
    let view = View::Tree(vec![
        Node::new("●", "user one"),
        Node::new("●", "user two").with_children(vec![Node::new("◇", "assistant two")]),
    ]);

    let expected = "\
├─ ● user one
└─ ● user two
   └─ ◇ assistant two
";
    assert_eq!(render_text(&view), expected);
}

#[test]
fn nested_siblings_get_a_continuation_rail() {
    let view = View::Tree(vec![Node::new("●", "u").with_children(vec![
        Node::new("◇", "a"),
        Node::new("·", "s"),
    ])]);

    let expected = "\
└─ ● u
   ├─ ◇ a
   └─ · s
";
    assert_eq!(render_text(&view), expected);
}

#[test]
fn a_node_marker_is_appended() {
    let view = View::Tree(vec![Node::new("◇", "last").with_marker("← leaf")]);
    assert_eq!(render_text(&view), "└─ ◇ last  ← leaf\n");
}

#[test]
fn document_prints_title_then_body() {
    let view = View::Document {
        title: "session abc · 1 exchange".to_string(),
        body: vec![View::Tree(vec![Node::new("●", "hi")])],
    };
    let expected = "\
session abc · 1 exchange
└─ ● hi
";
    assert_eq!(render_text(&view), expected);
}
