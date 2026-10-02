//! GET/POST/DELETE /api/snoozes: tabs closed until a wake time, persisted under
//! `state_dir` so they survive a daemon restart.

use eigenform_daemon::{app, Config};

mod helpers;
use helpers::{http_delete, http_get, http_post};

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

#[tokio::test]
async fn snoozes_round_trip_sorted_and_survive_restart() {
    let state = tempfile::tempdir().unwrap();
    let base = start(cfg(state.path())).await;

    let (status, body) = http_post(
        &base,
        "/api/snoozes",
        &[JSON],
        r#"{"until":2000,"tab":{"label":"later","uuid":"u-2"}}"#,
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let late: serde_json::Value = serde_json::from_str(&body).unwrap();
    let (status, _) = http_post(
        &base,
        "/api/snoozes",
        &[JSON],
        r#"{"until":1000,"tab":{"label":"sooner","ptyId":"7"}}"#,
    )
    .await;
    assert_eq!(status, 201);

    let list: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base, "/api/snoozes").await).unwrap();
    let labels: Vec<_> = list
        .iter()
        .map(|s| s["tab"]["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["sooner", "later"], "soonest wake first");
    assert!(state.path().join("snoozes.json").exists());

    // A fresh daemon over the same state dir sees both.
    let base2 = start(cfg(state.path())).await;
    let list2: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base2, "/api/snoozes").await).unwrap();
    assert_eq!(list2.len(), 2);

    // Delete is the claim: the first succeeds, a second (another window) gets 404.
    let id = late["id"].as_str().unwrap();
    assert_eq!(
        http_delete(&base2, &format!("/api/snoozes/{id}")).await,
        200
    );
    assert_eq!(
        http_delete(&base2, &format!("/api/snoozes/{id}")).await,
        404
    );
    let list3: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base2, "/api/snoozes").await).unwrap();
    assert_eq!(list3.len(), 1);
}

#[tokio::test]
async fn snooze_create_rejects_bad_bodies_and_foreign_origins() {
    let state = tempfile::tempdir().unwrap();
    let base = start(cfg(state.path())).await;
    let ok = r#"{"until":1000,"tab":{"label":"x"}}"#;

    // No JSON content type → forbidden (no CORS-simple POSTs).
    let (status, _) = http_post(&base, "/api/snoozes", &[], ok).await;
    assert_eq!(status, 403);
    // Cross-site origin → forbidden.
    let (status, _) = http_post(
        &base,
        "/api/snoozes",
        &[JSON, ("Origin", "https://evil.example")],
        ok,
    )
    .await;
    assert_eq!(status, 403);
    // Missing label / non-positive wake time → bad request.
    let (status, _) = http_post(&base, "/api/snoozes", &[JSON], r#"{"until":1000,"tab":{}}"#).await;
    assert_eq!(status, 400);
    let (status, _) = http_post(
        &base,
        "/api/snoozes",
        &[JSON],
        r#"{"until":0,"tab":{"label":"x"}}"#,
    )
    .await;
    assert_eq!(status, 400);

    let list: Vec<serde_json::Value> =
        serde_json::from_str(&http_get(&base, "/api/snoozes").await).unwrap();
    assert!(list.is_empty());
}
