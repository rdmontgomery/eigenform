//! session_ref / tail-peek: recency = last timestamped row (else mtime); title = last
//! ai-title (else last-prompt snippet). Byte-stream tail read with size escalation.

use chrono::{DateTime, Utc};
use eigen_forest::{session_ref, SessionStub};

fn write_session(lines: &[String]) -> (tempfile::TempDir, SessionStub) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sess.jsonl");
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    let stub = SessionStub {
        uuid: "sess".to_string(),
        path,
        cwd: "/home/me/p".into(),
    };
    (dir, stub)
}

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn recency_is_the_last_timestamped_row_past_trailing_state_rows() {
    let (_d, stub) = write_session(&[
        r#"{"type":"user","uuid":"u1","timestamp":"2026-06-01T10:00:00Z","sessionId":"sess"}"#.into(),
        r#"{"type":"assistant","uuid":"a1","timestamp":"2026-06-01T10:00:05Z","sessionId":"sess"}"#.into(),
        r#"{"type":"system","uuid":"s1","subtype":"turn_duration","timestamp":"2026-06-01T10:00:06Z","sessionId":"sess"}"#.into(),
        r#"{"type":"last-prompt","leafUuid":"s1","sessionId":"sess"}"#.into(),
        r#"{"type":"ai-title","aiTitle":"Hello World","sessionId":"sess"}"#.into(),
        r#"{"type":"mode","mode":"normal","sessionId":"sess"}"#.into(),
    ]);
    let r = session_ref(&stub);
    assert_eq!(r.recency, ts("2026-06-01T10:00:06Z"));
    assert_eq!(r.title.as_deref(), Some("Hello World"));
}

#[test]
fn title_falls_back_to_last_prompt_snippet() {
    let (_d, stub) = write_session(&[
        r#"{"type":"user","uuid":"u1","timestamp":"2026-06-01T10:00:00Z","sessionId":"sess"}"#.into(),
        r#"{"type":"last-prompt","lastPrompt":"do the thing","leafUuid":"u1","sessionId":"sess"}"#.into(),
    ]);
    let r = session_ref(&stub);
    assert_eq!(r.title.as_deref(), Some("do the thing"));
}

#[test]
fn recency_falls_back_to_mtime_when_no_timestamps() {
    let (_d, stub) = write_session(&[
        r#"{"type":"mode","mode":"normal","sessionId":"sess"}"#.into(),
        r#"{"type":"last-prompt","leafUuid":"x","sessionId":"sess"}"#.into(),
    ]);
    let mtime: DateTime<Utc> =
        std::fs::metadata(&stub.path).unwrap().modified().unwrap().into();
    let r = session_ref(&stub);
    assert_eq!(r.recency, mtime);
}

#[test]
fn escalates_past_an_oversized_final_turn() {
    // A 70 KB assistant line is the last timestamped row; the 64 KB window can't see its
    // start, so peek must grow the window to find the timestamp.
    let big = "x".repeat(70_000);
    let (_d, stub) = write_session(&[
        r#"{"type":"user","uuid":"u1","timestamp":"2026-06-01T09:00:00Z","sessionId":"sess"}"#.into(),
        format!(r#"{{"type":"assistant","uuid":"a1","timestamp":"2026-06-02T00:00:00Z","sessionId":"sess","message":{{"role":"assistant","content":[{{"type":"text","text":"{big}"}}]}}}}"#),
        r#"{"type":"last-prompt","leafUuid":"a1","sessionId":"sess"}"#.into(),
        r#"{"type":"ai-title","aiTitle":"Big","sessionId":"sess"}"#.into(),
    ]);
    let r = session_ref(&stub);
    assert_eq!(r.recency, ts("2026-06-02T00:00:00Z"));
    assert_eq!(r.title.as_deref(), Some("Big"));
}
