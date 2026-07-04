//! POST /api/session/:uuid/recover-downgrade: detect the guardrail downgrade, fork a
//! Fable branch truncated to before the offending prompt, and stage a suggested
//! restatement (rephrased, or the verbatim prompt if the rephraser errors). Never sends.

use std::io::Write;

use eigenform_daemon::{app, Config};

const DOWNGRADED_UUID: &str = "dddd4444-0000-4000-8000-000000000004";

/// A downgraded session (Task 1's `guardrail_fixture()` shape, mirrored from
/// `forest_route.rs`): u1 → a1(fable) → sys1(turn_duration boundary) → u2(offending)
/// → a2(opus). The silent model-field flip fable→opus at a2 is the guardrail (no
/// `<synthetic>` notice is written for a real downgrade). The completed-turn boundary
/// before u2 lets `fork_before` truncate to a resumable Fable branch.
fn downgrade_fixture(rephrase_cmd: Vec<String>) -> (tempfile::TempDir, std::path::PathBuf, Config) {
    let proj = tempfile::tempdir().unwrap();
    let pdir = proj.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();

    let downgraded = [
        r#"{"type":"user","isSidechain":false,"uuid":"u1","timestamp":"2026-06-06T10:00:00Z","sessionId":"s","message":{"role":"user","content":"benign question"}}"#.to_string(),
        r#"{"type":"assistant","isSidechain":false,"uuid":"a1","timestamp":"2026-06-06T10:00:01Z","message":{"model":"claude-fable-5","role":"assistant","content":[{"type":"text","text":"ok"}]}}"#.to_string(),
        r#"{"type":"system","subtype":"turn_duration","uuid":"sys1","timestamp":"2026-06-06T10:00:01.5Z"}"#.to_string(),
        r#"{"type":"user","isSidechain":false,"uuid":"u2","timestamp":"2026-06-06T10:00:02Z","sessionId":"s","message":{"role":"user","content":"the offending prompt"}}"#.to_string(),
        r#"{"type":"assistant","isSidechain":false,"uuid":"a2","timestamp":"2026-06-06T10:00:04Z","message":{"model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"reply"}]}}"#.to_string(),
    ].join("\n") + "\n";
    std::fs::write(pdir.join(format!("{DOWNGRADED_UUID}.jsonl")), downgraded).unwrap();

    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: Some(proj.path().to_path_buf()),
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
        rephrase_cmd,
        log_file: None,
    };
    (proj, pdir, cfg)
}

/// A stub rephraser that ignores its args and prints a canned line. Returns a `TempPath`
/// (not the open `NamedTempFile`) so the writable fd is closed before we exec it —
/// otherwise Linux refuses with ETXTBSY. (Mirrors `rephrase.rs`.)
fn stub_script() -> tempfile::TempPath {
    let mut f = tempfile::Builder::new().suffix(".sh").tempfile().unwrap();
    writeln!(f, "#!/bin/sh\necho 'restated: please advise on defensive hardening'").unwrap();
    let mut perms = std::fs::metadata(f.path()).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(f.path(), perms).unwrap();
    f.into_temp_path()
}

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

/// Minimal HTTP POST (empty body) over raw TCP; returns the response body.
async fn post(url: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
}

#[tokio::test]
async fn recover_downgrade_forks_and_stages_the_rephrased_prompt() {
    let stub = stub_script();
    let cmd = vec![stub.to_str().unwrap().to_string()];
    let (_proj, pdir, cfg) = downgrade_fixture(cmd);
    let base = start(cfg).await;

    let body = post(&format!("{base}/api/session/{DOWNGRADED_UUID}/recover-downgrade")).await;
    let v: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| panic!("json:\n{body}"));

    let branch_uuid = v["branchUuid"].as_str().unwrap_or_else(|| panic!("branchUuid string:\n{body}"));
    assert!(!branch_uuid.is_empty(), "branchUuid non-empty:\n{body}");

    // The fork was written into the project dir alongside the source.
    let branch_path = pdir.join(format!("{branch_uuid}.jsonl"));
    assert!(branch_path.exists(), "branch file written at {branch_path:?}");

    assert!(
        v["stagedText"].as_str().unwrap_or("").contains("restated:"),
        "stagedText carries the stub output:\n{body}"
    );
    assert_eq!(v["offendingTurn"], serde_json::json!("u2"), "offending turn:\n{body}");
    assert_eq!(v["note"], serde_json::Value::Null, "no note on success:\n{body}");

    // fork_before dropped the offending prompt: the branch must not contain it.
    let branch = std::fs::read_to_string(&branch_path).unwrap();
    assert!(
        !branch.contains("the offending prompt"),
        "branch excludes the offending prompt:\n{branch}"
    );
}

#[tokio::test]
async fn recover_downgrade_falls_back_to_verbatim_when_rephraser_errors() {
    // `false` exits non-zero → rephrase_prompt Errs → stage the verbatim offending text.
    let (_proj, _pdir, cfg) = downgrade_fixture(vec!["false".to_string()]);
    let base = start(cfg).await;

    let body = post(&format!("{base}/api/session/{DOWNGRADED_UUID}/recover-downgrade")).await;
    let v: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| panic!("json:\n{body}"));

    assert_eq!(
        v["stagedText"], serde_json::json!("the offending prompt"),
        "verbatim offending text staged:\n{body}"
    );
    assert!(v["note"].is_string(), "a note explains the fallback:\n{body}");
    assert_eq!(v["offendingTurn"], serde_json::json!("u2"), "offending turn:\n{body}");
}
