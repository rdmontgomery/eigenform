//! The websocket↔pty bridge, driven against a dummy command (no claude, no tokens).

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use eigenform_daemon::{app, Config};

async fn start() -> String {
    let cfg = Config {
        program: "cat".to_string(),
        args: vec![],
        cwd: None,
        web_dir: None,
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        dev: false,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("ws://{addr}/pty")
}

#[tokio::test]
async fn ws_forwards_stdin_and_streams_pty_output() {
    let url = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.expect("connect");

    // stdin control message → pty; cat echoes it back as binary output
    ws.send(Message::Text(r#"{"type":"stdin","data":"ping\n"}"#.to_string()))
        .await
        .unwrap();

    let mut got = Vec::new();
    for _ in 0..50 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                got.extend_from_slice(&b);
                if String::from_utf8_lossy(&got).contains("ping") {
                    break;
                }
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(
        String::from_utf8_lossy(&got).contains("ping"),
        "got: {:?}",
        String::from_utf8_lossy(&got)
    );
}

#[tokio::test]
async fn ws_rejects_a_cross_origin_connection() {
    let url = start().await;
    let mut req = url.as_str().into_client_request().unwrap();
    req.headers_mut()
        .insert("origin", "http://evil.example".parse().unwrap());
    // A page you visit must not be able to open a shell on your localhost daemon.
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
}

#[tokio::test]
async fn ws_allows_a_localhost_origin() {
    let url = start().await;
    let host = url.trim_start_matches("ws://").trim_end_matches("/pty");
    let mut req = url.as_str().into_client_request().unwrap();
    req.headers_mut()
        .insert("origin", format!("http://{host}").parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_ok());
}

#[tokio::test]
async fn ws_accepts_a_resize_message_without_closing() {
    let url = start().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.expect("connect");

    ws.send(Message::Text(r#"{"type":"resize","cols":100,"rows":40}"#.to_string()))
        .await
        .unwrap();
    // still usable afterward: stdin still round-trips
    ws.send(Message::Text(r#"{"type":"stdin","data":"after-resize\n"}"#.to_string()))
        .await
        .unwrap();

    let mut got = Vec::new();
    for _ in 0..50 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                got.extend_from_slice(&b);
                if String::from_utf8_lossy(&got).contains("after-resize") {
                    break;
                }
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert!(String::from_utf8_lossy(&got).contains("after-resize"));
}
