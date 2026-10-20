//! `POST /api/focus`: the deep-link contract Stead depends on. 200 when a window is
//! listening on `/api/events/stream`, 202 (held, replayed to the next window) when none
//! is, 404 for a uuid that is neither a live pty nor a forest session, 400 for junk.

mod helpers;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

use eigenform_daemon::{app, Config};

/// A session the forest knows (JSONL + a live pid file for the test process).
const FOREST_UUID: &str = "cccc3333-0000-4000-8000-000000000003";
/// A session only a live pty knows (adopted from the pty child's own pid file).
const PTY_UUID: &str = "dddd4444-0000-4000-8000-000000000004";
const UNKNOWN_UUID: &str = "eeee5555-0000-4000-8000-000000000005";

const JSON: (&str, &str) = ("content-type", "application/json");

struct Fixture {
    _proj: tempfile::TempDir,
    sess: tempfile::TempDir,
    _state: tempfile::TempDir,
    base: String,
}

async fn start() -> Fixture {
    let proj = tempfile::tempdir().unwrap();
    let sess = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let pdir = proj.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(
        pdir.join(format!("{FOREST_UUID}.jsonl")),
        format!(
            "{{\"type\":\"user\",\"timestamp\":\"2026-06-06T10:00:00Z\",\"sessionId\":\"{FOREST_UUID}\",\"cwd\":\"/home/me/p\",\"message\":{{\"role\":\"user\"}}}}\n"
        ),
    )
    .unwrap();
    let pid = std::process::id();
    std::fs::write(
        sess.path().join(format!("{pid}.json")),
        format!("{{\"pid\":{pid},\"sessionId\":\"{FOREST_UUID}\",\"cwd\":\"/home/me/p\"}}"),
    )
    .unwrap();

    let cfg = Config {
        program: "sh".into(),
        projects_dir: Some(proj.path().to_path_buf()),
        sessions_dir: Some(sess.path().to_path_buf()),
        state_dir: Some(state.path().to_path_buf()),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    Fixture {
        _proj: proj,
        sess,
        _state: state,
        base: format!("http://{addr}"),
    }
}

async fn focus(base: &str, body: &str) -> (u16, String) {
    helpers::http_post(base, "/api/focus", &[JSON], body).await
}

fn body_for(uuid: &str) -> String {
    format!("{{\"session\":\"{uuid}\"}}")
}

/// Open `/api/events/stream` over raw TCP and wait for the response head, so the
/// daemon has registered the client before the caller POSTs.
async fn open_stream(base: &str) -> tokio::net::TcpStream {
    let host = base.strip_prefix("http://").unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let req = format!("GET /api/events/stream HTTP/1.1\r\nHost: {host}\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut byte))
            .await
            .expect("stream head")
            .unwrap();
        assert!(n > 0, "stream closed before its head");
        head.push(byte[0]);
    }
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"));
    stream
}

/// Read SSE frames until one whose JSON `kind` is `focus`, or give up after ~5s.
async fn next_focus_frame(stream: &mut tokio::net::TcpStream) -> Option<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    loop {
        let found = acc
            .lines()
            .filter_map(|l| l.strip_prefix("data: ").or_else(|| l.strip_prefix("data:")))
            .filter_map(|d| serde_json::from_str::<serde_json::Value>(d).ok())
            .find(|v| v["kind"] == "focus");
        if found.is_some() || tokio::time::Instant::now() >= deadline {
            return found;
        }
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await
        {
            if n == 0 {
                return None;
            }
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
}

#[tokio::test]
async fn junk_is_rejected_before_any_lookup() {
    let f = start().await;
    assert_eq!(focus(&f.base, "not json").await.0, 400);
    assert_eq!(focus(&f.base, "{\"uuid\":\"x\"}").await.0, 400);
    assert_eq!(focus(&f.base, "{\"session\":\"../../etc\"}").await.0, 400);
    let (code, _) = helpers::http_post(&f.base, "/api/focus", &[], &body_for(FOREST_UUID)).await;
    assert_eq!(code, 415, "a JSON content type is required");
    let (code, _) = helpers::http_post(
        &f.base,
        "/api/focus",
        &[JSON, ("origin", "https://evil.example")],
        &body_for(FOREST_UUID),
    )
    .await;
    assert_eq!(code, 403, "a non-local browser origin is refused");
}

#[tokio::test]
async fn an_unknown_session_is_404() {
    let f = start().await;
    assert_eq!(focus(&f.base, &body_for(UNKNOWN_UUID)).await.0, 404);
}

#[tokio::test]
async fn with_no_window_the_request_is_held_and_replayed_to_the_first_one() {
    let f = start().await;
    let (code, body) = focus(&f.base, &body_for(FOREST_UUID)).await;
    assert_eq!(code, 202, "no window connected: {body}");

    let mut first = open_stream(&f.base).await;
    let ev = next_focus_frame(&mut first)
        .await
        .expect("the held request replays as the first window's first frame");
    assert_eq!(ev["data"]["session"], FOREST_UUID);
    assert_eq!(ev["data"]["replayed"], true, "a replay is marked as one");

    // Taken: a second window opening now does not replay it again.
    let mut second = open_stream(&f.base).await;
    assert!(next_focus_frame_quick(&mut second).await.is_none());

    // It is on the bus like any event.
    let all: serde_json::Value =
        serde_json::from_str(&helpers::http_get(&f.base, "/api/events").await).unwrap();
    assert!(all
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "focus" && e["data"]["session"] == FOREST_UUID));
}

/// Like [`next_focus_frame`] but only waits ~600ms (for asserting absence).
async fn next_focus_frame_quick(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    for _ in 0..3 {
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(200), stream.read(&mut buf)).await
        {
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
    acc.contains("\"focus\"").then_some(acc)
}

#[tokio::test]
async fn with_a_window_connected_the_event_is_delivered_live() {
    let f = start().await;
    let mut win = open_stream(&f.base).await;
    let (code, body) = focus(&f.base, &body_for(FOREST_UUID)).await;
    assert_eq!(code, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["delivered"], 1);
    let ev = next_focus_frame(&mut win).await.expect("live focus frame");
    assert_eq!(ev["data"]["session"], FOREST_UUID);
    assert!(
        ev["data"].get("replayed").is_none(),
        "a live event is not a replay"
    );

    // Uppercase is the same uuid, normalized.
    let (code, _) = focus(&f.base, &body_for(&FOREST_UUID.to_uppercase())).await;
    assert_eq!(code, 200);
    let ev = next_focus_frame(&mut win)
        .await
        .expect("second focus frame");
    assert_eq!(ev["data"]["session"], FOREST_UUID);
}

#[tokio::test]
async fn a_closed_window_stops_counting() {
    let f = start().await;
    let win = open_stream(&f.base).await;
    drop(win);
    // The daemon notices the drop on its next write (at worst the 10s keep-alive).
    let mut code = 0;
    for _ in 0..60 {
        code = focus(&f.base, &body_for(FOREST_UUID)).await.0;
        if code == 202 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(code, 202, "a dropped stream must stop counting as a window");
}

#[tokio::test]
async fn a_live_pty_session_is_known_even_outside_the_forest() {
    let f = start().await;
    let host = f.base.strip_prefix("http://").unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{host}/pty"))
        .await
        .unwrap();
    // The shell writes its own claude-style pid file, which the daemon adopts as the
    // pty's session uuid (the `sessions/<pid>.json` reconcile path).
    let pidfile = format!(
        "echo '{{\"pid\":'$$',\"sessionId\":\"{PTY_UUID}\"}}' > {}/$$.json\n",
        f.sess.path().display()
    );
    let stdin = serde_json::json!({ "type": "stdin", "data": pidfile }).to_string();
    ws.send(Message::Text(stdin.into())).await.unwrap();

    let mut adopted = false;
    for _ in 0..40 {
        let ptys: serde_json::Value =
            serde_json::from_str(&helpers::http_get(&f.base, "/api/pty").await).unwrap();
        if ptys
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["uuid"] == PTY_UUID)
        {
            adopted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(adopted, "the pty adopted its session uuid");
    // The forest lists any live pid file; remove it so only the pty knows the uuid
    // (adoption is sticky: the pty keeps it).
    for entry in std::fs::read_dir(f.sess.path()).unwrap().flatten() {
        if std::fs::read_to_string(entry.path()).is_ok_and(|t| t.contains(PTY_UUID)) {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    let forest = helpers::http_get(&f.base, "/api/forest").await;
    assert!(!forest.contains(PTY_UUID), "not a forest session: {forest}");

    assert_eq!(focus(&f.base, &body_for(PTY_UUID)).await.0, 202);

    // Drain so the pty isn't blocked, then end it.
    let _ = tokio::time::timeout(Duration::from_millis(100), ws.next()).await;
    let ptys: serde_json::Value =
        serde_json::from_str(&helpers::http_get(&f.base, "/api/pty").await).unwrap();
    for p in ptys.as_array().unwrap() {
        helpers::http_delete(&f.base, &format!("/api/pty/{}", p["id"].as_str().unwrap())).await;
    }
}
