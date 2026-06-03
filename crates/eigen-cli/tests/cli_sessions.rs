//! `eigen sessions show <session> [--render text]` renders a session turn-tree.

use std::process::Command;
use tempfile::tempdir;

const SID: &str = "abcd1234-0000-4000-8000-000000000000";

fn session_text() -> String {
    [
        format!(r#"{{"type":"user","uuid":"U1","parentUuid":null,"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"hello there"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"A1","parentUuid":"U1","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"text","text":"hi back"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"S1","parentUuid":"A1","isSidechain":false,"subtype":"turn_duration","durationMs":4200,"sessionId":"{SID}"}}"#),
        format!(r#"{{"type":"last-prompt","leafUuid":"S1","sessionId":"{SID}"}}"#),
    ]
    .join("\n")
        + "\n"
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_eigen"))
        .args(args)
        .output()
        .expect("run eigen")
}

#[test]
fn sessions_show_renders_the_turn_tree() {
    let dir = tempdir().unwrap();
    let src = dir.path().join(format!("{SID}.jsonl"));
    std::fs::write(&src, session_text()).unwrap();

    let out = run(&["sessions", "show", src.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("session abcd1234 · 1 exchange"), "got:\n{stdout}");
    assert!(stdout.contains("● user"), "got:\n{stdout}");
    assert!(stdout.contains("◇ assistant"), "got:\n{stdout}");
    assert!(stdout.contains("← leaf"), "got:\n{stdout}");
}

/// Build a temp HOME containing one project with one session, return (home, uuid).
fn temp_home_with_session() -> (tempfile::TempDir, String) {
    let uuid = "aaaa1111-0000-4000-8000-000000000001";
    let home = tempdir().unwrap();
    let pdir = home.path().join(".claude/projects/-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    let lines = [
        format!(r#"{{"type":"user","uuid":"{uuid}","parentUuid":null,"isSidechain":false,"cwd":"/home/me/p","timestamp":"2026-06-03T10:00:00Z","sessionId":"{uuid}","message":{{"role":"user","content":"hello"}}}}"#),
        format!(r#"{{"type":"ai-title","aiTitle":"my recent work","sessionId":"{uuid}"}}"#),
    ];
    std::fs::write(pdir.join(format!("{uuid}.jsonl")), lines.join("\n") + "\n").unwrap();
    (home, uuid.to_string())
}

fn run_home(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_eigen"))
        .env("HOME", home)
        .args(args)
        .output()
        .expect("run eigen")
}

#[test]
fn show_resolves_a_session_by_uuid_prefix() {
    let (home, _uuid) = temp_home_with_session();
    let out = run_home(home.path(), &["sessions", "show", "aaaa1111"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("session aaaa1111"), "got:\n{stdout}");
    assert!(stdout.contains("● user"), "got:\n{stdout}");
}

#[test]
fn list_shows_recent_sessions_with_titles() {
    let (home, _uuid) = temp_home_with_session();
    let out = run_home(home.path(), &["sessions", "list", "--all-projects"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("aaaa1111"), "got:\n{stdout}");
    assert!(stdout.contains("my recent work"), "got:\n{stdout}");
}

/// A forkable session (user → assistant → system) under a temp HOME.
fn temp_home_with_forkable() -> (tempfile::TempDir, String) {
    let uuid = "bbbb2222-0000-4000-8000-000000000002";
    let home = tempdir().unwrap();
    let pdir = home.path().join(".claude/projects/-home-me-q");
    std::fs::create_dir_all(&pdir).unwrap();
    let lines = [
        format!(r#"{{"type":"user","uuid":"U1","parentUuid":null,"isSidechain":false,"cwd":"/home/me/q","sessionId":"{uuid}","message":{{"role":"user","content":"keep me"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"A1","parentUuid":"U1","isSidechain":false,"sessionId":"{uuid}","message":{{"role":"assistant","content":[{{"type":"text","text":"reply"}}]}}}}"#),
        format!(r#"{{"type":"user","uuid":"U2","parentUuid":"A1","isSidechain":false,"sessionId":"{uuid}","message":{{"role":"user","content":"drop me"}}}}"#),
        format!(r#"{{"type":"last-prompt","leafUuid":"U2","sessionId":"{uuid}"}}"#),
    ];
    std::fs::write(pdir.join(format!("{uuid}.jsonl")), lines.join("\n") + "\n").unwrap();
    (home, uuid.to_string())
}

#[test]
fn diff_reports_kept_and_dropped() {
    let (home, uuid) = temp_home_with_forkable();
    let fork = run_home(home.path(), &["surgery", "fork", &uuid, "--at", "U1"]);
    let fork_uuid = String::from_utf8(fork.stdout).unwrap().trim().to_string();

    let out = run_home(home.path(), &["sessions", "diff", &uuid, &fork_uuid]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("diff "), "header:\n{stdout}");
    assert!(stdout.contains("kept"), "summary:\n{stdout}");
    assert!(stdout.contains("keep me"), "kept turn shown:\n{stdout}");
    assert!(stdout.contains("drop me"), "dropped turn shown:\n{stdout}");
}

#[test]
fn surgery_fork_diff_flag_prints_diff_to_stderr() {
    let (home, uuid) = temp_home_with_forkable();
    let out = run_home(home.path(), &["surgery", "fork", &uuid, "--at", "U1", "--diff"]);
    assert!(out.status.success());
    // stdout stays just the new uuid (scriptable); the diff goes to stderr.
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "stdout is only the uuid:\n{stdout}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("diff "), "diff on stderr:\n{stderr}");
    assert!(stderr.contains("drop me"), "shows dropped turn:\n{stderr}");
}

#[test]
fn render_json_is_not_yet_supported() {
    let dir = tempdir().unwrap();
    let src = dir.path().join(format!("{SID}.jsonl"));
    std::fs::write(&src, session_text()).unwrap();

    let out = run(&["sessions", "show", src.to_str().unwrap(), "--render", "json"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("browser"), "expected a 'not until browser' message, got: {stderr}");
}
