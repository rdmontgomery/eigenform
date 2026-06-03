//! session_view: build a View from a parsed session — group by exchange, glyph by role,
//! one-line truncated previews, system rows shown muted with duration, resume leaf marked.

use eigen_render::{render_text, session_view};
use eigen_surgery::Session;

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
