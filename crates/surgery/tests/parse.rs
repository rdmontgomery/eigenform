//! Structural extraction: parse must surface the fields surgery reasons about,
//! without disturbing byte-for-byte round-trip (covered in roundtrip.rs).

use eigenform_surgery::{Role, Session};

const SID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

const U1: &str = "00000000-0000-4000-8000-0000000000u1";
const A1: &str = "00000000-0000-4000-8000-00000000as1";
const S1: &str = "00000000-0000-4000-8000-0000000000s1";

/// A one-turn conversation: user -> assistant -> system(turn_duration), wrapped in a
/// leading mode row and a trailing last-prompt pointing at the system row.
fn convo() -> String {
    [
        format!(r#"{{"type":"mode","mode":"normal","sessionId":"{SID}"}}"#),
        format!(r#"{{"type":"user","uuid":"{U1}","parentUuid":null,"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"hi"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A1}","parentUuid":"{U1}","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"text","text":"yo"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S1}","parentUuid":"{A1}","isSidechain":false,"subtype":"turn_duration","sessionId":"{SID}"}}"#),
        format!(r#"{{"type":"last-prompt","lastPrompt":"hi","leafUuid":"{S1}","sessionId":"{SID}"}}"#),
    ]
    .join("\n")
        + "\n"
}

fn fixture() -> String {
    [
        format!(r#"{{"type":"mode","mode":"normal","sessionId":"{SID}"}}"#),
        r#"{"type":"file-history-snapshot","messageId":"m1","snapshot":{}}"#.to_string(),
        format!(r#"{{"type":"user","uuid":"00000000-0000-4000-8000-0000000000u1","parentUuid":null,"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"hi"}}}}"#),
    ]
    .join("\n")
        + "\n"
}

#[test]
fn parse_extracts_the_session_id() {
    let session = Session::parse_str(&fixture()).expect("parse");
    assert_eq!(session.session_id, SID);
}

#[test]
fn parse_classifies_the_three_turn_roles_with_uuid_and_parent() {
    let session = Session::parse_str(&convo()).expect("parse");
    let turns = session.turns();
    assert_eq!(turns.len(), 3, "user, assistant, system are turns; mode + last-prompt are not");

    assert_eq!(turns[0].role, Role::User);
    assert_eq!(turns[0].uuid, U1);
    assert_eq!(turns[0].parent_uuid, None);
    assert!(!turns[0].is_sidechain);

    assert_eq!(turns[1].role, Role::Assistant);
    assert_eq!(turns[1].uuid, A1);
    assert_eq!(turns[1].parent_uuid.as_deref(), Some(U1));

    assert_eq!(turns[2].role, Role::System);
    assert_eq!(turns[2].uuid, S1);
    assert_eq!(turns[2].parent_uuid.as_deref(), Some(A1));
}

#[test]
fn parse_reads_the_trailing_last_prompt_leaf() {
    let session = Session::parse_str(&convo()).expect("parse");
    assert_eq!(session.resume_leaf(), Some(S1.to_string()));
}
