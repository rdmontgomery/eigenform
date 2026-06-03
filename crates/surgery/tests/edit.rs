//! edit_then_fork: rewrite a turn's content, keep the prefix up to it, drop the tail,
//! and fork. The edited turn keeps its identity (uuid + parent) and becomes the new
//! resume head — so resuming replays history as if you'd said the edited thing.

use eigen_surgery::{edit_then_fork, Session, SurgeryError};

const OLD: &str = "sess-old-aaaa";
const U1: &str = "turn-u1";
const A1: &str = "turn-a1";
const S1: &str = "turn-s1";
const U2: &str = "turn-u2";
const A2: &str = "turn-a2";
const S2: &str = "turn-s2";

fn two_turns() -> Session {
    let text = [
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

fn last_value(s: &Session) -> serde_json::Value {
    serde_json::from_str(s.turns().last().unwrap().raw()).unwrap()
}

#[test]
fn edit_replaces_content_keeps_uuid_and_drops_tail() {
    let edited = edit_then_fork(&two_turns(), U2, "two, but better").unwrap();

    let last = *edited.turns().last().unwrap();
    assert_eq!(last.uuid, U2, "edited turn keeps its identity");
    assert_eq!(last_value(&edited)["message"]["content"], "two, but better");

    let jsonl = edited.to_jsonl();
    for dropped in [A2, S2] {
        assert!(!jsonl.contains(dropped), "{dropped} should be gone");
    }
}

#[test]
fn edit_keeps_the_turns_parent() {
    let edited = edit_then_fork(&two_turns(), U2, "x").unwrap();
    assert_eq!(last_value(&edited)["parentUuid"], S1);
}

#[test]
fn edit_repoints_resume_head_and_mints_new_id() {
    let edited = edit_then_fork(&two_turns(), U2, "x").unwrap();
    assert_eq!(edited.resume_leaf().as_deref(), Some(U2));
    assert_ne!(edited.session_id, OLD);
    assert!(!edited.to_jsonl().contains(OLD));
}

#[test]
fn edit_the_first_turn_preserves_null_parent() {
    let edited = edit_then_fork(&two_turns(), U1, "fresh start").unwrap();
    let last = *edited.turns().last().unwrap();
    assert_eq!(last.uuid, U1);
    assert!(last.parent_uuid.is_none());
    assert!(last_value(&edited)["parentUuid"].is_null());
}

#[test]
fn edit_derives_the_role_from_the_target_turn() {
    use eigen_surgery::Role;
    // A1 is an assistant turn; editing it must keep an assistant content block.
    let edited = edit_then_fork(&two_turns(), A1, "as if I'd said this").unwrap();
    let last = *edited.turns().last().unwrap();
    assert_eq!(last.role, Role::Assistant);
    assert_eq!(last_value(&edited)["message"]["content"][0]["text"], "as if I'd said this");
}

#[test]
fn edit_unknown_turn_is_an_error() {
    assert!(matches!(
        edit_then_fork(&two_turns(), "nope", "x").unwrap_err(),
        SurgeryError::TurnNotFound(_)
    ));
}
