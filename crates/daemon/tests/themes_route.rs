//! GET/PUT/DELETE /api/themes: custom color themes, persisted under `state_dir`, with a
//! `themes-changed` event on every change so open windows refresh.

use eigenform_daemon::{app, Config};

mod helpers;
use helpers::{http_delete, http_get, http_send};

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

const DUSK: &str =
    r##"{"name":"Dusk","theme":{"background":"#1A1B26","foreground":"#c0caf5","red":"#f77"}}"##;

#[tokio::test]
async fn themes_round_trip_survive_restart_and_announce_changes() {
    let state = tempfile::tempdir().unwrap();
    let base = start(cfg(state.path())).await;

    let (status, body) = http_send("PUT", &base, "/api/themes/dusk", &[JSON], DUSK).await;
    assert_eq!(status, 200, "{body}");
    let list: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(list[0]["id"], "dusk");
    assert_eq!(list[0]["theme"]["background"], "#1a1b26", "lowercased");
    assert_eq!(list[0]["theme"]["red"], "#ff7777", "#rgb expanded");

    let events: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base, "/api/events").await).unwrap();
    assert!(events
        .iter()
        .any(|e| e["kind"] == "themes-changed" && e["data"]["id"] == "dusk"));

    // A fresh daemon over the same state dir sees it; delete is 200 then 404.
    let base2 = start(cfg(state.path())).await;
    let list2: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base2, "/api/themes").await).unwrap();
    assert_eq!(list2.len(), 1);
    assert_eq!(http_delete(&base2, "/api/themes/dusk").await, 200);
    assert_eq!(http_delete(&base2, "/api/themes/dusk").await, 404);
}

#[tokio::test]
async fn theme_put_rejects_bad_bodies_and_foreign_origins() {
    let state = tempfile::tempdir().unwrap();
    let base = start(cfg(state.path())).await;

    let (status, _) = http_send("PUT", &base, "/api/themes/dusk", &[], DUSK).await;
    assert_eq!(status, 403, "no JSON content type");
    let evil = [JSON, ("Origin", "https://evil.example")];
    let (status, _) = http_send("PUT", &base, "/api/themes/dusk", &evil, DUSK).await;
    assert_eq!(status, 403, "cross-site origin");

    let (status, body) = http_send("PUT", &base, "/api/themes/Not%20Ok", &[JSON], DUSK).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = http_send(
        "PUT",
        &base,
        "/api/themes/dusk",
        &[JSON],
        r##"{"name":"x","theme":{"background":"#000000"}}"##,
    )
    .await;
    assert_eq!(status, 400);
    assert!(body.contains("foreground"), "says what's missing: {body}");

    let list: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base, "/api/themes").await).unwrap();
    assert!(list.is_empty());
}
