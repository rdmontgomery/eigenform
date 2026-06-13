//! With `--features embed-assets` and no on-disk `term_dir`, the daemon serves the
//! webterm build baked into the binary: `/` → the app index, `/dist/main.js` → the
//! bundle with a JS content-type. Gated on the feature so default builds skip it.
#![cfg(feature = "embed-assets")]

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

fn embedded_cfg() -> Config {
    Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None, // force the embedded fallback
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
    }
}

#[tokio::test]
async fn embedded_root_serves_the_webterm_index() {
    let base = start(embedded_cfg()).await;
    let body = helpers::http_get(&base, "/").await;
    assert!(
        body.contains("<div id=\"app\">"),
        "embedded / must serve the webterm index, got: {body:?}"
    );
    assert!(
        body.contains("/dist/main.js"),
        "embedded index must reference the bundle, got: {body:?}"
    );
}

#[tokio::test]
async fn embedded_serves_the_bundle_asset() {
    let base = start(embedded_cfg()).await;
    let body = helpers::http_get(&base, "/dist/main.js").await;
    // The esbuild bundle is non-trivial; a stub index would be far smaller.
    assert!(body.len() > 1000, "embedded /dist/main.js looks empty: {} bytes", body.len());
}

#[tokio::test]
async fn embedded_unknown_path_falls_back_to_index() {
    let base = start(embedded_cfg()).await;
    let body = helpers::http_get(&base, "/some/spa/route").await;
    assert!(
        body.contains("<div id=\"app\">"),
        "unknown path must SPA-fallback to the index, got: {body:?}"
    );
}
