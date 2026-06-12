//! Dev mode: `GET /` injects a live-reload hook; prod serves the static index untouched.

use eigenform_daemon::{app, Config};

fn cfg(web_dir: std::path::PathBuf, dev: bool) -> Config {
    Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: Some(web_dir),
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        dev,
    }
}

fn web_with_index() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><html><head><title>x</title></head><body></body></html>",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("dist")).unwrap();
    dir
}

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("127.0.0.1:{}", addr.port())
}

async fn get(host: &str, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn dev_mode_injects_the_reload_meta() {
    let web = web_with_index();
    let host = start(cfg(web.path().to_path_buf(), true)).await;
    let body = get(&host, "/").await;
    assert!(body.contains(r#"<meta name="eigenform-dev""#), "dev meta injected:\n{body}");
}

#[tokio::test]
async fn prod_mode_serves_static_index_without_meta() {
    let web = web_with_index();
    let host = start(cfg(web.path().to_path_buf(), false)).await;
    let body = get(&host, "/").await;
    assert!(!body.contains("eigenform-dev"), "no dev meta in prod:\n{body}");
}
