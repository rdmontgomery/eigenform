//! `/api/feeds` and `/api/feeds/act` against a stub provider: the daemon lists what a
//! loopback feed advertises, reports a down or non-loopback feed without failing the
//! rest, and posts only actions it saw in the snapshot.

mod helpers;

use std::sync::{Arc, Mutex};

use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use eigenform_daemon::{app, Config};

const JSON: (&str, &str) = ("content-type", "application/json");

type Received = Arc<Mutex<Vec<Value>>>;

/// A provider serving one item, recording what its action endpoints receive.
async fn provider() -> (String, Received) {
    let received: Received = Arc::default();
    let rec = received.clone();
    let fail = Router::new().route(
        "/api/fail",
        post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
    );
    let router = Router::new()
        .route(
            "/feed",
            get(|| async {
                Json(json!({ "items": [
                    { "id": "review:o/r#1", "kind": "review", "title": "o/r#1",
                      "link": "https://github.com/o/r/pull/1", "at": "2026-11-12T10:00:00Z",
                      "actions": [
                        { "label": "Start", "post": "/api/launch", "body": { "key": "o/r#1" } },
                        { "label": "Break", "post": "/api/fail" }
                      ] },
                    { "id": "bad" }
                ]}))
            }),
        )
        .route(
            "/api/launch",
            post(move |Json(v): Json<Value>| async move {
                rec.lock().unwrap().push(v);
                Json(json!({ "ok": true }))
            }),
        )
        .merge(fail);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), received)
}

async fn daemon(feeds_json: &str) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("feeds.json");
    std::fs::write(&file, feeds_json).unwrap();
    let cfg = Config {
        program: "sh".into(),
        feeds_file: Some(file),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app(cfg)).await.unwrap() });
    (format!("http://{addr}"), dir)
}

async fn feeds(base: &str) -> Value {
    serde_json::from_str(&helpers::http_get(base, "/api/feeds").await).unwrap()
}

#[tokio::test]
async fn lists_items_and_reports_unreachable_and_non_loopback_feeds() {
    let (prov, _) = provider().await;
    // A port nothing listens on: bind then drop.
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    let (base, _dir) = daemon(&format!(
        r#"[{{"name":"tasks","url":"{prov}/feed"}},
            {{"name":"down","url":"http://127.0.0.1:{dead_port}/feed"}},
            {{"name":"remote","url":"http://example.com/feed"}}]"#
    ))
    .await;
    let v = feeds(&base).await;
    let list = v["feeds"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0]["name"], "tasks");
    assert_eq!(list[0]["ok"], true);
    let items = list[0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "the invalid item is dropped alone");
    assert_eq!(items[0]["title"], "o/r#1");
    assert_eq!(list[1]["ok"], false);
    assert_eq!(list[2]["ok"], false);
    assert!(list[2]["error"].as_str().unwrap().contains("loopback"));
}

#[tokio::test]
async fn no_file_means_no_feeds_and_a_bad_file_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        program: "sh".into(),
        feeds_file: Some(dir.path().join("absent.json")),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app(cfg)).await.unwrap() });
    let v = feeds(&format!("http://{addr}")).await;
    assert_eq!(v, json!({ "feeds": [] }));

    let (base, _d) = daemon("not json").await;
    let v = feeds(&base).await;
    assert_eq!(v["feeds"], json!([]));
    assert!(v["error"].as_str().unwrap().starts_with("feeds.json"));
}

#[tokio::test]
async fn act_posts_only_what_the_snapshot_advertised() {
    let (prov, received) = provider().await;
    let (base, _dir) = daemon(&format!(r#"[{{"name":"tasks","url":"{prov}/feed"}}]"#)).await;

    let act = |body: &'static str| {
        let base = base.clone();
        async move { helpers::http_post(&base, "/api/feeds/act", &[JSON], body).await }
    };
    // Before any fetch there is no snapshot.
    let (code, _) = act(r#"{"feed":"tasks","item":"review:o/r#1","action":0}"#).await;
    assert_eq!(code, 409);

    feeds(&base).await;
    let (code, body) = act(r#"{"feed":"tasks","item":"review:o/r#1","action":0}"#).await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(*received.lock().unwrap(), vec![json!({ "key": "o/r#1" })]);

    let (code, body) = act(r#"{"feed":"tasks","item":"review:o/r#1","action":1}"#).await;
    assert_eq!(code, 502);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], 500);
    assert_eq!(v["error"], "boom");

    for unknown in [
        r#"{"feed":"tasks","item":"review:o/r#1","action":7}"#,
        r#"{"feed":"tasks","item":"nope","action":0}"#,
        r#"{"feed":"other","item":"review:o/r#1","action":0}"#,
    ] {
        assert_eq!(act(unknown).await.0, 409, "{unknown}");
    }
    assert_eq!(act("junk").await.0, 400);

    let good = r#"{"feed":"tasks","item":"review:o/r#1","action":0}"#;
    let (code, _) = helpers::http_post(&base, "/api/feeds/act", &[], good).await;
    assert_eq!(code, 403, "a JSON content type is required");
    let (code, _) = helpers::http_post(
        &base,
        "/api/feeds/act",
        &[JSON, ("origin", "https://evil.example")],
        good,
    )
    .await;
    assert_eq!(code, 403, "a foreign origin is refused");
    assert_eq!(received.lock().unwrap().len(), 1);
}
