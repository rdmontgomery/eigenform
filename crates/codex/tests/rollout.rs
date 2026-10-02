//! A synthetic rollout shaped like Codex's on-disk format (see
//! `notes/spikes/16-codex-rollout-schema.md`) read through the public API.

use std::fs;
use std::path::{Path, PathBuf};

const ID: &str = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

fn rollout_lines() -> Vec<String> {
    vec![
        format!(r#"{{"timestamp":"2026-10-02T09:14:03.120Z","type":"session_meta","payload":{{"id":"{ID}","session_id":"{ID}","timestamp":"2026-10-02T09:14:03.100Z","cwd":"/w/repo","originator":"codex_exec","cli_version":"0.130.0","source":"exec","model_provider":"openai"}}}}"#),
        r#"{"timestamp":"2026-10-02T09:14:03.130Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/w/repo</cwd>\n</environment_context>"}]}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:03.131Z","type":"turn_context","payload":{"cwd":"/w/repo","model":"gpt-5.5-codex","approval_policy":"never"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:03.132Z","type":"event_msg","payload":{"type":"task_started"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:03.133Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the flaky retry test in crates/net"}]}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:03.134Z","type":"event_msg","payload":{"type":"user_message","message":"Fix the flaky retry test in crates/net"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:05.000Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"cargo test -p net retry\"],\"workdir\":\"/w/repo\"}","call_id":"call_1"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:09.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"test retry ... FAILED"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:12.000Z","type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","call_id":"call_2","input":"*** Begin Patch\n*** Update File: crates/net/src/retry.rs\n@@\n-sleep(10)\n+sleep(backoff)\n*** End Patch"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:12.500Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call_2","output":"Success. Updated the following files:\nM crates/net/src/retry.rs"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:20.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Replaced the fixed sleep with the backoff schedule; the test passes 50/50."}]}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:14:20.100Z","type":"event_msg","payload":{"type":"task_complete","last_agent_message":"done"}}"#.to_string(),
    ]
}

fn write_home(lines: &[String]) -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join("sessions/2026/10/02");
    fs::create_dir_all(&day).unwrap();
    let path = day.join(format!("rollout-2026-10-02T09-14-03-{ID}.jsonl"));
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    // A compressed cold rollout is skipped, not misread.
    fs::write(
        day.join("rollout-2026-10-01T00-00-00-ffff.jsonl.zst"),
        b"\x28\xb5\x2f\xfd",
    )
    .unwrap();
    (home, path)
}

#[test]
fn enumerates_and_resolves_by_prefix() {
    let (home, path) = write_home(&rollout_lines());
    let stubs = eigenform_codex::enumerate(home.path());
    assert_eq!(stubs.len(), 1);
    assert_eq!(stubs[0].id, ID);
    let hit = eigenform_codex::resolve(home.path(), "0199a1").unwrap();
    assert_eq!(hit.path, path);
    assert_eq!(
        eigenform_codex::resolve(home.path(), "dead"),
        Err(eigenform_codex::ResolveError::NotFound)
    );
}

#[test]
fn thread_row_reads_meta_title_model_and_recency() {
    let (home, _) = write_home(&rollout_lines());
    let rows = eigenform_codex::threads(home.path());
    assert_eq!(rows.len(), 1);
    let t = &rows[0];
    assert_eq!(t.cwd, Path::new("/w/repo"));
    assert_eq!(t.source, "exec");
    assert!(t.headless, "codex exec is headless");
    assert_eq!(
        t.title.as_deref(),
        Some("Fix the flaky retry test in crates/net")
    );
    assert_eq!(t.model.as_deref(), Some("gpt-5.5-codex"));
    assert_eq!(t.turns, 1);
    assert_eq!(t.recency.to_rfc3339(), "2026-10-02T09:14:20.100+00:00");
    assert!(!t.live(), "no writer lock held");
    assert_eq!(t.state.as_str(), "recent");
}

#[test]
fn session_json_maps_codex_tools_onto_the_drawer_shape() {
    let contents = rollout_lines().join("\n");
    let v: serde_json::Value =
        serde_json::from_str(&eigenform_codex::session_json(ID, &contents)).unwrap();
    assert_eq!(v["engine"], "codex");
    assert_eq!(v["model"], "gpt-5.5-codex");
    let ex = v["exchanges"].as_array().unwrap();
    // one user turn (injected context dropped, event_msg preferred over the duplicate
    // response_item), a second exchange for the second tool, then the leaf.
    assert_eq!(ex[0]["user"], "Fix the flaky retry test in crates/net");
    assert_eq!(ex[0]["tool"]["kind"], "Bash");
    assert_eq!(ex[0]["tool"]["input"]["command"], "cargo test -p net retry");
    assert_eq!(ex[0]["tool"]["output"], "test retry ... FAILED");
    assert_eq!(ex[1]["tool"]["kind"], "Edit");
    assert_eq!(
        ex[1]["tool"]["input"]["file_path"],
        "/w/repo/crates/net/src/retry.rs"
    );
    assert!(ex[1]["assistant"]
        .as_str()
        .unwrap()
        .contains("backoff schedule"));
    assert_eq!(ex.last().unwrap()["leaf"], true);
    assert_eq!(v["total"], ex.len());
}

/// Liveness is read from the kernel's lock table, never by taking the lock. Hold the
/// writer lock the way Codex does (an exclusive flock) and the row reads live.
#[cfg(target_os = "linux")]
#[test]
fn a_held_writer_lock_reads_as_live() {
    use std::process::{Command, Stdio};
    if Command::new("flock").arg("--version").output().is_err() {
        eprintln!("skipping: no flock(1)");
        return;
    }
    let (home, _) = write_home(&rollout_lines());
    let locks = home.path().join("thread-writer-locks");
    fs::create_dir_all(&locks).unwrap();
    let lock = locks.join(format!("{ID}.lock"));
    fs::write(&lock, b"").unwrap();

    let mut holder = Command::new("flock")
        .args(["-x", lock.to_str().unwrap(), "sleep", "10"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    // Wait for the lock to show up in /proc/locks.
    let mut pid = None;
    for _ in 0..50 {
        pid = eigenform_codex::writer_pid(home.path(), ID);
        if pid.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let rows = eigenform_codex::threads(home.path());
    holder.kill().ok();
    holder.wait().ok();

    assert!(pid.is_some(), "held flock should be visible in /proc/locks");
    assert!(rows[0].live());
    // last turn closed → an idle live writer is `ready`
    assert_eq!(rows[0].state.as_str(), "ready");
}
