//! The session transcript route: GET /session/:uuid renders the semantic HTML, and
//! GET /api/recent reports the most recent session uuid.

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

const UUID: &str = "aaaa1111-0000-4000-8000-000000000001";

/// A temp projects dir with one session, and a Config pointing the daemon at it.
fn fixture() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let pdir = dir.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    let lines = [
        format!(r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"cwd":"/home/me/p","timestamp":"2026-06-03T10:00:00Z","sessionId":"{UUID}","message":{{"role":"user","content":"render me in the right pane"}}}}"#),
        format!(r#"{{"type":"last-prompt","lastPrompt":"render me in the right pane","leafUuid":"u1","sessionId":"{UUID}"}}"#),
    ];
    std::fs::write(pdir.join(format!("{UUID}.jsonl")), lines.join("\n") + "\n").unwrap();
    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: Some(dir.path().to_path_buf()),
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
        rephrase_cmd: vec!["claude".to_string(), "-p".to_string()],
        log_file: None,
    };
    (dir, cfg)
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
async fn session_route_renders_the_transcript_html() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/session/aaaa1111").await;
    assert!(body.contains("<details"), "collapsible transcript:\n{body}");
    assert!(body.contains("render me in the right pane"), "content present:\n{body}");
}

#[tokio::test]
async fn session_fragment_route_returns_bare_html_for_in_page_injection() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/session/aaaa1111").await;
    assert!(body.contains("<details"), "transcript fragment:\n{body}");
    assert!(body.contains("render me in the right pane"));
    assert!(!body.to_lowercase().contains("<!doctype"), "fragment, not a full page:\n{body}");
}

#[tokio::test]
async fn sessions_route_lists_sessions_with_titles() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/sessions").await;
    assert!(body.contains(UUID), "uuid in list:\n{body}");
    assert!(body.contains("render me in the right pane"), "title in list:\n{body}");
}

#[tokio::test]
async fn projects_route_lists_distinct_cwds() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/projects").await;
    assert!(body.contains("/home/me/p"), "project cwd listed:\n{body}");
}

#[tokio::test]
async fn recent_route_reports_the_latest_uuid() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/recent").await;
    assert!(body.contains(UUID), "recent uuid:\n{body}");
}

#[tokio::test]
async fn watch_emits_change_when_the_session_file_is_written() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (dir, cfg) = fixture();
    let session_path = dir.path().join("-home-me-p").join(format!("{UUID}.jsonl"));
    let base = start(cfg).await;
    let host = base.strip_prefix("http://").unwrap().to_string();

    // Open the SSE stream.
    let mut stream = tokio::net::TcpStream::connect(&host).await.unwrap();
    let req = format!("GET /api/watch/{UUID} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();

    // Let the watcher register, then append to the session a couple of times.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    for _ in 0..3 {
        let mut f = tokio::fs::OpenOptions::new().append(true).open(&session_path).await.unwrap();
        f.write_all(b"{\"type\":\"system\",\"subtype\":\"x\"}\n").await.unwrap();
        f.flush().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let mut acc = Vec::new();
    let mut buf = [0u8; 1024];
    let mut saw_change = false;
    for _ in 0..40 {
        match tokio::time::timeout(std::time::Duration::from_millis(200), stream.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                acc.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&acc).contains("change") {
                    saw_change = true;
                    break;
                }
            }
            _ => {}
        }
    }
    assert!(saw_change, "expected an SSE 'change' event, got: {:?}", String::from_utf8_lossy(&acc));
}

