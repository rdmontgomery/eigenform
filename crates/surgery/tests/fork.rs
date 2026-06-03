//! fork_at: prefix the session through a chosen turn (mid-tree cold-load, spike 03),
//! re-point the resume head, and mint a fresh session id. Prefix-only — nothing after
//! the cut survives.

use eigen_surgery::{fork_at, Session, SurgeryError};

const OLD: &str = "sess-old-aaaa";

const U1: &str = "turn-u1";
const A1: &str = "turn-a1";
const S1: &str = "turn-s1";
const U2: &str = "turn-u2";
const A2: &str = "turn-a2";
const S2: &str = "turn-s2";

/// Two exchanges: U1->A1->S1, U2->A2->S2, bracketed by last-prompt rows.
fn two_turns() -> Session {
    let text = [
        format!(r#"{{"type":"last-prompt","leafUuid":"pre","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"user","uuid":"{U1}","parentUuid":null,"isSidechain":false,"sessionId":"{OLD}","message":{{"role":"user","content":"one"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A1}","parentUuid":"{U1}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"assistant","content":[{{"type":"text","text":"a-one"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S1}","parentUuid":"{A1}","isSidechain":false,"subtype":"turn_duration","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"user","uuid":"{U2}","parentUuid":"{S1}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"user","content":"two"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A2}","parentUuid":"{U2}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"assistant","content":[{{"type":"text","text":"a-two"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S2}","parentUuid":"{A2}","isSidechain":false,"subtype":"turn_duration","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"last-prompt","lastPrompt":"two","leafUuid":"{S2}","sessionId":"{OLD}"}}"#),
    ]
    .join("\n")
        + "\n";
    Session::parse_str(&text).unwrap()
}

fn turn_uuids(s: &Session) -> Vec<String> {
    s.turns().iter().map(|t| t.uuid.clone()).collect()
}

#[test]
fn fork_at_assistant_keeps_through_its_trailing_system_row() {
    let forked = fork_at(&two_turns(), A1).expect("fork");
    assert_eq!(turn_uuids(&forked), vec![U1, A1, S1]);
}

#[test]
fn fork_at_drops_everything_after_the_cut() {
    let forked = fork_at(&two_turns(), A1).unwrap();
    let jsonl = forked.to_jsonl();
    for dropped in [U2, A2, S2] {
        assert!(!jsonl.contains(dropped), "{dropped} should be gone");
    }
}

#[test]
fn fork_at_repoints_the_resume_head_to_the_new_tip() {
    let forked = fork_at(&two_turns(), A1).unwrap();
    assert_eq!(forked.resume_leaf().as_deref(), Some(S1));
}

#[test]
fn fork_at_mints_a_fresh_session_id_everywhere() {
    let forked = fork_at(&two_turns(), A1).unwrap();
    assert_ne!(forked.session_id, OLD);
    assert!(!forked.session_id.is_empty());
    let jsonl = forked.to_jsonl();
    assert!(!jsonl.contains(OLD), "old session id fully gone");
    assert!(jsonl.contains(&forked.session_id));
}

#[test]
fn fork_at_a_user_turn_stops_before_the_next_assistant() {
    let forked = fork_at(&two_turns(), U2).unwrap();
    assert_eq!(turn_uuids(&forked), vec![U1, A1, S1, U2]);
    assert_eq!(forked.resume_leaf().as_deref(), Some(U2));
}

#[test]
fn fork_at_unknown_turn_is_an_error() {
    let err = fork_at(&two_turns(), "nope").unwrap_err();
    assert!(matches!(err, SurgeryError::TurnNotFound(_)));
}

#[test]
fn fork_at_a_non_turn_uuid_is_an_error() {
    // "pre" is a last-prompt leaf pointer, not a turn.
    assert!(matches!(
        fork_at(&two_turns(), "pre").unwrap_err(),
        SurgeryError::TurnNotFound(_)
    ));
}
