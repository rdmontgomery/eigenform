//! The artifact pane's routes: list what a session wrote, serve it sandboxed, and
//! refuse anything outside that scope.

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

const UUID: &str = "dddd4444-0000-4000-8000-000000000004";

fn fixture() -> (tempfile::TempDir, Config, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().canonicalize().unwrap().join("work");
    let out = work.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join("report.html"),
        "<h1>report</h1><link rel=stylesheet href=style.css>",
    )
    .unwrap();
    std::fs::write(out.join("style.css"), "h1{color:red}").unwrap();
    std::fs::write(work.join("plan.md"), "# The plan\n\n1. ship\n").unwrap();
    std::fs::write(work.join("secret.txt"), "hunter2").unwrap();

    let projects = dir.path().join("projects");
    let pdir = projects.join("-work");
    std::fs::create_dir_all(&pdir).unwrap();
    let html = out.join("report.html");
    let md = work.join("plan.md");
    let lines = [
        format!(
            r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"cwd":"{w}","timestamp":"2026-10-02T09:00:00Z","sessionId":"{UUID}","message":{{"role":"user","content":"write a report and a plan"}}}}"#,
            w = work.display()
        ),
        format!(
            r#"{{"type":"assistant","uuid":"a1","parentUuid":"u1","isSidechain":false,"sessionId":"{UUID}","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"Write","input":{{"file_path":"{p}","content":"..."}}}},{{"type":"tool_use","id":"t2","name":"Write","input":{{"file_path":"{m}","content":"..."}}}}]}}}}"#,
            p = html.display(),
            m = md.display()
        ),
    ];
    std::fs::write(pdir.join(format!("{UUID}.jsonl")), lines.join("\n") + "\n").unwrap();

    let cfg = Config {
        program: "cat".into(),
        projects_dir: Some(projects),
        ..Default::default()
    };
    (dir, cfg, work)
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
async fn lists_artifacts_and_serves_them_sandboxed() {
    let (_d, cfg, work) = fixture();
    let base = start(cfg).await;

    let list: serde_json::Value = serde_json::from_str(
        &helpers::http_get(&base, &format!("/api/session/{UUID}/artifacts")).await,
    )
    .unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    // Newest write first: plan.md was written after report.html in the same turn.
    assert_eq!(list[0]["kind"], "markdown");
    assert_eq!(list[1]["kind"], "html");
    assert!(list[1]["mtime"].is_string());
    let html_url = list[1]["url"].as_str().unwrap().to_string();
    assert!(html_url.starts_with(&format!("/artifact/{UUID}/")));

    // The HTML itself: served with a sandbox CSP and no same-origin escape hatch.
    let (status, head, body) = helpers::http_get_full(&base, &html_url).await;
    assert_eq!(status, 200);
    assert!(head.contains("content-type: text/html"));
    assert!(head.contains("content-security-policy: sandbox allow-scripts"));
    assert!(!head.contains("allow-same-origin"));
    assert!(head.contains("x-content-type-options: nosniff"));
    assert!(body.contains("<h1>report</h1>"));

    // A relative asset beside it resolves.
    let css_url = html_url.replace("report.html", "style.css");
    let (status, head, _) = helpers::http_get_full(&base, &css_url).await;
    assert_eq!(status, 200);
    assert!(head.contains("content-type: text/css"));

    // Markdown renders to a script-free page; ?raw=1 gives the source.
    let md_url = list[0]["url"].as_str().unwrap();
    let (status, head, body) = helpers::http_get_full(&base, md_url).await;
    assert_eq!(status, 200);
    assert!(body.contains("<h1>The plan</h1>"));
    assert!(head.contains("script-src 'none'"));
    let (_, _, raw) = helpers::http_get_full(&base, &format!("{md_url}?raw=1")).await;
    assert!(raw.starts_with("# The plan"));

    // Out of scope: a file the session never wrote, outside any artifact's directory.
    let secret = format!("/artifact/{UUID}{}", work.join("secret.txt").display());
    let (status, _, body) = helpers::http_get_full(&base, &secret).await;
    assert_eq!(status, 403);
    assert!(!body.contains("hunter2"));
    let (status, _, _) =
        helpers::http_get_full(&base, &format!("/artifact/{UUID}/etc/passwd")).await;
    assert_eq!(status, 403);
    // Traversal out of the artifact dir is refused, not normalized into scope.
    let (status, _, body) = helpers::http_get_full(
        &base,
        &html_url.replace("out/report.html", "out/../secret.txt"),
    )
    .await;
    assert_ne!(status, 200);
    assert!(!body.contains("hunter2"));
}

#[tokio::test]
async fn unknown_session_is_404() {
    let (_d, cfg, _) = fixture();
    let base = start(cfg).await;
    let (status, _, _) = helpers::http_get_full(&base, "/api/session/ffff/artifacts").await;
    assert_eq!(status, 404);
    let (status, _, _) = helpers::http_get_full(&base, "/artifact/ffff/etc/passwd").await;
    assert_eq!(status, 404);
}
