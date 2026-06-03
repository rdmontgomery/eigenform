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
