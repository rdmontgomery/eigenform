//! eigenform-render: project an internal View tree to text (and later json/html).
//!
//! The View tree is an internal, refactor-freely IR — not a published schema. v0.1
//! ships the `text` projection only; json/html land when a consumer exists (browser
//! play, daemon). See `docs/plans/2026-06-03-render-crate-design.md`.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use eigenform_forest::SessionRef;
use eigenform_surgery::{Role, Session, Turn};
use serde_json::json;

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

/// The session transcript as structured JSON for woland's Manuscript: exchanges (a user
/// turn grouped with its assistant + system replies), plus a trailing `leaf` the UI
/// renders as the live input. This is ground-truth *content* only — per-turn token/cost
/// fields are left to the client's (currently stubbed) cache model. The shape matches the
/// frontend `Session` type so it can be consumed without mapping.
pub fn session_json(session: &Session) -> String {
    let visible = visible_turns(session);

    // Group like session_html: a user turn opens an exchange; assistant/system attach to
    // the open one (a stray reply before any user turn opens an empty-user exchange).
    let mut exchanges: Vec<serde_json::Value> = Vec::new();
    for turn in &visible {
        match turn.role {
            Role::User => exchanges.push(json!({ "user": content_raw(turn), "uuid": turn.uuid })),
            Role::Assistant => {
                let text = content_raw(turn);
                // Append this turn's text to the open exchange (or open one if this is a
                // leading assistant turn with no preceding user turn).
                match exchanges.last_mut() {
                    Some(cur) => append_text(cur, "assistant", &text),
                    None => {
                        // TODO: guard against leading tool-only turns producing an empty-user exchange.
                        // A session whose first visible turn is an assistant turn produces
                        // `{"user":"",…}` — technically valid but renders a blank group header.
                        exchanges.push(json!({ "user": "", "assistant": text }));
                    }
                }
                // Emit EVERY tool_use from this turn. The first one rides the open exchange
                // (which may be the group-opening user turn); each subsequent tool gets its
                // own `{user:"", tool}` exchange so the drawer's `toolExchanges` and the reach
                // map see the whole reach — not just the first call. tool_result user rows are
                // invisible, so without this all of a turn's calls would collapse onto one.
                for tool in extract_tools(turn, session) {
                    let needs_new = exchanges
                        .last()
                        .map(|e| e.get("tool").is_some())
                        .unwrap_or(true);
                    if needs_new {
                        exchanges.push(json!({ "user": "", "tool": tool }));
                    } else {
                        exchanges.last_mut().unwrap()["tool"] = tool;
                    }
                }
            }
            Role::System => {
                let dur = duration_label(turn);
                match exchanges.last_mut() {
                    Some(cur) => cur["system"] = json!(dur),
                    None => exchanges.push(json!({ "user": "", "system": dur })),
                }
            }
        }
    }

    let count = exchanges.len();
    let mut out: Vec<serde_json::Value> = Vec::with_capacity(count + 1);
    for (i, mut e) in exchanges.into_iter().enumerate() {
        e["n"] = json!(i + 1);
        e["tok"] = json!(0);
        out.push(e);
    }
    // the leaf — the live end the UI turns into an input
    let total = count + 1;
    out.push(json!({ "n": total, "tok": 0, "user": "", "leaf": true }));

    serde_json::to_string(&json!({
        "id": short_id(&session.session_id),
        "total": total,
        "branches": 0,
        "windowStart": 1,
        "model": session_model(session),
        "exchanges": out,
    }))
    .expect("session json serializes")
}

/// Maximum byte length of tool output (and input) strings emitted in session_json.
/// Inputs can be large (e.g. a Write tool with full file contents). We cap both at the
/// same limit for simplicity; the truncated flag signals the cut to the UI.
const TOOL_CONTENT_BYTES: usize = 50 * 1024; // 50 KB

