//! GET /api/candidates: the launcher list — recent cwds merged with workspace subdirs.

#[path = "helpers/mod.rs"]
mod helpers;

use eigen_daemon::{app, Config};

const UUID: &str = "cccc3333-0000-4000-8000-000000000003";

/// Two workspace subdirs (`alpha`, `beta`); one session whose cwd is the `beta` path.
/// Expected response: beta first (recent: true), alpha second (recent: false).
fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Config) {
    // Workspace root with two immediate subdirs.
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("alpha")).unwrap();
    std::fs::create_dir_all(workspace.path().join("beta")).unwrap();

    let beta_path = workspace.path().join("beta");

    // Projects dir with one session whose cwd is the beta path.
    let projects = tempfile::tempdir().unwrap();
    // Claude Code escapes the cwd to build the project dir name (/ → -).
    let escaped = beta_path.to_str().unwrap().replace('/', "-");
    let pdir = projects.path().join(&escaped);
    std::fs::create_dir_all(&pdir).unwrap();
    let beta_str = beta_path.to_str().unwrap();
    let line = format!(
        r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"cwd":"{beta_str}","timestamp":"2026-06-11T10:00:00Z","sessionId":"{UUID}","message":{{"role":"user","content":"hello"}}}}"#
    );
    std::fs::write(pdir.join(format!("{UUID}.jsonl")), line + "\n").unwrap();

    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: Some(projects.path().to_path_buf()),
        sessions_dir: None,
        state_dir: None,
        workspace_root: Some(workspace.path().to_path_buf()),
        dev: false,
    };
    (workspace, projects, cfg)
}

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn candidates_recents_first_then_subdirs_deduped() {
    let (_ws, _proj, cfg) = fixture();
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/candidates").await;

    let v: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| panic!("expected JSON array, got:\n{body}"));
    let arr = v.as_array().expect("JSON array");

    assert_eq!(arr.len(), 2, "beta (recent) + alpha (subdir), no duplicates:\n{body}");

    // beta must be first and tagged recent: true
    let beta = &arr[0];
    assert!(
        beta["path"].as_str().unwrap().ends_with("beta"),
        "first entry should be the beta cwd (recent): {beta}"
    );
    assert_eq!(beta["recent"], true, "beta is a recent cwd: {beta}");

    // alpha must be second and tagged recent: false
    let alpha = &arr[1];
    assert!(
        alpha["path"].as_str().unwrap().ends_with("alpha"),
        "second entry should be alpha (workspace subdir): {alpha}"
    );
    assert_eq!(alpha["recent"], false, "alpha is only a subdir: {alpha}");
}

#[tokio::test]
async fn candidates_empty_when_nothing_configured() {
    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
    };
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/candidates").await;
    let v: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| panic!("expected JSON, got:\n{body}"));
    assert_eq!(v.as_array().unwrap().len(), 0, "no config → empty array:\n{body}");
}

#[tokio::test]
async fn candidates_missing_workspace_root_returns_only_recents() {
    // workspace_root is None but projects_dir has a session → only that cwd, tagged recent.
    let (_ws, _proj, mut cfg) = fixture();
    cfg.workspace_root = None;
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/candidates").await;
    let v: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| panic!("expected JSON, got:\n{body}"));
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1, "one recent cwd, no subdirs:\n{body}");
    assert_eq!(arr[0]["recent"], true);
}
