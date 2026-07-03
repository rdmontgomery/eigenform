//! The attach protocol + registry endpoints (Task 1.7), driven against a real shell
//! (`sh`) so bare `/pty` registers a live pty. No claude, no tokens.

mod helpers;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use eigenform_daemon::{app, Config};

/// Spawn the daemon with `program: "sh"` so a bare `/pty` runs a shell. Returns the
/// `http://addr` base; callers derive the `ws://` url from it.
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
        rephrase_cmd: vec!["claude".to_string(), "-p".to_string()],
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

/// Like [`start`], but with a `sessions_dir` configured so `GET /api/pty` reconciles
/// against `sessions/<pid>.json`. Returns the `http://addr` base.
async fn start_with_sessions(sessions_dir: std::path::PathBuf) -> String {
    let cfg = Config {
        program: "sh".to_string(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: None,
        sessions_dir: Some(sessions_dir),
        state_dir: None,
        workspace_root: None,
        dev: false,
        rephrase_cmd: vec!["claude".to_string(), "-p".to_string()],
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

/// Read frames until the first text frame; parse it as JSON and return it.
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

#[tokio::test]
async fn reattach_repaints_prior_output() {
    let base = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .expect("connect bare");

    // First frame announces the pty id.
    let hello = first_text_frame(&mut ws).await;
    assert_eq!(hello["type"], "pty");
    let id = hello["id"].as_str().expect("id is a string").to_string();

    // Emit a distinctive marker into the shell.
    ws.send(Message::Text(
        r#"{"type":"stdin","data":"printf REPAINT_ME\n"}"#.to_string(),
    ))
    .await
    .unwrap();

    // Wait until the marker shows in this socket's stream, so the model has it.
    let mut seen = Vec::new();
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                seen.extend_from_slice(&b);
                if String::from_utf8_lossy(&seen).contains("REPAINT_ME") {
                    break;
                }
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(
        String::from_utf8_lossy(&seen).contains("REPAINT_ME"),
        "marker must appear in the live stream first"
    );

    // Close, then re-attach: the snapshot (first binary frames) must repaint the marker.
    ws.close(None).await.ok();
    drop(ws);

    let (mut ws2, _) = tokio_tungstenite::connect_async(ws_url(&base, &format!("attach={id}")))
        .await
        .expect("connect attach");
    let hello2 = first_text_frame(&mut ws2).await;
    assert_eq!(hello2["type"], "pty");
    assert_eq!(hello2["id"], id, "attach reports the same id");

    let mut repaint = Vec::new();
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_secs(2), ws2.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                repaint.extend_from_slice(&b);
                if String::from_utf8_lossy(&repaint).contains("REPAINT_ME") {
                    break;
                }
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(
        String::from_utf8_lossy(&repaint).contains("REPAINT_ME"),
        "re-attach must repaint prior output, got: {:?}",
        String::from_utf8_lossy(&repaint)
    );
}

#[tokio::test]
async fn closing_the_socket_leaves_the_pty_listed() {
    let base = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .expect("connect bare");
    let hello = first_text_frame(&mut ws).await;
    let id = hello["id"].as_str().unwrap().to_string();

    ws.close(None).await.ok();
    drop(ws);

    // The pty outlives the socket: GET /api/pty still lists it.
    // Poll briefly so the detach has settled.
    let mut row: Option<serde_json::Value> = None;
    for _ in 0..20 {
        let body = helpers::http_get(&base, "/api/pty").await;
        let arr: serde_json::Value = serde_json::from_str(&body).expect("json array");
        if let Some(r) = arr.as_array().unwrap().iter().find(|p| p["id"] == id) {
            row = Some(r.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let row = row.expect("the pty must remain listed after the socket closes");
    // Pin the camelCase contract: every row must carry these keys.
    assert!(
        row.get("spawnedAt").is_some(),
        "row must carry `spawnedAt`: {row}"
    );
    assert!(
        row.get("lastActivity").is_some(),
        "row must carry `lastActivity`: {row}"
    );
    assert!(
        row.get("state").is_some(),
        "row must carry `state`: {row}"
    );
    // The classifier replaced the "unknown" placeholder (Task 1.9). A freshly-spawned
    // sh pty is either still streaming its prompt (working) or quiet (idle) — never
    // "unknown", and never "waiting" (that's claude-specific; spike-08 grid is unit-tested).
    let st = row["state"].as_str().unwrap();
    assert!(
        st == "working" || st == "idle",
        "fresh sh pty state must be working or idle, got {st:?}"
    );
}

#[tokio::test]
async fn get_api_pty_reconciles_uuid_from_pid_file() {
    let dir = tempfile::tempdir().unwrap();
    let base = start_with_sessions(dir.path().to_path_buf()).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .expect("connect bare");
    let hello = first_text_frame(&mut ws).await;
    let id = hello["id"].as_str().unwrap().to_string();

    // The pty's child is this `sh`, so `$$` is the pid claude's authority file would key
    // on. Have the shell write its own `sessions/<pid>.json` with a known sessionId.
    let sessions = dir.path().display().to_string();
    ws.send(Message::Text(format!(
        r#"{{"type":"stdin","data":"printf '{{\"pid\":%d,\"sessionId\":\"sess-xyz\",\"cwd\":\"/tmp\"}}' $$ > {sessions}/$$.json\n"}}"#,
    )))
    .await
    .unwrap();

    // Poll GET /api/pty until reconcile (run inline by the handler) adopts the uuid.
    let mut uuid: Option<serde_json::Value> = None;
    for _ in 0..40 {
        let body = helpers::http_get(&base, "/api/pty").await;
        let arr: serde_json::Value = serde_json::from_str(&body).expect("json array");
        if let Some(r) = arr.as_array().unwrap().iter().find(|p| p["id"] == id) {
            if r["uuid"] == serde_json::json!("sess-xyz") {
                uuid = Some(r["uuid"].clone());
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        uuid,
        Some(serde_json::json!("sess-xyz")),
        "GET /api/pty must reconcile the uuid from sessions/<pid>.json"
    );

    // Hygiene: kill the shell so no stray process lingers.
    helpers::http_delete(&base, &format!("/api/pty/{id}")).await;
}

#[tokio::test]
async fn delete_kills_and_unlists() {
    let base = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .expect("connect bare");
    let hello = first_text_frame(&mut ws).await;
    let id = hello["id"].as_str().unwrap().to_string();
    ws.close(None).await.ok();
    drop(ws);

    // DELETE → 204, then GET /api/pty no longer lists it.
    let code = helpers::http_delete(&base, &format!("/api/pty/{id}")).await;
    assert_eq!(code, 204, "kill returns 204 No Content");

    let body = helpers::http_get(&base, "/api/pty").await;
    let arr: serde_json::Value = serde_json::from_str(&body).expect("json array");
    assert!(
        !arr.as_array().unwrap().iter().any(|p| p["id"] == id),
        "killed pty must be gone from the list"
    );

    // DELETE again → 404 (unknown id).
    let code = helpers::http_delete(&base, &format!("/api/pty/{id}")).await;
    assert_eq!(code, 404, "second kill is 404 NotFound");
}

#[tokio::test]
async fn attach_to_a_missing_id_closes_the_socket() {
    let base = start().await;
    // No such id: the handshake upgrades, then the server closes with a reason.
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, "attach=999999"))
        .await
        .expect("upgrade ok even for a missing id");
    // We should get a Close frame (or stream end), not a pty/text frame.
    match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
        Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => {}
        other => panic!("expected the socket to close, got {other:?}"),
    }
}

#[tokio::test]
async fn attach_to_an_exited_pty_repaints_then_signals_exit() {
    let base = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(&base, ""))
        .await
        .expect("connect bare");
    let hello = first_text_frame(&mut ws).await;
    let id = hello["id"].as_str().unwrap().to_string();

    // Print a marker, then exit the shell so the pty's child dies.
    ws.send(Message::Text(
        r#"{"type":"stdin","data":"printf GOODBYE\n"}"#.to_string(),
    ))
    .await
    .unwrap();
    // Drain until we see GOODBYE (model has it), then ask the shell to exit.
    let mut seen = Vec::new();
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                seen.extend_from_slice(&b);
                if String::from_utf8_lossy(&seen).contains("GOODBYE") {
                    break;
                }
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    ws.send(Message::Text(r#"{"type":"stdin","data":"exit\n"}"#.to_string()))
        .await
        .unwrap();
    // Drain this socket until it closes / we see the exit frame, confirming the child died.
    let mut got_exit = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) if t.contains("\"exit\"") => {
                got_exit = true;
                break;
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(got_exit, "the live socket should observe the exit");
    drop(ws);

    // Now attach to the EXITED pty (still within the sweep window). We must get:
    //  - the pty hello frame,
    //  - the repaint snapshot (GOODBYE),
    //  - then an {"type":"exit"} frame synthesized to this late subscriber.
    let (mut ws2, _) = tokio_tungstenite::connect_async(ws_url(&base, &format!("attach={id}")))
        .await
        .expect("attach to exited pty");
    let hello2 = first_text_frame(&mut ws2).await;
    assert_eq!(hello2["type"], "pty");

    let mut got_repaint = false;
    let mut got_exit2 = false;
    for _ in 0..30 {
        match tokio::time::timeout(Duration::from_secs(2), ws2.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                if String::from_utf8_lossy(&b).contains("GOODBYE") {
                    got_repaint = true;
                }
            }
            Ok(Some(Ok(Message::Text(t)))) if t.contains("\"exit\"") => {
                got_exit2 = true;
                break;
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(got_repaint, "attach to exited pty must repaint prior output");
    assert!(got_exit2, "attach to exited pty must synthesize an exit frame");
}
