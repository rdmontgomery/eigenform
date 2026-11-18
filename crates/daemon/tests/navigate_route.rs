//! `GET /s/:ref` and `GET /api/resolve/:ref` (a session by uuid, uuid prefix or peer
//! name), and `/api/pins` (pinned sessions, persisted across a daemon restart).

mod helpers;

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use eigenform_daemon::{app, Config};

const LIVE: &str = "abcd1111-0000-4000-8000-000000000001";
const ASLEEP: &str = "abcd2222-0000-4000-8000-000000000002";
const PEER: &str = "app-dd";

const JSON: (&str, &str) = ("content-type", "application/json");

struct Fixture {
    _proj: tempfile::TempDir,
    _sess: tempfile::TempDir,
    state: tempfile::TempDir,
    base: String,
}

fn transcript(uuid: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"timestamp\":\"2026-06-06T10:00:00Z\",\"sessionId\":\"{uuid}\",\"cwd\":\"/home/me/p\",\"message\":{{\"role\":\"user\"}}}}\n"
    )
}

async fn serve(proj: &std::path::Path, sess: &std::path::Path, state: &std::path::Path) -> String {
    let cfg = Config {
        program: "sh".into(),
        projects_dir: Some(proj.to_path_buf()),
        sessions_dir: Some(sess.to_path_buf()),
        state_dir: Some(state.to_path_buf()),
        ..Default::default()
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

async fn start() -> Fixture {
    let proj = tempfile::tempdir().unwrap();
    let sess = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let pdir = proj.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    for uuid in [LIVE, ASLEEP] {
        std::fs::write(pdir.join(format!("{uuid}.jsonl")), transcript(uuid)).unwrap();
    }
    // The test process stands in for the live session's claude, so its claim is alive.
    let pid = std::process::id();
    std::fs::write(
        sess.path().join(format!("{pid}.json")),
        format!(
            "{{\"pid\":{pid},\"sessionId\":\"{LIVE}\",\"cwd\":\"/home/me/p\",\"name\":\"{PEER}\"}}"
        ),
    )
    .unwrap();
    let base = serve(proj.path(), sess.path(), state.path()).await;
    Fixture {
        _proj: proj,
        _sess: sess,
        state,
        base,
    }
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}: ")))
        .map(str::trim)
}

#[tokio::test]
async fn a_link_with_no_window_open_redirects_into_the_app() {
    let f = start().await;
    for r in ["abcd1", PEER, "APP-DD", LIVE] {
        let (code, head, _) = helpers::http_get_full(&f.base, &format!("/s/{r}")).await;
        assert_eq!(code, 303, "{r}");
        assert_eq!(
            header(&head, "location"),
            Some(format!("/?session={LIVE}").as_str()),
            "{r}"
        );
    }
    let (code, head, _) = helpers::http_get_full(&f.base, "/s/abcd2").await;
    assert_eq!(code, 303, "a session that isn't running resolves by id");
    assert_eq!(
        header(&head, "location"),
        Some(format!("/?session={ASLEEP}").as_str())
    );
}

#[tokio::test]
async fn an_ambiguous_or_unknown_link_lists_rather_than_guesses() {
    let f = start().await;
    let (code, _, body) = helpers::http_get_full(&f.base, "/s/abcd").await;
    assert_eq!(code, 300);
    assert!(body.contains(&format!("href=\"/s/{LIVE}\"")), "{body}");
    assert!(body.contains(&format!("href=\"/s/{ASLEEP}\"")), "{body}");

    let (code, _, body) = helpers::http_get_full(&f.base, "/s/%3Cnobody%3E").await;
    assert_eq!(code, 404);
    assert!(
        body.contains("&lt;nobody&gt;"),
        "the ref is escaped: {body}"
    );
}

#[tokio::test]
async fn resolve_answers_the_same_lookup_as_json() {
    let f = start().await;
    let (code, _, body) = helpers::http_get_full(&f.base, &format!("/api/resolve/{PEER}")).await;
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["session"]["uuid"], LIVE);
    assert_eq!(v["session"]["peer"], PEER);

    let (code, _, body) = helpers::http_get_full(&f.base, "/api/resolve/abcd").await;
    assert_eq!(code, 300);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["candidates"].as_array().unwrap().len(), 2);

    let (code, _, _) = helpers::http_get_full(&f.base, "/api/resolve/ffff").await;
    assert_eq!(code, 404);
}

/// Open `/api/events/stream` and wait for its head, so the daemon counts a window.
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
        assert!(n > 0);
        head.push(byte[0]);
    }
    stream
}

#[tokio::test]
async fn a_link_with_a_window_open_focuses_the_session_there() {
    let f = start().await;
    let mut win = open_stream(&f.base).await;
    let (code, _, body) = helpers::http_get_full(&f.base, &format!("/s/{PEER}")).await;
    assert_eq!(code, 200, "{body}");
    assert!(
        body.contains(&format!("/?session={LIVE}")),
        "open-here link: {body}"
    );
    assert!(
        body.contains("window.close()"),
        "the relay tab closes itself: {body}"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    while !acc.contains("\"focus\"") && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(200), win.read(&mut buf)).await
        {
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
    assert!(acc.contains("\"focus\"") && acc.contains(LIVE), "{acc}");
}

#[tokio::test]
async fn pins_persist_across_a_restart() {
    let f = start().await;
    let pin = |uuid: &str| format!("{{\"session\":\"{uuid}\"}}");

    let (code, _) = helpers::http_post(&f.base, "/api/pins", &[JSON], &pin(ASLEEP)).await;
    assert_eq!(code, 200);
    let (code, body) = helpers::http_post(&f.base, "/api/pins", &[JSON], &pin(LIVE)).await;
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let order: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["uuid"].as_str().unwrap())
        .collect();
    assert_eq!(order, [ASLEEP, LIVE], "oldest pin first");

    assert_eq!(
        helpers::http_post(&f.base, "/api/pins", &[JSON], "{\"session\":\"nope\"}")
            .await
            .0,
        400
    );
    assert_eq!(
        helpers::http_post(&f.base, "/api/pins", &[], &pin(LIVE))
            .await
            .0,
        403,
        "a JSON content type is required"
    );

    // A second daemon on the same state dir sees the same pins.
    let proj = tempfile::tempdir().unwrap();
    let sess = tempfile::tempdir().unwrap();
    let again = serve(proj.path(), sess.path(), f.state.path()).await;
    let v: serde_json::Value =
        serde_json::from_str(&helpers::http_get(&again, "/api/pins").await).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);

    assert_eq!(
        helpers::http_delete(&again, &format!("/api/pins/{ASLEEP}")).await,
        200
    );
    assert_eq!(
        helpers::http_delete(&again, &format!("/api/pins/{ASLEEP}")).await,
        404
    );
    let v: serde_json::Value =
        serde_json::from_str(&helpers::http_get(&again, "/api/pins").await).unwrap();
    assert_eq!(v[0]["uuid"], LIVE);
}
