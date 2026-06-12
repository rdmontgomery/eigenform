//! eigenform-surgery: context surgery on Claude Code session JSONL files.
//!
//! Passthrough model: only user/assistant/system turns and `last-prompt` rows are
//! modeled; every other row is retained as a verbatim line. See
//! `docs/plans/2026-06-03-surgery-crate-design.md`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ParseError {}

#[derive(Debug, Error)]
pub enum SurgeryError {
    /// No modeled turn (user/assistant/system) carries the requested uuid.
    #[error("no turn with uuid `{0}` in session")]
    TurnNotFound(String),
    /// There is no completed-turn boundary before the requested turn (e.g. it is the
    /// first turn), so there is nothing to rewind to.
    #[error("no completed-turn boundary before `{0}` to fork from")]
    NoBoundaryBefore(String),
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
}

#[derive(Debug, Error)]
pub enum WriteError {
    /// A session file with this uuid already exists; surgery never overwrites.
    #[error("session file already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum RewriteError {
    /// The old session uuid appears somewhere other than a `sessionId` value — a blind
    /// swap would corrupt content. Refuse rather than mangle.
    #[error("session uuid `{old}` appears off a sessionId field; refusing to swap row: {line}")]
    StrayOccurrence { old: String, line: String },
}

/// Replace the session id `old` with `new` in one JSONL line, targeting **only** values
/// that sit at a `sessionId` key (anywhere in the tree). Substring occurrences inside
/// content — e.g. a tool that printed the session's own `<id>.jsonl` filename — are left
/// untouched (spike 07 finding 4). Bails if `old` is the exact full value of some other
/// key (`exact-other`), as an early warning for an id-bearing key we don't yet rewrite.
/// A line not containing `old`, or not valid JSON, is returned unchanged.
pub fn rewrite_session_id(line: &str, old: &str, new: &str) -> Result<String, RewriteError> {
    if !line.contains(old) {
        return Ok(line.to_string());
    }
    let Ok(mut value) = serde_json::from_str::<Value>(line) else {
        return Ok(line.to_string());
    };
    let mut stray = false;
    rewrite_session_fields(&mut value, None, old, new, &mut stray);
    if stray {
        return Err(RewriteError::StrayOccurrence {
            old: old.to_string(),
            line: line.to_string(),
        });
    }
    Ok(serde_json::to_string(&value).expect("re-serialize rewritten row"))
}

/// Walk the tree: swap string values equal to `old` that are keyed `sessionId`; flag any
/// other key whose full value equals `old` as a stray.
fn rewrite_session_fields(
    value: &mut Value,
    key: Option<&str>,
    old: &str,
    new: &str,
    stray: &mut bool,
) {
    match value {
        Value::String(s) => {
            if s == old {
                if key == Some("sessionId") {
                    *s = new.to_string();
                } else {
                    *stray = true;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_session_fields(item, None, old, new, stray);
            }
        }
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                rewrite_session_fields(v, Some(k.as_str()), old, new, stray);
            }
        }
        _ => {}
    }
}

/// The role of a conversation turn, taken from the row's `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// A modeled conversation turn (`type` ∈ {user, assistant, system}). We index the
/// fields surgery reasons about; `raw` is the original line, re-emitted verbatim
/// unless deliberately edited.
#[derive(Debug, Clone)]
pub struct Turn {
    pub uuid: String,
    pub parent_uuid: Option<String>,
    pub is_sidechain: bool,
    pub role: Role,
    raw: String,
}

impl Turn {
    /// The original JSONL line for this turn.
    pub fn raw(&self) -> &str {
        &self.raw
    }
}

/// A `last-prompt` row: carries the resume-head pointer (`leafUuid`).
#[derive(Debug, Clone)]
pub struct LastPrompt {
    pub leaf_uuid: String,
    raw: String,
}

/// One line of a session JSONL. Unmodeled rows are kept as verbatim bytes so surgery
/// never perturbs a row it doesn't understand.
#[derive(Debug, Clone)]
pub enum Row {
    Turn(Turn),
    LastPrompt(LastPrompt),
    Opaque { raw: String },
}

