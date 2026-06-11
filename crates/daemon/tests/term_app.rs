//! Task 0.1: verify that /term serves the webterm app and / still serves woland.

#[path = "helpers/mod.rs"]
mod helpers;

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
    let body = helpers::http_get(&base, "/term/").await;
    assert!(body.contains("TERM"), "/term/ must serve the webterm app, got: {body:?}");
    let body = helpers::http_get(&base, "/").await;
    assert!(body.contains("WOLAND"), "/ must still serve woland, got: {body:?}");
}
