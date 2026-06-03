//! eigen-surgery: context surgery on Claude Code session JSONL files.
//!
//! Passthrough model: only user/assistant/system turns and `last-prompt` rows are
//! modeled; every other row is retained as a verbatim line. See
//! `docs/plans/2026-06-03-surgery-crate-design.md`.

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {}

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
