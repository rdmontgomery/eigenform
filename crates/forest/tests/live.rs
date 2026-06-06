//! live_forest: corroborate ~/.claude/sessions/<pid>.json (process liveness) with the
//! project JSONLs (state + recency) into the live-Forest snapshot. Liveness is injected
//! so the test is deterministic (no real pids).

use chrono::{DateTime, Utc};
use eigen_forest::{is_pid_alive, live_forest_with, SessionState};

#[test]
fn is_pid_alive_sees_this_running_process() {
    assert!(is_pid_alive(std::process::id()), "the test process is alive");
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-06-06T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A JSONL whose tail ends on a *completed* turn (user → assistant → turn_duration),
/// the assistant message carrying `output_tokens` so the spark is non-empty.
fn completed(uuid: &str, day: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"timestamp\":\"{day}T00:00:00Z\",\"sessionId\":\"{uuid}\",\"cwd\":\"/home/me/p\",\"message\":{{\"role\":\"user\"}}}}\n\
         {{\"type\":\"assistant\",\"timestamp\":\"{day}T00:00:01Z\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"output_tokens\":123}}}}}}\n\
         {{\"type\":\"system\",\"subtype\":\"turn_duration\",\"timestamp\":\"{day}T00:00:02Z\"}}\n"
    )
}

/// A JSONL whose tail ends on a *pending* prompt (trailing user, no closing turn).
fn pending(uuid: &str, day: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"timestamp\":\"{day}T00:00:00Z\",\"sessionId\":\"{uuid}\",\"cwd\":\"/home/me/p\",\"message\":{{\"role\":\"user\"}}}}\n"
    )
}

fn sfile(pid: u32, uuid: &str) -> String {
    format!(
        "{{\"pid\":{pid},\"sessionId\":\"{uuid}\",\"cwd\":\"/home/me/p\",\"startedAt\":1,\"kind\":\"interactive\",\"entrypoint\":\"cli\"}}"
    )
}

/// projects: aaa (completed, 06-06), bbb (pending, 06-06), ccc (completed, 06-05).
/// sessions: pid 42→aaa, pid 43→bbb. ccc has no session file.
fn fixture() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let proj = tempfile::tempdir().unwrap();
    let sess = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let pdir = proj.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(pdir.join("aaa.jsonl"), completed("aaa", "2026-06-06")).unwrap();
    std::fs::write(pdir.join("bbb.jsonl"), pending("bbb", "2026-06-06")).unwrap();
    std::fs::write(pdir.join("ccc.jsonl"), completed("ccc", "2026-06-05")).unwrap();
    std::fs::write(sess.path().join("42.json"), sfile(42, "aaa")).unwrap();
    std::fs::write(sess.path().join("43.json"), sfile(43, "bbb")).unwrap();
    (proj, sess, state)
}

#[test]
fn live_sessions_badged_ready_or_working_dead_are_recent() {
    let (proj, sess, state) = fixture();
    let alive = |pid: u32| pid == 42 || pid == 43;
    let got = live_forest_with(proj.path(), sess.path(), state.path(), now(), alive);

    let by = |u: &str| {
        got.iter()
            .find(|s| s.uuid == u)
            .unwrap_or_else(|| panic!("{u} present"))
    };
    assert_eq!(by("aaa").state, SessionState::Ready, "completed + live → ready");
    assert!(by("aaa").live);
    assert_eq!(by("aaa").spark, vec![123], "spark = output_tokens per turn");
    assert_eq!(by("bbb").state, SessionState::Working, "pending + live → working");
    assert!(by("bbb").live);
    assert_eq!(by("ccc").state, SessionState::Recent, "no live process → recent");
    assert!(!by("ccc").live);
}

#[test]
fn dead_pids_are_filtered_out() {
    let (proj, sess, state) = fixture();
    let got = live_forest_with(proj.path(), sess.path(), state.path(), now(), |_| false);
    assert!(got.iter().all(|s| !s.live), "no live when all pids dead");
    assert!(got.iter().all(|s| s.state == SessionState::Recent));
}

#[test]
fn ready_sorts_before_working_before_recent() {
    let (proj, sess, state) = fixture();
    let alive = |pid: u32| pid == 42 || pid == 43;
    let got = live_forest_with(proj.path(), sess.path(), state.path(), now(), alive);
    let order: Vec<_> = got.iter().map(|s| s.uuid.as_str()).collect();
    assert_eq!(order, vec!["aaa", "bbb", "ccc"], "ready, then working, then recent");
}
