//! GET /api/forest: the corroborated live-Forest snapshot. The test process's own pid is
//! a guaranteed-live process, so writing a session file for it makes the result deterministic.

use eigen_daemon::{app, Config};

const UUID: &str = "bbbb2222-0000-4000-8000-000000000002";

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir, Config) {
    let proj = tempfile::tempdir().unwrap();
    let sess = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let pdir = proj.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    let lines = format!(
        "{{\"type\":\"user\",\"timestamp\":\"2026-06-06T10:00:00Z\",\"sessionId\":\"{UUID}\",\"cwd\":\"/home/me/p\",\"message\":{{\"role\":\"user\"}}}}\n\
         {{\"type\":\"assistant\",\"timestamp\":\"2026-06-06T10:00:01Z\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"output_tokens\":77}}}}}}\n\
         {{\"type\":\"system\",\"subtype\":\"turn_duration\",\"timestamp\":\"2026-06-06T10:00:02Z\"}}\n"
    );
    std::fs::write(pdir.join(format!("{UUID}.jsonl")), lines).unwrap();
    let pid = std::process::id();
    std::fs::write(
        sess.path().join(format!("{pid}.json")),
        format!("{{\"pid\":{pid},\"sessionId\":\"{UUID}\",\"cwd\":\"/home/me/p\"}}"),
    )
    .unwrap();
    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: Some(proj.path().to_path_buf()),
        sessions_dir: Some(sess.path().to_path_buf()),
        state_dir: Some(state.path().to_path_buf()),
        workspace_root: None,
        dev: false,
    };
    (proj, sess, state, cfg)
}

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

async fn get(url: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
}

#[tokio::test]
async fn forest_route_reports_a_live_ready_session_with_spark() {
    let (_p, _s, _st, cfg) = fixture();
    let base = start(cfg).await;
    let body = get(&format!("{base}/api/forest")).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|_| panic!("json:\n{body}"));
    let arr = v.as_array().expect("array");
    let entry = arr
        .iter()
        .find(|e| e["uuid"] == UUID)
        .unwrap_or_else(|| panic!("our session present:\n{body}"));
    assert_eq!(entry["live"], true, "the test process's session is live");
    assert_eq!(entry["state"], "ready", "completed turn → ready");
    assert_eq!(entry["spark"], serde_json::json!([77]), "output_tokens per turn");
}
