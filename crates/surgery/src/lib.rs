//! eigen-surgery: context surgery on Claude Code session JSONL files.
//!
//! Passthrough model: only user/assistant/system turns and `last-prompt` rows are
//! modeled; every other row is retained as a verbatim line. See
//! `docs/plans/2026-06-03-surgery-crate-design.md`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {}

/// The top-level `sessionId` of a JSONL line, if it is valid JSON carrying one.
fn session_id_of(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value.get("sessionId")?.as_str().map(str::to_string)
}

/// One line of a session JSONL. Unmodeled rows are kept as verbatim bytes so surgery
/// never perturbs a row it doesn't understand.
#[derive(Debug, Clone)]
pub enum Row {
    Opaque { raw: String },
}

impl Row {
    fn raw(&self) -> &str {
        match self {
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
            if session_id.is_none() {
                if let Some(sid) = session_id_of(line) {
                    session_id = Some(sid);
                }
            }
            rows.push(Row::Opaque {
                raw: line.to_string(),
            });
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
}
