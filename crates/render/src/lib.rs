//! eigen-render: project an internal View tree to text (and later json/html).
//!
//! The View tree is an internal, refactor-freely IR — not a published schema. v0.1
//! ships the `text` projection only; json/html land when a consumer exists (browser
//! play, daemon). See `docs/plans/2026-06-03-render-crate-design.md`.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use eigen_forest::SessionRef;
use eigen_surgery::{Role, Session, Turn};

/// Render a recent-session list: one row per session, newest at the bottom.
pub fn sessions_view(sessions: &[SessionRef], now: DateTime<Utc>, show_project: bool) -> View {
    let title = format!("{} session{}", sessions.len(), if sessions.len() == 1 { "" } else { "s" });

    // `sessions` arrives recent-first; emit oldest→newest so the latest sits at the bottom.
    let lines = sessions
        .iter()
        .rev()
        .map(|s| {
            let short = short_id(&s.uuid);
            let when = relative_time(now, s.recency);
            let label = s.title.clone().unwrap_or_else(|| "(untitled)".to_string());
            if show_project {
                format!("{short}  {when:<8}  {:<24}  {label}", s.cwd.display())
            } else {
                format!("{short}  {when:<8}  {label}")
            }
        })
        .collect();

    View::Document {
        title,
        body: vec![View::Lines(lines)],
    }
}

fn relative_time(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let d = now - then;
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", d.num_minutes())
    } else if secs < 86_400 {
        format!("{}h ago", d.num_hours())
    } else if d.num_days() < 7 {
        format!("{}d ago", d.num_days())
    } else {
        format!("{}w ago", d.num_weeks())
    }
}

/// Maximum displayed width of a turn's content preview, in chars.
const PREVIEW_WIDTH: usize = 60;

/// Build a [`View`] from a parsed session: group turns by exchange (user turns at top
/// level, assistant/system replies nested beneath), glyph by role, one-line previews,
/// resume leaf marked.
pub fn session_view(session: &Session) -> View {
    // Only conversational content: user/assistant turns with text, and system rows that
    // carry a turn duration. Thinking-only rows and meta system rows are noise.
    let visible: Vec<&Turn> = session
        .turns()
        .into_iter()
        .filter(|t| is_visible(t))
        .collect();

    // The resume leaf often points at a row we hide (a turn_duration or meta system row).
    // Mark it where it lands if visible, otherwise fall back to the last visible turn.
    let leaf = session.resume_leaf();
    let leaf_target = match &leaf {
        Some(l) if visible.iter().any(|t| &t.uuid == l) => Some(l.clone()),
        _ => visible.last().map(|t| t.uuid.clone()),
    };

    let exchanges = visible.iter().filter(|t| t.role == Role::User).count();
    let title = format!(
        "session {} · {} exchange{}",
        short_id(&session.session_id),
        exchanges,
        if exchanges == 1 { "" } else { "s" },
    );

    let mut top: Vec<Node> = Vec::new();
    for turn in &visible {
        let node = turn_node(turn, leaf_target.as_deref());
        if turn.role == Role::User {
            top.push(node);
        } else if let Some(parent) = top.last_mut() {
            parent.children.push(node);
        } else {
            // A non-user turn before any user turn: keep it at top level rather than drop.
            top.push(node);
        }
    }

    View::Document {
        title,
        body: vec![View::Tree(top)],
    }
}

/// Whether a turn carries conversational content worth showing.
fn is_visible(turn: &Turn) -> bool {
    match turn.role {
        Role::System => has_duration(turn),
        _ => !content_preview(turn).is_empty(),
    }
}

fn has_duration(turn: &Turn) -> bool {
    let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
    value.get("durationMs").and_then(|d| d.as_f64()).is_some()
}

fn turn_node(turn: &Turn, leaf: Option<&str>) -> Node {
    let (glyph, label) = match turn.role {
        Role::User => ("●", "user"),
        Role::Assistant => ("◇", "assistant"),
        Role::System => ("·", "system"),
    };
    let preview = match turn.role {
        Role::System => duration_label(turn),
        _ => content_preview(turn),
    };
    let mut node = Node::new(glyph, &format!("{label:<9}  {preview}"));
    if leaf == Some(turn.uuid.as_str()) {
        node = node.with_marker("← leaf");
    }
    node
}

fn short_id(session_id: &str) -> String {
    session_id.split('-').next().unwrap_or(session_id).to_string()
}

/// Collapse a turn's message content to a single truncated line.
fn content_preview(turn: &Turn) -> String {
    let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
    let raw = match &value["message"]["content"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    truncate(&collapse_ws(&raw))
}

/// A system turn_duration row's duration in seconds, e.g. `4.2s`.
fn duration_label(turn: &Turn) -> String {
    let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
    match value.get("durationMs").and_then(|d| d.as_f64()) {
        Some(ms) => format!("{:.1}s", ms / 1000.0),
        None => "system".to_string(),
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= PREVIEW_WIDTH {
        return s.to_string();
    }
    let kept: String = s.chars().take(PREVIEW_WIDTH).collect();
    format!("{kept}…")
}

/// A renderable view. Grows new variants (Section, Table, KeyValues, …) as views need
/// them; today a titled document and a connector-drawn tree cover the session view.
pub enum View {
    Document { title: String, body: Vec<View> },
    Tree(Vec<Node>),
    /// A flat list of pre-formatted rows (no tree connectors).
    Lines(Vec<String>),
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
        View::Lines(lines) => {
            for line in lines {
                out.push_str(line);
                out.push('\n');
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