impl Row {
    fn raw(&self) -> &str {
        match self {
            Row::Turn(t) => &t.raw,
            Row::LastPrompt(lp) => &lp.raw,
            Row::Opaque { raw } => raw,
        }
    }
}

/// A parsed session: an ordered stream of rows. Re-emitting an unedited session is
/// byte-identical to the input.
#[derive(Debug, Clone)]
pub struct Session {
    /// The session uuid, read from the first row that carries a top-level `sessionId`.
    pub session_id: String,
    pub rows: Vec<Row>,
    /// Whether the source ended with a trailing newline, so re-emit is exact.
    trailing_newline: bool,
}

impl Session {
    /// Parse JSONL text into rows, preserving each line verbatim.
    pub fn parse_str(input: &str) -> Result<Session, ParseError> {
        if input.is_empty() {
            return Ok(Session {
                session_id: String::new(),
                rows: Vec::new(),
                trailing_newline: false,
            });
        }
        let trailing_newline = input.ends_with('\n');
        // A trailing '\n' produces a final empty segment that is not a row.
        let body = if trailing_newline {
            &input[..input.len() - 1]
        } else {
            input
        };

        let mut rows = Vec::new();
        let mut session_id: Option<String> = None;
        for line in body.split('\n') {
            let value: Option<Value> = serde_json::from_str(line).ok();
            if session_id.is_none() {
                if let Some(sid) = value.as_ref().and_then(session_id_of) {
                    session_id = Some(sid);
                }
            }
            rows.push(classify(line, value.as_ref()));
        }
        Ok(Session {
            session_id: session_id.unwrap_or_default(),
            rows,
            trailing_newline,
        })
    }

