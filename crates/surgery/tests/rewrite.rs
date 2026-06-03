//! Guarded session-id swap: replace the old session uuid with a new one across a row,
//! byte-faithful everywhere else — but REFUSE if the old uuid appears anywhere that
//! isn't a `sessionId` value (a blind swap there would corrupt content). Spike 07
//! verified the guard passes on every real session; these tests pin the contract.

use eigen_surgery::rewrite_session_id;

const OLD: &str = "old-session-uuid-0000";
const NEW: &str = "new-session-uuid-1111";

#[test]
fn swaps_the_session_id_in_an_opaque_row() {
    let line = format!(r#"{{"type":"mode","mode":"normal","sessionId":"{OLD}"}}"#);
    let out = rewrite_session_id(&line, OLD, NEW).expect("clean swap");
    assert_eq!(out, format!(r#"{{"type":"mode","mode":"normal","sessionId":"{NEW}"}}"#));
}

#[test]
fn is_byte_faithful_except_the_swapped_id() {
    // Two sessionId occurrences, surrounding structure untouched.
    let line = format!(
        r#"{{"type":"last-prompt","lastPrompt":"keep me","leafUuid":"some-turn-uuid","sessionId":"{OLD}"}}"#
    );
    let out = rewrite_session_id(&line, OLD, NEW).unwrap();
    assert!(out.contains(NEW));
    assert!(!out.contains(OLD));
    assert_eq!(out, line.replace(OLD, NEW));
}

#[test]
fn bails_when_old_uuid_appears_inside_message_content() {
    // User pasted the session uuid into their prompt — a blind replace would mangle it.
    let line = format!(
        r#"{{"type":"user","uuid":"u1","sessionId":"{OLD}","message":{{"role":"user","content":"my session is {OLD}"}}}}"#
    );
    assert!(rewrite_session_id(&line, OLD, NEW).is_err());
}

#[test]
fn bails_when_old_uuid_is_an_exact_value_under_a_non_session_key() {
    let line = format!(r#"{{"type":"x","note":"{OLD}","sessionId":"{OLD}"}}"#);
    assert!(rewrite_session_id(&line, OLD, NEW).is_err());
}

#[test]
fn leaves_a_row_without_the_old_id_untouched() {
    let line = r#"{"type":"file-history-snapshot","messageId":"m1","snapshot":{}}"#;
    let out = rewrite_session_id(line, OLD, NEW).unwrap();
    assert_eq!(out, line);
}
