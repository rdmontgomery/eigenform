//! Routing: the eigenform terminal app is the front door at `/`. The webterm index uses
//! root-relative asset URLs (/dist/...) now that it serves from the root.

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

/// Spin up the daemon serving `term_dir` (webterm); return base URL.
async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

fn cfg(term: std::path::PathBuf) -> Config {
    Config {
        program: "cat".into(),
        term_dir: Some(term),
        ..Default::default()
    }
}

/// The real webterm/index.html serves at `/` with root-relative asset URLs (/dist/...),
/// not /term/-prefixed and not bare-relative. Pins the actual file.
#[tokio::test]
async fn root_serves_webterm_index_with_root_relative_asset_urls() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let real_index = std::fs::read_to_string(repo_root.join("webterm/index.html"))
        .expect("webterm/index.html must exist");

    let term = tempfile::tempdir().unwrap();
    std::fs::write(term.path().join("index.html"), &real_index).unwrap();

    let base = start(cfg(term.path().to_path_buf())).await;
    let body = helpers::http_get(&base, "/").await;

    assert!(
        body.contains("<div id=\"app\">"),
        "/ must serve the webterm index, got: {body:?}"
    );
    assert!(
        body.contains("src=\"/dist/main.js\""),
        "script src must be root-relative /dist/main.js, got: {body:?}"
    );
    assert!(
        body.contains("href=\"/dist/main.css\""),
        "stylesheet href must be root-relative /dist/main.css, got: {body:?}"
    );
    assert!(
        !body.contains("/term/dist/"),
        "stale /term/dist/ URLs must be gone, got: {body:?}"
    );
}
