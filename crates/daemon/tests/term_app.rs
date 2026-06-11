//! Task 0.1: verify that /term serves the webterm app and / still serves woland.

use eigen_daemon::{app, Config};

/// Spin up the daemon with both web_dir and term_dir, return the base URL.
async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

/// Minimal HTTP GET (no external client crate): raw request over TCP.
async fn get(base: &str, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let host = base.strip_prefix("http://").unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    text.split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn serves_term_app_at_term_prefix_and_woland_at_root() {
    let woland = tempfile::tempdir().unwrap();
    std::fs::write(woland.path().join("index.html"), "WOLAND").unwrap();
    let term = tempfile::tempdir().unwrap();
    std::fs::write(term.path().join("index.html"), "TERM").unwrap();

    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: Some(woland.path().to_path_buf()),
        term_dir: Some(term.path().to_path_buf()),
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        dev: false,
    };
    let base = start(cfg).await;
    let body = get(&base, "/term/").await;
    assert!(body.contains("TERM"), "/term/ must serve the webterm app, got: {body:?}");
    let body = get(&base, "/").await;
    assert!(body.contains("WOLAND"), "/ must still serve woland, got: {body:?}");
}
