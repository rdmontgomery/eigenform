//! `eigenform candidates [--workspace <dir>]` — CLI mirror of GET /api/candidates.
//!
//! Computed locally: recent session cwds merged with immediate workspace subdirs.
//! Mirrors the daemon's candidates_route (crates/daemon/src/lib.rs) exactly.

use std::process::Command;
use tempfile::tempdir;

const UUID_A: &str = "aaaa0001-0000-4000-8000-000000000001";
const UUID_B: &str = "bbbb0002-0000-4000-8000-000000000002";

/// Build a temp HOME with one recent session at `cwd_path` and a workspace root
/// containing two immediate subdirs (`alpha`, `beta`). The session cwd is `beta`.
///
/// Returns (home_dir, workspace_dir).
fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
    let workspace = tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("alpha")).unwrap();
    std::fs::create_dir_all(workspace.path().join("beta")).unwrap();
    let beta_path = workspace.path().join("beta");

    let home = tempdir().unwrap();
    // Claude Code escapes the cwd (/ → -) for the project dir name.
    let escaped = beta_path.to_str().unwrap().replace('/', "-");
    let pdir = home.path().join(".claude/projects").join(&escaped);
    std::fs::create_dir_all(&pdir).unwrap();
    let beta_str = beta_path.to_str().unwrap();
    // Two sessions in the same project (same cwd) to verify dedup.
    for (uuid, ts) in &[
        (UUID_A, "2026-06-11T10:00:00Z"),
        (UUID_B, "2026-06-11T09:00:00Z"),
    ] {
        let line = format!(
            r#"{{"type":"user","uuid":"{uuid}","parentUuid":null,"isSidechain":false,"cwd":"{beta_str}","timestamp":"{ts}","sessionId":"{uuid}","message":{{"role":"user","content":"hello"}}}}"#
        );
        std::fs::write(pdir.join(format!("{uuid}.jsonl")), line + "\n").unwrap();
    }
    (home, workspace)
}

fn run(home: &std::path::Path, extra_args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_eigenform"))
        .env("HOME", home)
        .args(["candidates"])
        .args(extra_args)
        .output()
        .expect("run eigenform")
}

#[test]
fn candidates_recents_first_deduped_then_subdirs() {
    let (home, workspace) = fixture();
    let out = run(
        home.path(),
        &["--workspace", workspace.path().to_str().unwrap()],
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();

    // beta appears once (recent), alpha once (subdir) — 2 rows total (no dup).
    assert_eq!(lines.len(), 2, "expected 2 rows (beta+alpha), got:\n{stdout}");

    // First row: beta, tagged [recent]
    assert!(lines[0].contains("beta"), "first row must be beta (recent):\n{stdout}");
    assert!(lines[0].contains("[recent]"), "first row must carry [recent] tag:\n{stdout}");

    // Second row: alpha, no [recent] tag
    assert!(lines[1].contains("alpha"), "second row must be alpha (subdir):\n{stdout}");
    assert!(!lines[1].contains("[recent]"), "alpha must not carry [recent] tag:\n{stdout}");
}

#[test]
fn candidates_no_workspace_shows_only_recents() {
    let (home, _workspace) = fixture();
    // No --workspace flag; HOME has no ~/projects so workspace_root resolves to None.
    let out = run(home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();

    // Only the one recent cwd (deduped from two sessions).
    assert_eq!(lines.len(), 1, "expected 1 row (beta only), got:\n{stdout}");
    assert!(lines[0].contains("beta"), "should be the beta cwd:\n{stdout}");
    assert!(lines[0].contains("[recent]"), "must be tagged [recent]:\n{stdout}");
}

#[test]
fn candidates_empty_when_no_projects_and_no_workspace() {
    let home = tempdir().unwrap();
    // Projects dir doesn't exist; no --workspace.
    let out = run(home.path(), &[]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.trim().is_empty(), "expected empty output, got:\n{stdout}");
}

#[test]
fn candidates_workspace_subdirs_without_any_recents() {
    let workspace = tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("gamma")).unwrap();
    // HOME has no .claude/projects at all.
    let home = tempdir().unwrap();

    let out = run(
        home.path(),
        &["--workspace", workspace.path().to_str().unwrap()],
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();

    assert_eq!(lines.len(), 1, "one subdir row:\n{stdout}");
    assert!(lines[0].contains("gamma"), "must show gamma:\n{stdout}");
    assert!(!lines[0].contains("[recent]"), "gamma is not recent:\n{stdout}");
}
