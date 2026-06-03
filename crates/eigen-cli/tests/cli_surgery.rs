//! End-to-end CLI: `eigen surgery <op>` parses a session file, performs surgery, writes
//! the new session beside the source, and prints the new uuid.

use std::process::Command;
use tempfile::tempdir;

const OLD: &str = "11111111-1111-4111-8111-111111111111";
const U1: &str = "turn-u1";
const A1: &str = "turn-a1";
const S1: &str = "turn-s1";

fn session_text() -> String {
    [
        format!(r#"{{"type":"user","uuid":"{U1}","parentUuid":null,"isSidechain":false,"sessionId":"{OLD}","message":{{"role":"user","content":"one"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"{A1}","parentUuid":"{U1}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"assistant","content":[{{"type":"text","text":"a-one"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"{S1}","parentUuid":"{A1}","isSidechain":false,"subtype":"turn_duration","sessionId":"{OLD}"}}"#),
        format!(r#"{{"type":"user","uuid":"turn-u2","parentUuid":"{S1}","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"user","content":"two"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"turn-a2","parentUuid":"turn-u2","isSidechain":false,"sessionId":"{OLD}","message":{{"role":"assistant","content":[{{"type":"text","text":"a-two"}}]}}}}"#),
        format!(r#"{{"type":"last-prompt","lastPrompt":"two","leafUuid":"turn-a2","sessionId":"{OLD}"}}"#),
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
fn fork_writes_a_new_session_beside_the_source_and_prints_its_uuid() {
    let dir = tempdir().unwrap();
    let src = dir.path().join(format!("{OLD}.jsonl"));
    std::fs::write(&src, session_text()).unwrap();

    let out = run(&["surgery", "fork", src.to_str().unwrap(), "--at", A1]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let new_uuid = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(!new_uuid.is_empty());
    assert_ne!(new_uuid, OLD);

    let new_path = dir.path().join(format!("{new_uuid}.jsonl"));
    assert!(new_path.exists(), "expected forked session at {new_path:?}");

    let contents = std::fs::read_to_string(&new_path).unwrap();
    assert!(contents.contains(&new_uuid));
    assert!(!contents.contains(OLD), "old id fully rewritten");
    assert!(!contents.contains("turn-a2"), "tail dropped");
}

#[test]
fn inject_reads_content_from_a_file() {
    let dir = tempdir().unwrap();
    let src = dir.path().join(format!("{OLD}.jsonl"));
    std::fs::write(&src, session_text()).unwrap();
    let content = dir.path().join("inject.txt");
    std::fs::write(&content, "a synthetic instruction").unwrap();

    let out = run(&[
        "surgery", "inject", src.to_str().unwrap(),
        "--at", A1, "--as", "user", "--content", content.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let new_uuid = String::from_utf8(out.stdout).unwrap().trim().to_string();
    let new_path = dir.path().join(format!("{new_uuid}.jsonl"));
    let contents = std::fs::read_to_string(&new_path).unwrap();
    assert!(contents.contains("a synthetic instruction"));
}

#[test]
fn unknown_turn_exits_nonzero() {
    let dir = tempdir().unwrap();
    let src = dir.path().join(format!("{OLD}.jsonl"));
    std::fs::write(&src, session_text()).unwrap();

    let out = run(&["surgery", "rewind", src.to_str().unwrap(), "--to", "does-not-exist"]);
    assert!(!out.status.success());
}
