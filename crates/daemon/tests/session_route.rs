//! The session transcript route: GET /session/:uuid renders the semantic HTML, and
//! GET /api/recent reports the most recent session uuid.

use eigen_daemon::{app, Config};

const UUID: &str = "aaaa1111-0000-4000-8000-000000000001";

/// A temp projects dir with one session, and a Config pointing the daemon at it.
fn fixture() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let pdir = dir.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    let lines = [
        format!(r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"cwd":"/home/me/p","timestamp":"2026-06-03T10:00:00Z","sessionId":"{UUID}","message":{{"role":"user","content":"render me in the right pane"}}}}"#),
        format!(r#"{{"type":"last-prompt","leafUuid":"u1","sessionId":"{UUID}"}}"#),
    ];
    std::fs::write(pdir.join(format!("{UUID}.jsonl")), lines.join("\n") + "\n").unwrap();
    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        projects_dir: Some(dir.path().to_path_buf()),
    };
    (dir, cfg)
}

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn session_route_renders_the_transcript_html() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = reqwest_get(&format!("{base}/session/aaaa1111")).await;
    assert!(body.contains("<details"), "collapsible transcript:\n{body}");
    assert!(body.contains("render me in the right pane"), "content present:\n{body}");
}

#[tokio::test]
async fn recent_route_reports_the_latest_uuid() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = reqwest_get(&format!("{base}/api/recent")).await;
    assert!(body.contains(UUID), "recent uuid:\n{body}");
}

/// Minimal HTTP GET without pulling in a client crate: raw request over TCP.
async fn reqwest_get(url: &str) -> String {
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
