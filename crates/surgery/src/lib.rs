//! eigen-surgery: context surgery on Claude Code session JSONL files.
//!
//! Passthrough model: only user/assistant/system turns and `last-prompt` rows are
//! modeled; every other row is retained as a verbatim line. See
//! `docs/plans/2026-06-03-surgery-crate-design.md`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ParseError {}

#[derive(Debug, Error)]
pub enum SurgeryError {
    /// No modeled turn (user/assistant/system) carries the requested uuid.
    #[error("no turn with uuid `{0}` in session")]
    TurnNotFound(String),
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

/// Replace `old` with `new` in one JSONL line, but only if every occurrence of `old`
/// is a `sessionId` value. Byte-faithful everywhere else. A line not containing `old`
/// is returned unchanged. See spike 07.
pub fn rewrite_session_id(line: &str, old: &str, new: &str) -> Result<String, RewriteError> {
    let total = line.matches(old).count();
    if total == 0 {
        return Ok(line.to_string());
    }
    let bail = || RewriteError::StrayOccurrence {
        old: old.to_string(),
        line: line.to_string(),
    };
    // Count the occurrences that are legitimate: a JSON string value exactly equal to
    // `old`, sitting at a key named `sessionId`. Anything else (a different key, a
    // substring inside a larger string) is a stray.
    let value: Value = serde_json::from_str(line).map_err(|_| bail())?;
    let mut session_field_hits = 0;
    let mut stray = false;
    walk_strings(&value, None, &mut |key, s| {
        if s == old {
            if key == Some("sessionId") {
                session_field_hits += 1;
            } else {
                stray = true;
            }
        }
    });
    // A substring occurrence (old buried inside a larger token) shows up in `total` but
    // never as an exact string value, so the counts diverge — also a stray.
    if stray || total != session_field_hits {
        return Err(bail());
    }
    Ok(line.replace(old, new))
}

/// Visit every string value in a JSON tree with the key it was found under (None for
/// array elements / the root).
fn walk_strings(value: &Value, key: Option<&str>, f: &mut impl FnMut(Option<&str>, &str)) {
    match value {
        Value::String(s) => f(key, s),
        Value::Array(items) => {
            for item in items {
                walk_strings(item, None, f);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                walk_strings(v, Some(k), f);
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
}

impl Session {
    /// Parse JSONL text into rows, preserving each line verbatim.
    pub fn parse_str(input: &str) -> Result<Session, ParseError> {
        let mut rows = Vec::new();
        let mut session_id: Option<String> = None;
        for line in input.split('\n') {
            let value: Option<Value> = serde_json::from_str(line).ok();
            if session_id.is_none() {
                if let Some(sid) = value.as_ref().and_then(session_id_of) {
                    session_id = Some(sid);
                }
            }
            rows.push(classify(line, value.as_ref()));
        }
        // A trailing '\n' produces a final empty segment; to_jsonl re-adds the newline
        // after every row, so drop it here to round-trip exactly.
        if input.ends_with('\n') {
            rows.pop();
        }
        Ok(Session {
            session_id: session_id.unwrap_or_default(),
            rows,
        })
    }

    /// Re-emit the session as JSONL bytes.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            out.push_str(row.raw());
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
    let target = src
        .rows
        .iter()
        .position(|r| matches!(r, Row::Turn(t) if t.uuid == turn))
        .ok_or_else(|| SurgeryError::TurnNotFound(turn.to_string()))?;
    // Extend the cut over any trailing `system` rows (e.g. turn_duration) that belong
    // to the kept turn, stopping before the next user/assistant turn.
    let mut cut = target;
    while let Some(Row::Turn(t)) = src.rows.get(cut + 1) {
        if t.role == Role::System {
            cut += 1;
        } else {
            break;
        }
    }
    seal_prefix(src, cut)
}

/// Rewind is a fork with no resume seed beyond the re-point — the same prefix operation.
pub fn rewind_to(src: &Session, turn: &str) -> Result<Session, SurgeryError> {
    fork_at(src, turn)
}

/// Keep `src.rows[..=cut]`, append a fresh `last-prompt` pointing at the cut row, and
/// rewrite the session id across the whole result.
fn seal_prefix(src: &Session, cut: usize) -> Result<Session, SurgeryError> {
    let tip = match &src.rows[cut] {
        Row::Turn(t) => t.uuid.clone(),
        _ => unreachable!("cut always lands on a turn"),
    };
    let old = &src.session_id;
    let new = Uuid::new_v4().to_string();

    let mut lines: Vec<String> = Vec::with_capacity(cut + 2);
    for row in &src.rows[..=cut] {
        lines.push(rewrite_session_id(row.raw(), old, &new)?);
    }
    lines.push(format!(
        r#"{{"type":"last-prompt","leafUuid":"{tip}","sessionId":"{new}"}}"#
    ));

    let text = lines.join("\n") + "\n";
    Ok(Session::parse_str(&text).expect("re-parse of sealed prefix"))
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
