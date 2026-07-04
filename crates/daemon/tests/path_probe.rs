//! GET /api/path?path=<abs> → {"exists": bool, "isDir": bool}.
//! The launcher uses this to tell "open an existing dir" from "make a new one".

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

fn cfg() -> Config {
    Config {
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
        log_file: None,
    }
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
async fn health_route_identifies_eigenform_with_pid() {
    let base = start(cfg()).await;
    let body = helpers::http_get(&base, "/api/health").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["app"], "eigenform", "marker the launcher keys reuse on");
    assert_eq!(v["pid"], std::process::id(), "pid the `stop` command terminates");
    assert!(v["version"].is_string(), "version string present");
}

#[tokio::test]
async fn existing_directory_reports_exists_and_isdir() {
    let dir = tempfile::tempdir().unwrap();
    let base = start(cfg()).await;
    let enc = urlencoding(dir.path().to_str().unwrap());
    let body = helpers::http_get(&base, &format!("/api/path?path={enc}")).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["exists"], true);
    assert_eq!(v["isDir"], true);
}

#[tokio::test]
async fn missing_path_reports_not_exists() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope-not-here");
    let base = start(cfg()).await;
    let enc = urlencoding(missing.to_str().unwrap());
    let body = helpers::http_get(&base, &format!("/api/path?path={enc}")).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["exists"], false);
    assert_eq!(v["isDir"], false);
}

#[tokio::test]
async fn existing_file_exists_but_not_isdir() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let base = start(cfg()).await;
    let enc = urlencoding(file.to_str().unwrap());
    let body = helpers::http_get(&base, &format!("/api/path?path={enc}")).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["exists"], true);
    assert_eq!(v["isDir"], false);
}

/// Percent-encode the bytes a tempdir path can contain (`/` stays; space/`%` don't appear).
/// Tempdir paths are ASCII-safe, so a minimal encoder suffices for these tests.
fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '-' | '_' | '.' => c.to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}