/// All `tool_use` content blocks from an assistant turn's message content array.
/// Returns `(tool_use_id, name, input)` triples.
fn tool_use_blocks(turn: &Turn) -> Vec<(String, String, serde_json::Value)> {
    let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
    let Some(blocks) = value["message"]["content"].as_array() else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        .filter_map(|b| {
            let id = b.get("id")?.as_str()?.to_string();
            let name = b.get("name")?.as_str()?.to_string();
            let input = b.get("input").cloned().unwrap_or(serde_json::Value::Null);
            Some((id, name, input))
        })
        .collect()
}

/// The model id the session ran on, read from assistant turns' `message.model`.
/// Returns the most recent one seen (a resumed session can change models), or
/// `None` if no assistant turn carries a model.
fn session_model(session: &Session) -> Option<String> {
    let mut model = None;
    for turn in session.turns() {
        if turn.role != Role::Assistant {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
        if let Some(m) = value["message"]["model"].as_str() {
            model = Some(m.to_string());
        }
    }
    model
}

/// Find the tool_result output for a given `tool_use_id` by scanning all session turns
/// (including invisible ones — tool_result user rows have no text content so `visible_turns`
/// drops them). Returns the joined text content, or `None` if not found.
fn tool_result_output(session: &Session, tool_use_id: &str) -> Option<String> {
    for turn in session.turns() {
        if turn.role != Role::User {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
        let Some(blocks) = value["message"]["content"].as_array() else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                continue;
            }
            if block.get("tool_use_id").and_then(|t| t.as_str()) != Some(tool_use_id) {
                continue;
            }
            // content may be a string or an array of text blocks
            let text = match &block["content"] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(parts) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            return Some(text);
        }
    }
    None
}

/// Truncate a string to at most `TOOL_CONTENT_BYTES` bytes (on a char boundary).
/// Returns `(truncated_string, was_truncated)`.
fn truncate_tool_content(s: &str) -> (&str, bool) {
    if s.len() <= TOOL_CONTENT_BYTES {
        return (s, false);
    }
    // Walk back to a char boundary.
    let mut end = TOOL_CONTENT_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

/// Build `tool` JSON objects for EVERY `tool_use` block in an assistant turn, in
/// content order, each paired with its result from the session. A single assistant
/// message can issue several tool calls at once (parallel tool use); all are emitted.
///
/// Field naming asymmetry (historical-compat): `truncated` applies to OUTPUT (pre-existing
/// field name consumed by woland), while `inputTruncated` applies to INPUT (added in 4.1).
/// Do not normalise these without a simultaneous woland update.
fn extract_tools(turn: &Turn, session: &Session) -> Vec<serde_json::Value> {
    tool_use_blocks(turn)
        .into_iter()
        .map(|(id, name, input)| build_tool(id, name, input, session))
        .collect()
}

/// Build one `tool` JSON object from a single tool_use block, pairing it with its result.
fn build_tool(id: String, name: String, input: serde_json::Value, session: &Session) -> serde_json::Value {
    // Truncate input if serialized form exceeds the cap.
    let input_str = serde_json::to_string(&input).unwrap_or_default();
    let (input_val, input_truncated) = if input_str.len() <= TOOL_CONTENT_BYTES {
        (input, false)
    } else {
        let (cut, _) = truncate_tool_content(&input_str);
        // Re-parse the cut JSON; if it fails (mid-value cut), fall back to the string.
        let fallback: serde_json::Value = serde_json::from_str(cut)
            .unwrap_or_else(|_| serde_json::Value::String(cut.to_string()));
        (fallback, true)
    };

    // Derive a one-word display arg from the input object (first string value found).
    // v1 heuristic: scan object values in insertion order, take the first str.
    // Known failure modes: non-string first field (e.g. numeric or object) falls back
    // to `name`; a UUID-ish first field (e.g. tool_use_id leaked into input) would be
    // shown verbatim. Both accepted for v1 — Task 4.3 drill-down renders the full input.
    let arg = input_val
        .as_object()
        .and_then(|m| m.values().find_map(|v| v.as_str()))
        .unwrap_or(&name)
        .to_string();

    let mut tool = json!({
        "kind": name,
        "arg": arg,
        "delta": "",
        "input": input_val,
    });
    if input_truncated {
        tool["inputTruncated"] = json!(true);
    }

    // Attach output if a matching tool_result exists in the session.
    if let Some(output_raw) = tool_result_output(session, &id) {
        let (out, out_truncated) = truncate_tool_content(&output_raw);
        tool["output"] = json!(out);
        if out_truncated {
            tool["truncated"] = json!(true);
        }
    }

    tool
}

/// Set `obj[key]` to `text`, or append (blank-line joined) if it already holds text — so
/// several assistant turns in one exchange read as one reply.
fn append_text(obj: &mut serde_json::Value, key: &str, text: &str) {
    match obj.get_mut(key) {
        Some(serde_json::Value::String(s)) if !s.is_empty() => {
            s.push_str("\n\n");
            s.push_str(text);
        }
        _ => obj[key] = json!(text),
    }
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
        t.extend(std::iter::repeat_n(' ', width - chars.len()));
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
        Role::Assistant => !content_preview(turn).is_empty() || has_tool_use(turn),
        Role::User => !content_preview(turn).is_empty(),
    }
}

