//! Field-targeted session-id swap: replace ONLY values sitting at a `sessionId` key,
//! anywhere in the row's JSON tree. Substring occurrences inside content (tool output
//! that printed the session's own id — see spike 07 finding 4) are left untouched.
//! A guard bails only on the `exact-other` case: the old id as the full value of a
//! non-`sessionId` key.

use eigenform_surgery::rewrite_session_id;

const OLD: &str = "old-session-uuid-0000";
const NEW: &str = "new-session-uuid-1111";

#[test]
fn swaps_the_session_id_in_an_opaque_row() {
    let line = format!(r#"{{"type":"mode","mode":"normal","sessionId":"{OLD}"}}"#);
    let out = rewrite_session_id(&line, OLD, NEW).expect("clean swap");
    assert_eq!(out, format!(r#"{{"type":"mode","mode":"normal","sessionId":"{NEW}"}}"#));
}

#[test]
fn preserves_old_id_appearing_as_substring_in_message_content() {
    // A tool printed the session's own <id>.jsonl filename into content. The sessionId
    // field must flip; the content occurrence must NOT (spike 07 finding 4).
    let line = format!(
        r#"{{"type":"user","uuid":"u1","sessionId":"{OLD}","message":{{"role":"user","content":"see {OLD}.jsonl"}}}}"#
    );
    let out = rewrite_session_id(&line, OLD, NEW).expect("swap, not bail");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["sessionId"], NEW, "sessionId field swapped");
    assert_eq!(
        v["message"]["content"],
        format!("see {OLD}.jsonl"),
        "content substring left intact"
    );
}

#[test]
fn rewrites_a_nested_session_id_too() {
    // Whatever the nesting, every value at a `sessionId` key flips.
    let line = format!(r#"{{"type":"x","sessionId":"{OLD}","meta":{{"sessionId":"{OLD}"}}}}"#);
    let out = rewrite_session_id(&line, OLD, NEW).unwrap();
    assert!(!out.contains(OLD));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["sessionId"], NEW);
    assert_eq!(v["meta"]["sessionId"], NEW);
}

#[test]
fn bails_when_old_id_is_an_exact_value_under_a_non_session_key() {
    let line = format!(r#"{{"type":"x","resumedFrom":"{OLD}","sessionId":"{OLD}"}}"#);
    assert!(rewrite_session_id(&line, OLD, NEW).is_err());
}

#[test]
fn leaves_a_row_without_the_old_id_untouched() {
    let line = r#"{"type":"file-history-snapshot","messageId":"m1","snapshot":{}}"#;
    let out = rewrite_session_id(line, OLD, NEW).unwrap();
    assert_eq!(out, line);
}
