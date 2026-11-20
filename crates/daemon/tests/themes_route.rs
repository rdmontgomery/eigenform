//! GET/PUT/DELETE /api/themes: the user's custom color themes, persisted under
//! `state_dir` so they survive a daemon restart.

use eigenform_daemon::{app, Config};

mod helpers;
use helpers::{http_delete, http_get, http_put};

const JSON: (&str, &str) = ("Content-Type", "application/json");

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

fn cfg(state: &std::path::Path) -> Config {
    Config {
        program: "cat".into(),
        state_dir: Some(state.to_path_buf()),
        ..Default::default()
    }
}

fn body(id: &str, name: &str) -> String {
    let keys = [
        "background",
        "foreground",
        "cursor",
        "selectionBackground",
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "brightBlack",
        "brightRed",
        "brightGreen",
        "brightYellow",
        "brightBlue",
        "brightMagenta",
        "brightCyan",
        "brightWhite",
    ];
    let theme: serde_json::Map<String, serde_json::Value> = keys
        .iter()
        .map(|k| (k.to_string(), "#112233".into()))
        .collect();
    serde_json::json!({
        "id": id, "name": name, "base": "dracula", "theme": theme,
        "overrides": { "accent": "#FF8800" },
    })
    .to_string()
}

#[tokio::test]
async fn themes_round_trip_and_survive_restart() {
    let state = tempfile::tempdir().unwrap();
    let base = start(cfg(state.path())).await;

    let (status, out) = http_put(
        &base,
        "/api/themes/custom-a",
        &[JSON],
        &body("custom-a", "A"),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    let (status, _) = http_put(
        &base,
        "/api/themes/custom-a",
        &[JSON],
        &body("custom-a", "A2"),
    )
    .await;
    assert_eq!(status, 200, "a second save replaces");

    let base2 = start(cfg(state.path())).await;
    let list: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base2, "/api/themes").await).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "A2");
    assert_eq!(list[0]["overrides"]["accent"], "#ff8800");
    assert_eq!(list[0]["base"], "dracula");

    assert_eq!(http_delete(&base2, "/api/themes/custom-a").await, 200);
    assert_eq!(http_delete(&base2, "/api/themes/custom-a").await, 404);
}

#[tokio::test]
async fn theme_put_rejects_bad_bodies_and_foreign_origins() {
    let state = tempfile::tempdir().unwrap();
    let base = start(cfg(state.path())).await;
    let ok = body("custom-a", "A");

    let (status, _) = http_put(&base, "/api/themes/custom-a", &[], &ok).await;
    assert_eq!(status, 403, "no JSON content type");
    let (status, _) = http_put(
        &base,
        "/api/themes/custom-a",
        &[JSON, ("Origin", "https://evil.example")],
        &ok,
    )
    .await;
    assert_eq!(status, 403, "cross-site origin");
    let (status, _) = http_put(&base, "/api/themes/custom-b", &[JSON], &ok).await;
    assert_eq!(status, 400, "id mismatch");
    let (status, _) = http_put(
        &base,
        "/api/themes/custom-a",
        &[JSON],
        r#"{"id":"custom-a"}"#,
    )
    .await;
    assert_eq!(status, 400, "incomplete");

    let list: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base, "/api/themes").await).unwrap();
    assert!(list.is_empty());
}