/// Whether an assistant turn contains at least one tool_use content block.
fn has_tool_use(turn: &Turn) -> bool {
    let value: serde_json::Value = serde_json::from_str(turn.raw()).unwrap_or_default();
    value["message"]["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        })
        .unwrap_or(false)
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

#[cfg(test)]
mod session_json_tests {
    use super::*;
    use eigenform_surgery::Session;

    fn parse(lines: &[&str]) -> Session {
        Session::parse_str(&(lines.join("\n") + "\n")).unwrap_or_else(|e| match e {})
    }

    #[test]
    fn groups_exchanges_and_appends_a_leaf() {
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"render the transcript"}}"#,
            r#"{"type":"assistant","uuid":"a1","sessionId":"abc12345-0000","message":{"role":"assistant","content":[{"type":"text","text":"on it"}]}}"#,
            r#"{"type":"system","uuid":"s1","sessionId":"abc12345-0000","durationMs":4200}"#,
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        assert_eq!(doc["id"], "abc12345");
        assert_eq!(doc["total"], 2); // one real exchange + the leaf
        let ex = doc["exchanges"].as_array().unwrap();
        assert_eq!(ex.len(), 2);
        assert_eq!(ex[0]["n"], 1);
        assert_eq!(ex[0]["uuid"], "u1"); // the user turn's uuid — the fork target
        assert_eq!(ex[0]["user"], "render the transcript");
        assert_eq!(ex[0]["assistant"], "on it");
        assert_eq!(ex[0]["system"], "4.2s");
        assert_eq!(ex[1]["n"], 2);
        assert_eq!(ex[1]["leaf"], true);
    }

    #[test]
    fn empty_session_is_just_a_leaf() {
        let doc: serde_json::Value = serde_json::from_str(&session_json(&parse(&[]))).unwrap();
        assert_eq!(doc["total"], 1);
        assert_eq!(doc["exchanges"].as_array().unwrap().len(), 1);
        assert_eq!(doc["exchanges"][0]["leaf"], true);
    }

    #[test]
    fn emits_model_from_assistant_turn() {
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"assistant","uuid":"a1","sessionId":"abc12345-0000","message":{"role":"assistant","model":"claude-opus-4-8","content":[{"type":"text","text":"yo"}]}}"#,
        ]);
        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        assert_eq!(doc["model"], "claude-opus-4-8");
    }

    #[test]
    fn model_is_null_when_no_assistant_turn_carries_one() {
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"hi"}}"#,
        ]);
        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        assert!(doc["model"].is_null());
    }

    #[test]
    fn model_takes_the_most_recent_assistant_turn() {
        let session = parse(&[
            r#"{"type":"assistant","uuid":"a1","sessionId":"abc12345-0000","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"text","text":"a"}]}}"#,
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"more"}}"#,
            r#"{"type":"assistant","uuid":"a2","sessionId":"abc12345-0000","message":{"role":"assistant","model":"claude-opus-4-8","content":[{"type":"text","text":"b"}]}}"#,
        ]);
        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        assert_eq!(doc["model"], "claude-opus-4-8");
    }

    // ── tool input/output tests ──────────────────────────────────────────────

    /// Build an assistant turn containing one tool_use block (and optionally a text block).
    fn assistant_with_tool(uuid: &str, tool_id: &str, name: &str, input_json: &str, text: &str) -> String {
        let input: serde_json::Value = serde_json::from_str(input_json).unwrap();
        let mut blocks = serde_json::json!([{
            "type": "tool_use",
            "id": tool_id,
            "name": name,
            "input": input,
        }]);
        if !text.is_empty() {
            let arr = blocks.as_array_mut().unwrap();
            arr.insert(0, json!({ "type": "text", "text": text }));
        }
        serde_json::to_string(&json!({
            "type": "assistant",
            "uuid": uuid,
            "sessionId": "abc12345-0000",
            "message": { "role": "assistant", "content": blocks }
        })).unwrap()
    }

    /// Build a user turn that is entirely a tool_result (no visible text).
    fn tool_result_turn(uuid: &str, tool_id: &str, output: &str) -> String {
        serde_json::to_string(&json!({
            "type": "user",
            "uuid": uuid,
            "sessionId": "abc12345-0000",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": [{ "type": "text", "text": output }]
                }]
            }
        })).unwrap()
    }

    #[test]
    fn tool_use_input_and_output_are_attached_to_exchange() {
        let input = r#"{"file_path":"/x","old_string":"a","new_string":"b"}"#;
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"edit the file"}}"#,
            &assistant_with_tool("a1", "toolu_01", "Edit", input, "I'll edit it"),
            &tool_result_turn("u2", "toolu_01", "ok, done"),
            r#"{"type":"system","uuid":"s1","sessionId":"abc12345-0000","durationMs":1200}"#,
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        let ex = &doc["exchanges"][0];

        assert_eq!(ex["tool"]["kind"], "Edit");
        assert_eq!(ex["tool"]["input"]["file_path"], "/x");
        assert_eq!(ex["tool"]["input"]["old_string"], "a");
        assert_eq!(ex["tool"]["input"]["new_string"], "b");
        assert_eq!(ex["tool"]["output"], "ok, done");
        // truncated absent when content fits
        assert!(ex["tool"]["truncated"].is_null(), "truncated absent when content fits");
    }

    #[test]
    fn exchange_without_tool_use_has_no_tool_field() {
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"hello"}}"#,
            r#"{"type":"assistant","uuid":"a1","sessionId":"abc12345-0000","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#,
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        // tool field must be absent (not null) for existing consumers
        assert!(doc["exchanges"][0].get("tool").is_none(), "no tool field on plain text exchange");
    }

    #[test]
    fn tool_output_truncated_at_50kb_with_flag() {
        // Build an output string that is exactly 50KB + 1 byte
        let big_output = "x".repeat(TOOL_CONTENT_BYTES + 1);
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"run it"}}"#,
            &assistant_with_tool("a1", "toolu_02", "Bash", r#"{"command":"cat big"}"#, ""),
            &tool_result_turn("u2", "toolu_02", &big_output),
            r#"{"type":"system","uuid":"s1","sessionId":"abc12345-0000","durationMs":500}"#,
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        let tool = &doc["exchanges"][0]["tool"];

        assert_eq!(tool["truncated"], true, "truncated flag set for oversized output");
        let out = tool["output"].as_str().expect("output is a string");
        assert!(out.len() <= TOOL_CONTENT_BYTES, "output capped at 50KB");
        assert!(!out.is_empty(), "output not empty");
    }

    #[test]
    fn tool_input_truncated_at_50kb_with_flag() {
        // Build an input JSON whose serialized form exceeds 50KB
        let big_value = "y".repeat(TOOL_CONTENT_BYTES + 1);
        let input = format!(r#"{{"content":{}}}"#, serde_json::to_string(&big_value).unwrap());
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"write it"}}"#,
            &assistant_with_tool("a1", "toolu_03", "Write", &input, ""),
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        let tool = &doc["exchanges"][0]["tool"];

        assert_eq!(tool["inputTruncated"], true, "inputTruncated flag set for oversized input");
    }

    #[test]
    fn existing_exchanges_unchanged_when_no_tools() {
        // Verify exact parity with the pre-tool-feature behaviour for a plain session.
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"render the transcript"}}"#,
            r#"{"type":"assistant","uuid":"a1","sessionId":"abc12345-0000","message":{"role":"assistant","content":[{"type":"text","text":"on it"}]}}"#,
            r#"{"type":"system","uuid":"s1","sessionId":"abc12345-0000","durationMs":4200}"#,
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        assert_eq!(doc["id"], "abc12345");
        assert_eq!(doc["exchanges"][0]["user"], "render the transcript");
        assert_eq!(doc["exchanges"][0]["assistant"], "on it");
        assert_eq!(doc["exchanges"][0]["system"], "4.2s");
        assert!(doc["exchanges"][0].get("tool").is_none(), "no tool field on plain exchange");
    }

    #[test]
    fn all_tools_across_a_user_turn_are_emitted() {
        // One user turn drives three tool calls across three assistant turns, each
        // separated by an (invisible) tool_result user row. Every tool must survive —
        // not just the first — so the reach map / drawer see the whole reach.
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"do the thing"}}"#,
            &assistant_with_tool("a1", "toolu_01", "Read", r#"{"file_path":"/a"}"#, "reading"),
            &tool_result_turn("r1", "toolu_01", "contents a"),
            &assistant_with_tool("a2", "toolu_02", "Edit", r#"{"file_path":"/b"}"#, ""),
            &tool_result_turn("r2", "toolu_02", "edited b"),
            &assistant_with_tool("a3", "toolu_03", "Bash", r#"{"command":"ls"}"#, "listing"),
            &tool_result_turn("r3", "toolu_03", "file list"),
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        let exchanges = doc["exchanges"].as_array().unwrap();

        // Every tool kind, in session order.
        let kinds: Vec<&str> = exchanges
            .iter()
            .filter_map(|e| e["tool"]["kind"].as_str())
            .collect();
        assert_eq!(kinds, vec!["Read", "Edit", "Bash"], "all three tool calls are emitted in order");

        // Each tool keeps its own paired output.
        let outputs: Vec<&str> = exchanges
            .iter()
            .filter_map(|e| e["tool"]["output"].as_str())
            .collect();
        assert_eq!(outputs, vec!["contents a", "edited b", "file list"]);

        // The first tool still rides the group-opening user exchange (drawer invariant).
        assert_eq!(exchanges[0]["user"], "do the thing");
        assert_eq!(exchanges[0]["tool"]["kind"], "Read");
        // Subsequent tool exchanges carry no user text so they don't open new groups.
        assert_eq!(exchanges[1]["user"], "");
        assert_eq!(exchanges[2]["user"], "");
    }

    #[test]
    fn parallel_tool_uses_in_one_message_are_all_emitted() {
        // A single assistant message issues two tool_use blocks at once — both survive.
        let two_tools = serde_json::to_string(&json!({
            "type": "assistant",
            "uuid": "a1",
            "sessionId": "abc12345-0000",
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "toolu_01", "name": "Read", "input": {"file_path": "/a"} },
                { "type": "tool_use", "id": "toolu_02", "name": "Read", "input": {"file_path": "/b"} }
            ]}
        })).unwrap();
        let session = parse(&[
            r#"{"type":"user","uuid":"u1","sessionId":"abc12345-0000","message":{"role":"user","content":"read both"}}"#,
            &two_tools,
            &tool_result_turn("r1", "toolu_01", "contents a"),
            &tool_result_turn("r2", "toolu_02", "contents b"),
        ]);

        let doc: serde_json::Value = serde_json::from_str(&session_json(&session)).unwrap();
        let exchanges = doc["exchanges"].as_array().unwrap();
        let paths: Vec<&str> = exchanges
            .iter()
            .filter_map(|e| e["tool"]["input"]["file_path"].as_str())
            .collect();
        assert_eq!(paths, vec!["/a", "/b"], "both parallel tool calls are emitted");
        // Outputs pair correctly by tool_use_id, not position.
        let outputs: Vec<&str> = exchanges
            .iter()
            .filter_map(|e| e["tool"]["output"].as_str())
            .collect();
        assert_eq!(outputs, vec!["contents a", "contents b"]);
    }
}