    /// Re-emit the session as JSONL bytes, byte-identical to the parsed source.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(row.raw());
        }
        if self.trailing_newline {
            out.push('\n');
        }
        out
    }

    /// The modeled conversation turns, in order.
    pub fn turns(&self) -> Vec<&Turn> {
        self.rows
            .iter()
            .filter_map(|r| match r {
                Row::Turn(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// The resume head: the `leafUuid` of the last `last-prompt` row, if any.
    pub fn resume_leaf(&self) -> Option<String> {
        self.rows
            .iter()
            .rev()
            .find_map(|r| match r {
                Row::LastPrompt(lp) => Some(lp.leaf_uuid.clone()),
                _ => None,
            })
    }
}

/// Write the session as `<session_id>.jsonl` into `projects_dir`, refusing to clobber
/// an existing file. Returns the new session id.
pub fn write(session: &Session, projects_dir: &Path) -> Result<String, WriteError> {
    let path = projects_dir.join(format!("{}.jsonl", session.session_id));
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(WriteError::AlreadyExists(path));
        }
        Err(e) => return Err(WriteError::Io(e)),
    };
    file.write_all(session.to_jsonl().as_bytes())?;
    Ok(session.session_id.clone())
}

/// Fork the session so it ends at `turn`: keep the prefix through that turn (and any
/// `system` row that closes it), drop everything after, re-point the resume head, and
/// mint a fresh session id. Validated by spike 03 (mid-tree cold-load).
pub fn fork_at(src: &Session, turn: &str) -> Result<Session, SurgeryError> {
    let cut = cut_through(src, turn)?;
    finish(&src.rows[..=cut], &[], turn_uuid(&src.rows[cut]), &src.session_id)
}

/// Rewind is a fork with no resume seed beyond the re-point — the same prefix operation.
pub fn rewind_to(src: &Session, turn: &str) -> Result<Session, SurgeryError> {
    fork_at(src, turn)
}

/// Inject a synthetic turn after `after`: keep the prefix through that turn, append a
/// freshly-authored `role` turn parented on the cut tip, and make it the new resume
/// head. The original tail is dropped. Validated by spike 03 Run 2.
pub fn inject(src: &Session, after: &str, role: Role, text: &str) -> Result<Session, SurgeryError> {
    let cut = cut_through(src, after)?;
    let parent = turn_uuid(&src.rows[cut]);
    let new_uuid = Uuid::new_v4().to_string();
    let synthetic = build_turn_line(role, &new_uuid, Some(&parent), text, &src.session_id);
    finish(&src.rows[..=cut], &[synthetic], new_uuid, &src.session_id)
}

/// Fork to the completed-turn boundary *before* `turn`: keep the prefix through the last
/// turn-closing `system` row preceding it, drop `turn` and everything after, and re-point
/// the resume head at that system row. The replacement prompt is NOT written here — it is
/// delivered live into the resumed branch (woland's leaf→pty path), so the new leaf is a
/// completed turn, which is the only shape `claude --resume` accepts (spike 03: every
/// resumable session's `last-prompt.leafUuid` resolves to a `turn_duration`/`away_summary`
/// system row, never a bare user turn).
pub fn fork_before(src: &Session, turn: &str) -> Result<Session, SurgeryError> {
    let idx = src
        .rows
        .iter()
        .position(|r| matches!(r, Row::Turn(t) if t.uuid == turn))
        .ok_or_else(|| SurgeryError::TurnNotFound(turn.to_string()))?;
    // the last turn-closing system row before the edited turn = the prior turn's tip
    let cut = src.rows[..idx]
        .iter()
        .rposition(|r| matches!(r, Row::Turn(t) if t.role == Role::System))
        .ok_or_else(|| SurgeryError::NoBoundaryBefore(turn.to_string()))?;
    finish(&src.rows[..=cut], &[], turn_uuid(&src.rows[cut]), &src.session_id)
}

/// Edit a turn's content in place and fork at it: keep the prefix *before* the turn,
/// re-author the turn with `text` (keeping its uuid, parent, and role), drop the tail.
///
/// ⚠ The resulting leaf is the edited turn itself. If that turn is a *user* turn, the
/// fork ends on a pending user prompt — a shape `claude --resume` will NOT load on
/// 2.1.165+ (spike 03 re-vet). Use [`fork_before`] for a resumable branch and deliver
/// the edited prompt live. This remains for static inspection / the source→fork diff
/// view, where the edited turn must be materialized to be shown.
pub fn edit_then_fork(src: &Session, turn: &str, text: &str) -> Result<Session, SurgeryError> {
    let idx = src
        .rows
        .iter()
        .position(|r| matches!(r, Row::Turn(t) if t.uuid == turn))
        .ok_or_else(|| SurgeryError::TurnNotFound(turn.to_string()))?;
    let Row::Turn(target) = &src.rows[idx] else {
        unreachable!("position matched a Turn");
    };
    let edited = build_turn_line(
        target.role,
        &target.uuid,
        target.parent_uuid.as_deref(),
        text,
        &src.session_id,
    );
    finish(&src.rows[..idx], &[edited], target.uuid.clone(), &src.session_id)
}

/// Index of the prefix cut for forking/injecting at `turn`: the turn's own row,
/// extended over any trailing `system` rows (e.g. turn_duration) that close it.
fn cut_through(src: &Session, turn: &str) -> Result<usize, SurgeryError> {
    let target = src
        .rows
        .iter()
        .position(|r| matches!(r, Row::Turn(t) if t.uuid == turn))
        .ok_or_else(|| SurgeryError::TurnNotFound(turn.to_string()))?;
    let mut cut = target;
    while let Some(Row::Turn(t)) = src.rows.get(cut + 1) {
        if t.role == Role::System {
            cut += 1;
        } else {
            break;
        }
    }
    Ok(cut)
}

fn turn_uuid(row: &Row) -> String {
    match row {
        Row::Turn(t) => t.uuid.clone(),
        _ => unreachable!("cut always lands on a turn"),
    }
}

/// The raw line of the last prefix row whose JSON `type` equals `kind` (the latest
/// `mode` / `permission-mode` / `ai-title` state row), if present.
fn last_row_of_type<'a>(prefix: &'a [Row], kind: &str) -> Option<&'a str> {
    prefix.iter().rev().map(Row::raw).find(|raw| {
        serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|v| v.get("type").and_then(Value::as_str).map(|t| t == kind))
            .unwrap_or(false)
    })
}

