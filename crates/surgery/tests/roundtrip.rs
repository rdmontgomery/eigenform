//! Passthrough fidelity: parsing then re-emitting an unedited session must return
//! the exact bytes we read. This is the load-bearing invariant for surgery on rows
//! we don't model (attachments, pr-link, queue-operation, future types).

use eigenform_surgery::Session;

/// A compact session mixing every shape the parser must survive:
/// - opaque rows WITH a sessionId (mode, attachment, pr-link)
/// - an opaque row WITHOUT a sessionId (file-history-snapshot)
/// - the three turn roles (user, assistant, system)
/// - typed last-prompt rows (leading placeholder + trailing, with a leafUuid)
const MIXED: &str = concat!(
    r#"{"type":"last-prompt","leafUuid":"00000000-0000-4000-8000-0000000000s1","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#, "\n",
    r#"{"type":"mode","mode":"normal","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#, "\n",
    r#"{"type":"attachment","uuid":"00000000-0000-4000-8000-0000000000a1","parentUuid":null,"sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","attachment":{"type":"hook_success"}}"#, "\n",
    r#"{"type":"file-history-snapshot","messageId":"m1","snapshot":{}}"#, "\n",
    r#"{"type":"user","uuid":"00000000-0000-4000-8000-0000000000u1","parentUuid":"00000000-0000-4000-8000-0000000000a1","isSidechain":false,"sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","message":{"role":"user","content":"howdy"}}"#, "\n",
    r#"{"type":"assistant","uuid":"00000000-0000-4000-8000-00000000as1","parentUuid":"00000000-0000-4000-8000-0000000000u1","isSidechain":false,"sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#, "\n",
    r#"{"type":"system","uuid":"00000000-0000-4000-8000-0000000000s1","parentUuid":"00000000-0000-4000-8000-00000000as1","isSidechain":false,"subtype":"turn_duration","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#, "\n",
    r#"{"type":"pr-link","url":"https://example.test/pr/1","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#, "\n",
    r#"{"type":"last-prompt","lastPrompt":"howdy","leafUuid":"00000000-0000-4000-8000-0000000000s1","sessionId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}"#, "\n",
);

#[test]
fn parse_then_emit_is_byte_identical_for_a_mixed_session() {
    let session = Session::parse_str(MIXED).expect("parse should succeed");
    assert_eq!(session.to_jsonl(), MIXED);
}

#[test]
fn round_trip_preserves_a_missing_trailing_newline() {
    // A file that doesn't end in '\n' must come back exactly as it went in.
    let no_newline = MIXED.trim_end_matches('\n');
    let session = Session::parse_str(no_newline).expect("parse");
    assert_eq!(session.to_jsonl(), no_newline);
}

#[test]
fn round_trip_of_empty_input_is_empty() {
    assert_eq!(Session::parse_str("").unwrap().to_jsonl(), "");
}
