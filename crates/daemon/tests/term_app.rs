//! Routing: the eigenform terminal app is the front door at `/`; the legacy woland
//! workbench is mounted at `/woland`. The webterm index uses root-relative asset URLs
//! (/dist/...) now that it serves from the root.

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

/// Spin up the daemon with both web_dir (woland) and term_dir (webterm); return base URL.
async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

fn cfg(web: std::path::PathBuf, term: std::path::PathBuf) -> Config {
    Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: Some(web),
        term_dir: Some(term),
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
        log_file: None,
    }
}

#[tokio::test]
async fn serves_eigenform_at_root_and_woland_at_woland() {
    let woland = tempfile::tempdir().unwrap();
    std::fs::write(woland.path().join("index.html"), "WOLAND").unwrap();
    let term = tempfile::tempdir().unwrap();
    std::fs::write(term.path().join("index.html"), "EIGENFORM").unwrap();

    let base = start(cfg(woland.path().to_path_buf(), term.path().to_path_buf())).await;

    let body = helpers::http_get(&base, "/").await;
    assert!(body.contains("EIGENFORM"), "/ must serve the eigenform app, got: {body:?}");

    let body = helpers::http_get(&base, "/woland/").await;
    assert!(body.contains("WOLAND"), "/woland/ must serve woland, got: {body:?}");
}

/// Bare /woland (no trailing slash) must still serve woland's index.
#[tokio::test]
async fn bare_woland_serves_its_index() {
    let woland = tempfile::tempdir().unwrap();
    std::fs::write(woland.path().join("index.html"), "WOLAND").unwrap();
    let term = tempfile::tempdir().unwrap();
    std::fs::write(term.path().join("index.html"), "EIGENFORM").unwrap();

    let base = start(cfg(woland.path().to_path_buf(), term.path().to_path_buf())).await;
    let body = helpers::http_get(&base, "/woland").await;
    assert!(body.contains("WOLAND"), "bare /woland must serve woland, got: {body:?}");
}

/// The real webterm/index.html serves at `/` with root-relative asset URLs (/dist/...),
/// not /term/-prefixed and not bare-relative. Pins the actual file.
#[tokio::test]
async fn root_serves_webterm_index_with_root_relative_asset_urls() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let real_index = std::fs::read_to_string(repo_root.join("webterm/index.html"))
        .expect("webterm/index.html must exist");

    let woland = tempfile::tempdir().unwrap();
    std::fs::write(woland.path().join("index.html"), "WOLAND").unwrap();
    let term = tempfile::tempdir().unwrap();
    std::fs::write(term.path().join("index.html"), &real_index).unwrap();

    let base = start(cfg(woland.path().to_path_buf(), term.path().to_path_buf())).await;
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