/// Assemble a sealed session: the kept `prefix` rows, then any `synthetic` rows, then the
/// resumable trailing block — a fresh `last-prompt` pointing at `tip`, followed by the
/// session-state rows (`mode` / `permission-mode` / `ai-title`) carried over from the
/// prefix. Real sessions end with that block; reproducing it keeps the fork resumable
/// (spike 03). The session id is rewritten old → new across the whole result.
fn finish(
    prefix: &[Row],
    synthetic: &[String],
    tip: String,
    old: &str,
) -> Result<Session, SurgeryError> {
    let new = Uuid::new_v4().to_string();
    let mut lines: Vec<String> = Vec::with_capacity(prefix.len() + synthetic.len() + 4);
    for row in prefix {
        lines.push(rewrite_session_id(row.raw(), old, &new)?);
    }
    for line in synthetic {
        lines.push(rewrite_session_id(line, old, &new)?);
    }
    lines.push(format!(
        r#"{{"type":"last-prompt","leafUuid":"{tip}","sessionId":"{new}"}}"#
    ));
    // Carry over the session-state rows that real sessions close with, in source order,
    // so the resumed branch has the same trailing shape. Absent in minimal fixtures → skipped.
    for kind in ["mode", "permission-mode", "ai-title"] {
        if let Some(raw) = last_row_of_type(prefix, kind) {
            lines.push(rewrite_session_id(raw, old, &new)?);
        }
    }

    let text = lines.join("\n") + "\n";
    Ok(Session::parse_str(&text).expect("re-parse of sealed session"))
}

/// Build a synthetic turn line in the field shape Claude Code accepts (spike 03 Run 2).
/// Built with the source `session_id`; `finish` swaps it to the new id uniformly.
fn build_turn_line(
    role: Role,
    uuid: &str,
    parent: Option<&str>,
    text: &str,
    session_id: &str,
) -> String {
    let parent = parent.map(Value::from).unwrap_or(Value::Null);
    let value = match role {
        Role::User => json!({
            "parentUuid": parent,
            "isSidechain": false,
            "promptId": Uuid::new_v4().to_string(),
            "type": "user",
            "message": { "role": "user", "content": text },
            "uuid": uuid,
            "permissionMode": "auto",
            "promptSource": "typed",
            "userType": "external",
            "sessionId": session_id,
        }),
        Role::Assistant => json!({
            "parentUuid": parent,
            "isSidechain": false,
            "message": {
                "model": "claude-opus-4-8",
                "id": format!("msg_{}", Uuid::new_v4().simple()),
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": text }],
                "stop_reason": "end_turn",
                "stop_sequence": Value::Null,
                "usage": { "input_tokens": 0, "output_tokens": 0, "service_tier": "standard" },
            },
            "requestId": format!("req_{}", Uuid::new_v4().simple()),
            "type": "assistant",
            "uuid": uuid,
            "userType": "external",
            "sessionId": session_id,
        }),
        Role::System => json!({
            "parentUuid": parent,
            "isSidechain": false,
            "type": "system",
            "subtype": "turn_duration",
            "uuid": uuid,
            "isMeta": false,
            "userType": "external",
            "sessionId": session_id,
        }),
    };
    serde_json::to_string(&value).expect("synthetic row serializes")
}

/// Classify a line into a modeled or opaque row. Invalid JSON or unmodeled `type`
/// falls through to `Opaque`, preserving the bytes.
fn classify(line: &str, value: Option<&Value>) -> Row {
    let raw = line.to_string();
    let Some(value) = value else {
        return Row::Opaque { raw };
    };
    match value.get("type").and_then(Value::as_str) {
        Some(role @ ("user" | "assistant" | "system")) => Row::Turn(Turn {
            uuid: str_field(value, "uuid").unwrap_or_default(),
            parent_uuid: str_field(value, "parentUuid"),
            is_sidechain: value
                .get("isSidechain")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            role: match role {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => Role::System,
            },
            raw,
        }),
        Some("last-prompt") => Row::LastPrompt(LastPrompt {
            leaf_uuid: str_field(value, "leafUuid").unwrap_or_default(),
            raw,
        }),
        _ => Row::Opaque { raw },
    }
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

/// The top-level `sessionId` of a parsed JSON line, if present.
fn session_id_of(value: &Value) -> Option<String> {
    value.get("sessionId")?.as_str().map(str::to_string)
}
