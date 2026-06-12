//! session_view: build a View from a parsed session — group by exchange, glyph by role,
//! one-line truncated previews, system rows shown muted with duration, resume leaf marked.

use eigenform_render::{render_text, session_view};
use eigenform_surgery::Session;

const SID: &str = "abcd1234-0000-4000-8000-000000000000";

fn session_from(rows: &[String]) -> Session {
    let text = rows.join("\n") + "\n";
    Session::parse_str(&text).unwrap()
}

fn user(uuid: &str, parent: &str, content: &str) -> String {
    let parent = if parent.is_empty() { "null".to_string() } else { format!("\"{parent}\"") };
    let content = serde_json::to_string(content).unwrap();
    format!(r#"{{"type":"user","uuid":"{uuid}","parentUuid":{parent},"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":{content}}}}}"#)
}
fn assistant(uuid: &str, parent: &str, text: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(r#"{{"type":"assistant","uuid":"{uuid}","parentUuid":"{parent}","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"text","text":{text}}}]}}}}"#)
}
fn system(uuid: &str, parent: &str, ms: u64) -> String {
    format!(r#"{{"type":"system","uuid":"{uuid}","parentUuid":"{parent}","isSidechain":false,"subtype":"turn_duration","durationMs":{ms},"sessionId":"{SID}"}}"#)
}
fn last_prompt(leaf: &str) -> String {
    format!(r#"{{"type":"last-prompt","leafUuid":"{leaf}","sessionId":"{SID}"}}"#)
}

#[test]
fn groups_by_exchange_with_leaf_marked() {
    let s = session_from(&[
        user("U1", "", "hello there"),
        assistant("A1", "U1", "hi back"),
        system("S1", "A1", 4200),
        last_prompt("S1"),
    ]);
    let expected = "\
session abcd1234 · 1 exchange
└─ ● user       hello there
   ├─ ◇ assistant  hi back
   └─ · system     4.2s  ← leaf
";
    assert_eq!(render_text(&session_view(&s)), expected);
}

#[test]
fn multiple_exchanges_are_top_level_siblings() {
    let s = session_from(&[
        user("U1", "", "one"),
        assistant("A1", "U1", "a-one"),
        system("S1", "A1", 1000),
        user("U2", "S1", "two"),
        assistant("A2", "U2", "a-two"),
        system("S2", "A2", 2000),
        last_prompt("S2"),
    ]);
    let out = render_text(&session_view(&s));
    assert!(out.starts_with("session abcd1234 · 2 exchanges\n"), "got: {out}");
    assert!(out.contains("├─ ● user       one"));
    assert!(out.contains("└─ ● user       two"));
    assert!(out.trim_end().ends_with("← leaf"));
}

/// An assistant row carrying only a thinking block (no text) — real sessions split
/// thinking and text into separate rows.
fn assistant_thinking(uuid: &str, parent: &str) -> String {
    format!(r#"{{"type":"assistant","uuid":"{uuid}","parentUuid":"{parent}","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"thinking","thinking":"hmm"}}]}}}}"#)
}
/// A non-turn_duration system row (e.g. an init/meta marker) — carries no durationMs.
fn system_meta(uuid: &str, parent: &str) -> String {
    format!(r#"{{"type":"system","uuid":"{uuid}","parentUuid":"{parent}","isSidechain":false,"subtype":"init","sessionId":"{SID}"}}"#)
}

#[test]
fn thinking_only_assistant_rows_are_omitted() {
    let s = session_from(&[
        user("U1", "", "hi"),
        assistant_thinking("A0", "U1"),
        assistant("A1", "A0", "real answer"),
        last_prompt("A1"),
    ]);
    let out = render_text(&session_view(&s));
    assert_eq!(out.matches("◇ assistant").count(), 1, "only the text row shows:\n{out}");
    assert!(out.contains("real answer"));
}

#[test]
fn system_rows_without_duration_are_omitted() {
    let s = session_from(&[
        user("U1", "", "hi"),
        assistant("A1", "U1", "yo"),
        system_meta("M1", "A1"),
        last_prompt("A1"),
    ]);
    let out = render_text(&session_view(&s));
    assert!(!out.contains("· system"), "non-duration system hidden:\n{out}");
}

#[test]
fn leaf_marker_falls_back_to_last_visible_turn_when_the_leaf_is_hidden() {
    // The real leaf points at a hidden meta system row; the marker should land on the
    // last visible turn (the assistant) instead of vanishing.
    let s = session_from(&[
        user("U1", "", "hi"),
        assistant("A1", "U1", "yo"),
        system_meta("M1", "A1"),
        last_prompt("M1"),
    ]);
    let out = render_text(&session_view(&s));
    let row = out.lines().find(|l| l.contains("← leaf")).expect("leaf marked somewhere");
    assert!(row.contains("assistant"), "leaf fell back to assistant:\n{out}");
}

#[test]
fn long_content_is_truncated_to_one_line() {
    let long = "x".repeat(200);
    let s = session_from(&[user("U1", "", &long), last_prompt("U1")]);
    let out = render_text(&session_view(&s));
    let row = out.lines().find(|l| l.contains("user")).unwrap();
    assert!(row.contains('…'), "expected ellipsis, got: {row}");
    assert!(!row.contains("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"), "not truncated: {row}");
    assert!(row.chars().count() < 90, "row too long: {row}");
}

#[test]
fn whitespace_in_content_is_collapsed_to_single_line() {
    let s = session_from(&[user("U1", "", "line1\nline2\t  line3"), last_prompt("U1")]);
    let out = render_text(&session_view(&s));
    assert!(out.contains("line1 line2 line3"), "got: {out}");
    // exactly one body line for the single turn (plus title)
    assert_eq!(out.lines().count(), 2);
}
