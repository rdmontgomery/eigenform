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

/// The session transcript as semantic HTML: a collapsible `<details>` per exchange
/// (user turn + nested replies), full untruncated content, escaped. This is render's
/// html projection for woland's right pane.
pub fn session_html(session: &Session) -> String {
    let visible = visible_turns(session);
    let leaf = visible_leaf(session, &visible);
    let exchanges = visible.iter().filter(|t| t.role == Role::User).count();

    let mut out = String::new();
    out.push_str(&format!(
        "<article class=\"session\"><header>session {} · {} exchange{}</header>",
        esc(&short_id(&session.session_id)),
        exchanges,
        if exchanges == 1 { "" } else { "s" },
    ));

    let mut open = false;
    for turn in &visible {
        if turn.role == Role::User {
            if open {
                out.push_str("</details>");
            }
            out.push_str("<details open class=\"exchange\"><summary>");
            out.push_str(&turn_html(turn, &leaf));
            out.push_str("</summary>");
            open = true;
        } else {
            if !open {
                out.push_str("<details open class=\"exchange\">");
                open = true;
            }
            out.push_str("<div class=\"reply\">");
            out.push_str(&turn_html(turn, &leaf));
            out.push_str("</div>");
        }
    }
    if open {
        out.push_str("</details>");
    }
    out.push_str("</article>");
    out
}

fn turn_html(turn: &Turn, leaf: &Option<String>) -> String {
    let (glyph, label) = turn_glyph_label(turn.role);
    let content = match turn.role {
        Role::System => duration_label(turn),
        _ => content_raw(turn), // preserve newlines; pre-wrap renders them
    };
    let marker = if leaf.as_deref() == Some(turn.uuid.as_str()) {
        " <span class=\"leaf\">← leaf</span>"
    } else {
        ""
    };
    format!(
        "<span class=\"glyph {label}\">{glyph}</span> \
         <span class=\"role\">{label}</span> \
         <span class=\"content\">{}</span>{marker}",
        esc(&content),
    )
}

/// Escape text for safe embedding in HTML.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Width of the source (left) column in the side-by-side fork diff.
const DIFF_COL: usize = 46;

/// A side-by-side diff of a source session and a fork, aligned by turn uuid (fork_at
/// preserves uuids). Source left, fork right; kept turns on both sides, dropped on the
/// left only, injected on the right only, edited on both with differing content.
pub fn fork_diff_view(source: &Session, fork: &Session) -> View {
    let s_turns = visible_turns(source);
    let f_turns = visible_turns(fork);
    let s_leaf = visible_leaf(source, &s_turns);
    let f_leaf = visible_leaf(fork, &f_turns);

    let f_by_uuid: std::collections::HashMap<&str, &Turn> =
        f_turns.iter().map(|t| (t.uuid.as_str(), *t)).collect();
    let s_uuids: std::collections::HashSet<&str> =
        s_turns.iter().map(|t| t.uuid.as_str()).collect();

    let (mut kept, mut dropped, mut injected, mut edited) = (0, 0, 0, 0);
    let mut rows: Vec<String> = Vec::new();

    // Source order: each turn is kept, edited, or dropped.
    for s in &s_turns {
        match f_by_uuid.get(s.uuid.as_str()) {
            Some(f) if content_text(s) == content_text(f) => {
                kept += 1;
                rows.push(diff_row(" ", Some((s, &s_leaf)), Some((f, &f_leaf))));
            }
            Some(f) => {
                edited += 1;
                rows.push(diff_row("~", Some((s, &s_leaf)), Some((f, &f_leaf))));
            }
            None => {
                dropped += 1;
                rows.push(diff_row("-", Some((s, &s_leaf)), None));
            }
        }
    }
    // Fork-only turns (injected), appended in fork order.
    for f in &f_turns {
        if !s_uuids.contains(f.uuid.as_str()) {
            injected += 1;
            rows.push(diff_row("+", None, Some((f, &f_leaf))));
        }
    }

    let title = format!(
        "diff {} → {}",
        short_id(&source.session_id),
        short_id(&fork.session_id)
    );
    let summary = vec![
        format!("kept {kept}, dropped {dropped}, injected {injected}, edited {edited}"),
        format!(
            "leaf: {}  ⇒  {}",
            leaf_desc(&s_turns, &s_leaf),
            leaf_desc(&f_turns, &f_leaf)
        ),
    ];

    View::Document {
        title,
        body: vec![View::Lines(summary), View::Lines(rows)],
    }
}

