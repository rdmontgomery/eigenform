//! fork_diff_view: side-by-side diff of a source session and a fork, aligned by turn
//! uuid (fork_at preserves uuids). Source left, fork right; summary + leaf move.

use eigen_render::{fork_diff_view, render_text};
use eigen_surgery::{edit_then_fork, fork_at, inject, Role, Session};

const SID: &str = "abcd1234-0000-4000-8000-000000000000";
const U1: &str = "u1";
const A1: &str = "a1";
const S1: &str = "s1";
const U2: &str = "u2";
const A2: &str = "a2";
const S2: &str = "s2";

fn source() -> Session {
    let text = [
        format!(r#"{{"type":"user","uuid":"{U1}","parentUuid":null,"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"first prompt"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A1}","parentUuid":"{U1}","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"text","text":"first reply"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S1}","parentUuid":"{A1}","isSidechain":false,"subtype":"turn_duration","durationMs":1000,"sessionId":"{SID}"}}"#),
        format!(r#"{{"type":"user","uuid":"{U2}","parentUuid":"{S1}","isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"second prompt"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A2}","parentUuid":"{U2}","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"text","text":"second reply"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S2}","parentUuid":"{A2}","isSidechain":false,"subtype":"turn_duration","durationMs":2000,"sessionId":"{SID}"}}"#),
        format!(r#"{{"type":"last-prompt","leafUuid":"{S2}","sessionId":"{SID}"}}"#),
    ]
    .join("\n")
        + "\n";
    Session::parse_str(&text).unwrap()
}

/// Find the row mentioning `needle` and report whether it's left or right of the `│`.
fn side_of(out: &str, needle: &str) -> (bool, bool) {
    let row = out.lines().find(|l| l.contains(needle)).unwrap_or("");
    match row.split_once('│') {
        Some((l, r)) => (l.contains(needle), r.contains(needle)),
        None => (row.contains(needle), false),
    }
}

#[test]
fn fork_drops_the_tail_kept_prefix_on_both_sides() {
    let src = source();
    let fork = fork_at(&src, A1).unwrap();
    let out = render_text(&fork_diff_view(&src, &fork));

    assert!(out.contains("kept 3"), "U1/A1/S1 kept:\n{out}");
    assert!(out.contains("dropped 3"), "U2/A2/S2 dropped:\n{out}");

    // a kept turn shows on both sides; a dropped turn only on the left
    let (kl, kr) = side_of(&out, "first prompt");
    assert!(kl && kr, "kept turn on both sides:\n{out}");
    let (dl, dr) = side_of(&out, "second prompt");
    assert!(dl && !dr, "dropped turn left-only:\n{out}");
}

#[test]
fn injected_turn_appears_on_the_right_only() {
    let src = source();
    let fork = inject(&src, A1, Role::User, "a synthetic instruction").unwrap();
    let out = render_text(&fork_diff_view(&src, &fork));

    assert!(out.contains("injected 1"), "summary:\n{out}");
    let (l, r) = side_of(&out, "a synthetic instruction");
    assert!(!l && r, "injected turn right-only:\n{out}");
}

#[test]
fn edited_turn_shows_old_left_new_right() {
    let src = source();
    let fork = edit_then_fork(&src, U2, "an edited prompt").unwrap();
    let out = render_text(&fork_diff_view(&src, &fork));

    assert!(out.contains("edited 1"), "summary:\n{out}");
    let (ol, _or) = side_of(&out, "second prompt");
    assert!(ol, "old content on the left:\n{out}");
    let (_nl, nr) = side_of(&out, "an edited prompt");
    assert!(nr, "new content on the right:\n{out}");
}

#[test]
fn header_names_both_sessions_and_reports_the_leaf_move() {
    let src = source();
    let fork = fork_at(&src, A1).unwrap();
    let out = render_text(&fork_diff_view(&src, &fork));
    assert!(out.contains("diff "), "has a diff header:\n{out}");
    assert!(out.to_lowercase().contains("leaf"), "reports leaf move:\n{out}");
}
