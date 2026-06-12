//! Task 0.1: verify that /term serves the webterm app and / still serves woland.
//! Bug fix: bare /term (no trailing slash) must serve the index with absolute /term/ asset URLs.

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

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
        workspace_root: None,
        dev: false,
    };
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/term/").await;
    assert!(body.contains("TERM"), "/term/ must serve the webterm app, got: {body:?}");
    let body = helpers::http_get(&base, "/").await;
    assert!(body.contains("WOLAND"), "/ must still serve woland, got: {body:?}");
}

/// Bare /term (no trailing slash) must serve the index AND every asset URL in that
/// HTML must be absolute under /term/ — not relative — so that the browser's base
/// directory (which is `/` for bare /term without a redirect) resolves them correctly.
///
/// The fixture content is read from the real webterm/index.html so this test pins the
/// actual file rather than a synthetic stub.
#[tokio::test]
async fn bare_term_serves_index_with_absolute_asset_urls() {
    // Read the real webterm/index.html from the repo root (two levels up from this
    // crate's Cargo.toml, which lives at crates/daemon/).
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let real_index = std::fs::read_to_string(repo_root.join("webterm/index.html"))
        .expect("webterm/index.html must exist");

    let woland = tempfile::tempdir().unwrap();
    std::fs::write(woland.path().join("index.html"), "WOLAND").unwrap();
    let term = tempfile::tempdir().unwrap();
    std::fs::write(term.path().join("index.html"), &real_index).unwrap();

    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: Some(woland.path().to_path_buf()),
        term_dir: Some(term.path().to_path_buf()),
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
    };
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/term").await;

    // Must serve the webterm index (not a 404 or woland's page).
    assert!(
        body.contains("<div id=\"app\">"),
        "bare /term must serve the webterm index, got: {body:?}"
    );

    // Asset URLs must be absolute /term/-prefixed, not relative.
    assert!(
        body.contains("src=\"/term/dist/main.js\""),
        "script src must be absolute /term/dist/main.js, got: {body:?}"
    );
    assert!(
        body.contains("href=\"/term/dist/main.css\""),
        "stylesheet href must be absolute /term/dist/main.css, got: {body:?}"
    );

    // Must NOT contain bare relative asset references.
    assert!(
        !body.contains("src=\"dist/"),
        "relative src=\"dist/... must not appear, got: {body:?}"
    );
    assert!(
        !body.contains("href=\"dist/"),
        "relative href=\"dist/... must not appear, got: {body:?}"
    );
}