/// One diff row: a 1-char status, the left (source) cell fitted to a column, `│`, then the
/// right (fork) cell. A `None` side renders blank.
fn diff_row(status: &str, left: Option<(&&Turn, &Option<String>)>, right: Option<(&&Turn, &Option<String>)>) -> String {
    let left_text = left.map(|(t, leaf)| diff_cell(t, leaf)).unwrap_or_default();
    let right_text = right.map(|(t, leaf)| diff_cell(t, leaf)).unwrap_or_default();
    format!("{status} {} │ {}", fit(&left_text, DIFF_COL), right_text)
}

/// A turn's cell: glyph, role, preview, plus a leaf marker when this turn is the head.
fn diff_cell(turn: &Turn, leaf: &Option<String>) -> String {
    let (glyph, label) = turn_glyph_label(turn.role);
    let preview = match turn.role {
        Role::System => duration_label(turn),
        _ => truncate(&content_text(turn)),
    };
    let marker = if leaf.as_deref() == Some(turn.uuid.as_str()) {
        "  ←leaf"
    } else {
        ""
    };
    format!("{glyph} {label:<9} {preview}{marker}")
}

fn leaf_desc(turns: &[&Turn], leaf: &Option<String>) -> String {
    match leaf {
        Some(uuid) => turns
            .iter()
            .find(|t| &t.uuid == uuid)
            .map(|t| {
                let (glyph, label) = turn_glyph_label(t.role);
                format!("{glyph} {label}")
            })
            .unwrap_or_else(|| "—".to_string()),
        None => "—".to_string(),
    }
}

/// Pad or truncate (with an ellipsis) a string to exactly `width` display chars.
fn fit(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > width {
        let mut t: String = chars[..width.saturating_sub(1)].iter().collect();
        t.push('…');
        t
    } else {
        let mut t = s.to_string();
        t.extend(std::iter::repeat(' ').take(width - chars.len()));
        t
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
    let visible = visible_turns(session);
    let leaf_target = visible_leaf(session, &visible);

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

/// The conversational turns worth showing: user/assistant with text, system with a
/// duration. Thinking-only and meta system rows are noise.
fn visible_turns(session: &Session) -> Vec<&Turn> {
    session.turns().into_iter().filter(|t| is_visible(t)).collect()
}

/// The visible turn that carries the resume head: the leaf if it maps to a visible turn,
/// otherwise the last visible turn (the leaf often points at a hidden system row).
fn visible_leaf(session: &Session, visible: &[&Turn]) -> Option<String> {
    match session.resume_leaf() {
        Some(l) if visible.iter().any(|t| t.uuid == l) => Some(l),
        _ => visible.last().map(|t| t.uuid.clone()),
    }
}

/// The glyph and role label for a turn.
fn turn_glyph_label(role: Role) -> (&'static str, &'static str) {
    match role {
        Role::User => ("●", "user"),
        Role::Assistant => ("◇", "assistant"),
        Role::System => ("·", "system"),
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
    let (glyph, label) = turn_glyph_label(turn.role);
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

/// A turn's full message content with newlines preserved (text blocks joined by a blank
/// line). For the Manuscript HTML, where `white-space: pre-wrap` renders the breaks.
fn content_raw(turn: &Turn) -> String {
    let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
    match &value["message"]["content"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// A turn's content as a single whitespace-collapsed line (for the tree and diff views).
fn content_text(turn: &Turn) -> String {
    collapse_ws(&content_raw(turn))
}

/// Collapse a turn's message content to a single truncated line for display.
fn content_preview(turn: &Turn) -> String {
    truncate(&content_text(turn))
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
