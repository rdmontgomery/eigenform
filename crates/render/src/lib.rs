//! eigen-render: project an internal View tree to text (and later json/html).
//!
//! The View tree is an internal, refactor-freely IR — not a published schema. v0.1
//! ships the `text` projection only; json/html land when a consumer exists (browser
//! play, daemon). See `docs/plans/2026-06-03-render-crate-design.md`.

use std::fmt::Write as _;

/// A renderable view. Grows new variants (Section, Table, KeyValues, …) as views need
/// them; today a titled document and a connector-drawn tree cover the session view.
pub enum View {
    Document { title: String, body: Vec<View> },
    Tree(Vec<Node>),
}

/// One node in a [`View::Tree`].
pub struct Node {
    /// A leading symbol (role glyph, status dot, …).
    pub glyph: Option<String>,
    /// The node's primary text.
    pub text: String,
    /// A trailing annotation (e.g. `← leaf`).
    pub marker: Option<String>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(glyph: &str, text: &str) -> Node {
        Node {
            glyph: Some(glyph.to_string()),
            text: text.to_string(),
            marker: None,
            children: Vec::new(),
        }
    }

    pub fn with_children(mut self, children: Vec<Node>) -> Node {
        self.children = children;
        self
    }

    pub fn with_marker(mut self, marker: &str) -> Node {
        self.marker = Some(marker.to_string());
        self
    }
}

/// Render a view to plain text.
pub fn render_text(view: &View) -> String {
    let mut out = String::new();
    render_into(view, &mut out);
    out
}

fn render_into(view: &View, out: &mut String) {
    match view {
        View::Document { title, body } => {
            out.push_str(title);
            out.push('\n');
            for v in body {
                render_into(v, out);
            }
        }
        View::Tree(nodes) => {
            for (i, node) in nodes.iter().enumerate() {
                render_node(node, "", i == nodes.len() - 1, out);
            }
        }
    }
}

fn render_node(node: &Node, prefix: &str, is_last: bool, out: &mut String) {
    let connector = if is_last { "└─ " } else { "├─ " };
    out.push_str(prefix);
    out.push_str(connector);
    if let Some(glyph) = &node.glyph {
        out.push_str(glyph);
        out.push(' ');
    }
    out.push_str(&node.text);
    if let Some(marker) = &node.marker {
        let _ = write!(out, "  {marker}");
    }
    out.push('\n');

    let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
    for (i, child) in node.children.iter().enumerate() {
        render_node(child, &child_prefix, i == node.children.len() - 1, out);
    }
}
