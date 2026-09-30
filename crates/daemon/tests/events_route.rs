//! The observability event routes: `GET /api/events` (with `?since=`) and the
//! `pty-spawned` / `pty-exited` instrumentation, driven against a real shell.

mod helpers;

use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

use eigenform_daemon::{app, Config};

/// Spawn the daemon with `program: "sh"` so a bare `/pty` runs a shell. Returns the
/// `http://addr` base.
async fn start() -> String {
    let cfg = Config {
        program: "sh".to_string(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
        log_file: None,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

fn ws_url(base: &str, query: &str) -> String {
    let host = base.strip_prefix("http://").unwrap();
    if query.is_empty() {
        format!("ws://{host}/pty")
    } else {
        format!("ws://{host}/pty?{query}")
    }
}

async fn first_text_frame(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> serde_json::Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                return serde_json::from_str(&t).expect("text frame is JSON");
            }
            Ok(Some(Ok(_))) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

/// Poll `GET /api/events` until an event of `kind` for pty `id` appears, or give up.
async fn wait_for_event(base: &str, kind: &str, id: &str) -> Option<serde_json::Value> {
    for _ in 0..40 {
        let body = helpers::http_get(base, "/api/events").await;
        let arr: serde_json::Value = serde_json::from_str(&body).expect("json array");
        if let Some(ev) = arr.as_array().unwrap().iter().find(|e| {
            e["kind"] == serde_json::json!(kind) && e["data"]["id"] == serde_json::json!(id)
        }) {
            return Some(ev.clone());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

#[tokio::test]
async fn empty_events_is_a_json_array() {
    let base = start().await;
    let body = helpers::http_get(&base, "/api/events").await;
    let arr: serde_json::Value = serde_json::from_str(&body).expect("json array");
    assert!(arr.as_array().unwrap().is_empty(), "no events yet");
}

#[tokio::test]
async fn spawning_a_pty_records_pty_spawned_with_the_wire_shape() {
    let base = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .expect("connect bare");
    let hello = first_text_frame(&mut ws).await;
    let id = hello["id"].as_str().unwrap().to_string();

    let ev = wait_for_event(&base, "pty-spawned", &id)
        .await
        .expect("pty-spawned must be recorded");
    // Pin the event wire contract every consumer relies on.
    assert!(ev["seq"].as_u64().is_some(), "seq is a number: {ev}");
    assert!(ev["at"].as_str().is_some(), "at is an ISO string: {ev}");
    assert_eq!(ev["kind"], "pty-spawned");
    assert_eq!(ev["data"]["program"], "sh");

    ws.close(None).await.ok();
    drop(ws);
    // Kill the pty so the pump reaches EOF and records pty-exited.
    helpers::http_delete(&base, &format!("/api/pty/{id}")).await;
}

#[tokio::test]
async fn since_returns_only_newer_events() {
    let base = start().await;
    // First pty → at least one event.
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .unwrap();
    let id1 = first_text_frame(&mut ws).await["id"].as_str().unwrap().to_string();
    wait_for_event(&base, "pty-spawned", &id1).await.expect("first spawn recorded");

    // Read the current high-water seq.
    let body = helpers::http_get(&base, "/api/events").await;
    let arr: serde_json::Value = serde_json::from_str(&body).unwrap();
    let max_seq = arr.as_array().unwrap().iter().map(|e| e["seq"].as_u64().unwrap()).max().unwrap();

    // Second pty → a new event with a higher seq.
    let (mut ws2, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .unwrap();
    let id2 = first_text_frame(&mut ws2).await["id"].as_str().unwrap().to_string();
    wait_for_event(&base, "pty-spawned", &id2).await.expect("second spawn recorded");

    // ?since=max_seq must exclude everything from before the second pty.
    let body = helpers::http_get(&base, &format!("/api/events?since={max_seq}")).await;
    let newer: serde_json::Value = serde_json::from_str(&body).unwrap();
    let newer = newer.as_array().unwrap();
    assert!(!newer.is_empty(), "there must be at least the second spawn");
    assert!(
        newer.iter().all(|e| e["seq"].as_u64().unwrap() > max_seq),
        "?since must return only events with seq > since: {newer:?}"
    );
    assert!(
        newer.iter().any(|e| e["data"]["id"] == serde_json::json!(id2)),
        "the second pty's spawn must be among the newer events"
    );

    for id in [&id1, &id2] {
        helpers::http_delete(&base, &format!("/api/pty/{id}")).await;
    }
}
