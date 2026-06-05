//! fork_before: rewind the session to the completed-turn boundary BEFORE a turn, drop
//! that turn and everything after, and re-point the resume head at the prior turn's
//! closing `system` row. The edited prompt is delivered live into the resumed branch
//! (woland), not written here — so the new leaf is always a completed turn, the only
//! shape `claude --resume` accepts (spike 03).

use eigen_surgery::{fork_before, Session, SurgeryError};

const OLD: &str = "sess-old-aaaa";
const U1: &str = "turn-u1";
const A1: &str = "turn-a1";
const S1: &str = "turn-s1";
const U2: &str = "turn-u2";
const A2: &str = "turn-a2";
const S2: &str = "turn-s2";

/// Two complete exchanges: U1→A1→S1, U2→A2→S2. Real sessions carry `mode`/`permission-mode`
/// state rows in the leading scaffolding (and recurring) — included here so the kept
/// prefix has them to carry into the resumable trailing block.
fn two_turns() -> Session {
    let text = [
        format!(r#"{{"type":"mode","mode":"normal","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"permission-mode","permissionMode":"auto","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"user","uuid":"{U1}","parentUuid":null,"isSidechain":false,"sessionId":"{OLD}","message":{{"role":"user","content":"one"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A1}","parentUuid":"{U1}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"assistant","content":[{{"type":"text","text":"a-one"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S1}","parentUuid":"{A1}","isSidechain":false,"subtype":"turn_duration","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"user","uuid":"{U2}","parentUuid":"{S1}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"user","content":"two"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A2}","parentUuid":"{U2}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"assistant","content":[{{"type":"text","text":"a-two"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S2}","parentUuid":"{A2}","isSidechain":false,"subtype":"turn_duration","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"mode","mode":"normal","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"permission-mode","permissionMode":"auto","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"last-prompt","leafUuid":"{S2}","sessionId":"{OLD}"}}"#),
    ]
    .join("\n")
        + "\n";
    Session::parse_str(&text).unwrap()
}

#[test]
fn forks_to_the_prior_system_boundary_and_drops_the_turn_and_tail() {
    let forked = fork_before(&two_turns(), U2).unwrap();
    // resume head is the completed-turn system row before U2 — never a bare user turn
    assert_eq!(forked.resume_leaf().as_deref(), Some(S1));
    let jsonl = forked.to_jsonl();
    assert!(jsonl.contains(U1) && jsonl.contains(A1) && jsonl.contains(S1), "prefix kept");
    for dropped in [U2, A2, S2] {
        assert!(!jsonl.contains(dropped), "{dropped} should be gone");
    }
}

#[test]
fn carries_over_the_trailing_state_block() {
    let jsonl = fork_before(&two_turns(), U2).unwrap().to_jsonl();
    assert!(jsonl.contains(r#""type":"mode""#), "mode row re-emitted");
    assert!(jsonl.contains(r#""type":"permission-mode""#), "permission-mode row re-emitted");
}

#[test]
fn mints_a_fresh_session_id_everywhere() {
    let forked = fork_before(&two_turns(), U2).unwrap();
    assert_ne!(forked.session_id, OLD);
    assert!(!forked.to_jsonl().contains(OLD), "old session id fully gone");
}

#[test]
fn forking_before_the_first_turn_has_no_boundary() {
    // U1 is the first turn — nothing completed precedes it.
    assert!(matches!(
        fork_before(&two_turns(), U1).unwrap_err(),
        SurgeryError::NoBoundaryBefore(_)
    ));
}

#[test]
fn fork_before_unknown_turn_is_an_error() {
    assert!(matches!(
        fork_before(&two_turns(), "nope").unwrap_err(),
        SurgeryError::TurnNotFound(_)
    ));
}
