//! inject: prefix through a chosen turn, then append a synthetic turn authored by us
//! as the new resume head. Validated end-to-end by spike 03 Run 2 (a fully synthetic
//! turn loaded and was recalled).

use eigen_surgery::{inject, Role, Session, SurgeryError};

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

#[test]
fn inject_appends_a_synthetic_turn_parented_on_the_cut_tip() {
    let injected = inject(&two_turns(), A1, Role::User, "synthetic instruction").unwrap();
    let last = *injected.turns().last().unwrap();
    assert_eq!(last.role, Role::User);
    assert_eq!(last.parent_uuid.as_deref(), Some(S1), "parents on A1's trailing system row");
}

#[test]
fn inject_drops_the_original_tail() {
    let injected = inject(&two_turns(), A1, Role::User, "x").unwrap();
    let jsonl = injected.to_jsonl();
    for dropped in [U2, A2, S2] {
        assert!(!jsonl.contains(dropped), "{dropped} should be gone");
    }
}

#[test]
fn inject_repoints_resume_head_to_the_synthetic_turn() {
    let injected = inject(&two_turns(), A1, Role::User, "x").unwrap();
    let last_uuid = injected.turns().last().unwrap().uuid.clone();
    assert_eq!(injected.resume_leaf(), Some(last_uuid));
}

#[test]
fn inject_mints_a_fresh_session_id() {
    let injected = inject(&two_turns(), A1, Role::User, "x").unwrap();
    assert_ne!(injected.session_id, OLD);
    assert!(!injected.to_jsonl().contains(OLD));
}

#[test]
fn inject_escapes_special_characters_in_content() {
    let tricky = "quote \" brace } newline \n end";
    let injected = inject(&two_turns(), A1, Role::User, tricky).unwrap();
    let last = *injected.turns().last().unwrap();
    let v: serde_json::Value = serde_json::from_str(last.raw()).expect("synthetic row is valid JSON");
    assert_eq!(v["message"]["content"], serde_json::json!(tricky));
}

#[test]
fn inject_an_assistant_turn_uses_a_content_block() {
    let injected = inject(&two_turns(), U1, Role::Assistant, "as if I said this").unwrap();
    let last = *injected.turns().last().unwrap();
    assert_eq!(last.role, Role::Assistant);
    let v: serde_json::Value = serde_json::from_str(last.raw()).unwrap();
    assert_eq!(v["message"]["content"][0]["type"], "text");
    assert_eq!(v["message"]["content"][0]["text"], "as if I said this");
}

#[test]
fn inject_after_unknown_turn_is_an_error() {
    assert!(matches!(
        inject(&two_turns(), "nope", Role::User, "x").unwrap_err(),
        SurgeryError::TurnNotFound(_)
    ));
}
