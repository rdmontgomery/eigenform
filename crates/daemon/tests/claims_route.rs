//! GET/DELETE /api/claims: the audit of Claude's `sessions/<pid>.json` claims. The test
//! process's own pid is a guaranteed-live process; a huge pid is a guaranteed-dead one.

use eigenform_daemon::{app, Config};

const LIVE: &str = "cccc3333-0000-4000-8000-000000000003";
const DEAD: &str = "dddd4444-0000-4000-8000-000000000004";
const DEAD_PID: u32 = 999_999_999;

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Config) {
    let proj = tempfile::tempdir().unwrap();
    let sess = tempfile::tempdir().unwrap();
    let pdir = proj.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(
        pdir.join(format!("{LIVE}.jsonl")),
        "{\"type\":\"ai-title\",\"aiTitle\":\"nightly digest\",\"entrypoint\":\"sdk-cli\"}\n",
    )
    .unwrap();
    let pid = std::process::id();
    std::fs::write(
        sess.path().join(format!("{pid}.json")),
        format!("{{\"pid\":{pid},\"sessionId\":\"{LIVE}\",\"cwd\":\"/home/me/p\",\"startedAt\":2}}"),
    )
    .unwrap();
    std::fs::write(
        sess.path().join(format!("{DEAD_PID}.json")),
        format!("{{\"pid\":{DEAD_PID},\"sessionId\":\"{DEAD}\",\"cwd\":\"/home/me/p\",\"startedAt\":1,\"entrypoint\":\"cli\"}}"),
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
        state_dir: None,
        workspace_root: None,
        dev: false,
        log_file: None,
    };
    (proj, sess, cfg)
}

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

async fn request(method: &str, url: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn claims_are_listed_with_health_title_and_headless() {
    let (_p, _s, cfg) = fixture();
    let base = start(cfg).await;
    let (status, body) = request("GET", &format!("{base}/api/claims")).await;
    assert_eq!(status, 200);
    let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.len(), 2);
    let live = rows.iter().find(|r| r["sessionId"] == LIVE).unwrap();
    assert_eq!(live["health"], "alive");
    assert_eq!(live["title"], "nightly digest");
    assert_eq!(live["headless"], true, "entrypoint read from the transcript");
    let dead = rows.iter().find(|r| r["sessionId"] == DEAD).unwrap();
    assert_eq!(dead["health"], "dead");
    assert_eq!(dead["headless"], false);
}

#[tokio::test]
async fn deleting_a_dead_claim_removes_its_file() {
    let (_p, sess, cfg) = fixture();
    let base = start(cfg).await;
    let (status, body) = request("DELETE", &format!("{base}/api/claims/{DEAD_PID}")).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("cleared"));
    assert!(!sess.path().join(format!("{DEAD_PID}.json")).exists());
}

#[tokio::test]
async fn the_daemon_never_signals_itself_or_an_unclaimed_pid() {
    let (_p, sess, cfg) = fixture();
    let base = start(cfg).await;
    let me = std::process::id();
    let (status, _) = request("DELETE", &format!("{base}/api/claims/{me}")).await;
    assert_eq!(status, 400);
    assert!(sess.path().join(format!("{me}.json")).exists());
    let (status, _) = request("DELETE", &format!("{base}/api/claims/12345678")).await;
    assert_eq!(status, 404);
}
