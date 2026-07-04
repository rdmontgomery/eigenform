//! GET /api/inspect: the unified config inventory (skills + memory across
//! resolution layers, token-budgeted) as JSON. `projects_dir` is `<home>/.claude/
//! projects`, so the route can recover `home` and walk the skill stack.

use std::path::Path;

use eigenform_daemon::{app, Config};

/// Write a SKILL.md under `<dir>/.claude/skills/<name>/SKILL.md`.
fn write_skill(dir: &Path, name: &str, desc: &str) {
    let d = dir.join(".claude/skills").join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {desc}\n---\nbody\n"),
    )
    .unwrap();
}

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Config) {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    // Global skill + a repo skill that shadows a same-named global one.
    write_skill(home.path(), "brainstorm", "global brainstorm");
    write_skill(home.path(), "review", "global review");
    write_skill(cwd.path(), "review", "repo review");

    // A recorded project (so all-projects discovers it) with one memory entry.
    let esc: String = cwd.path().to_string_lossy().replace('/', "-");
    let proj = home.path().join(".claude/projects").join(&esc);
    std::fs::create_dir_all(proj.join("memory")).unwrap();
    std::fs::write(
        proj.join("sess.jsonl"),
        format!("{{\"cwd\":\"{}\"}}\n", cwd.path().display()),
    )
    .unwrap();
    std::fs::write(
        proj.join("memory/auth.md"),
        "---\nname: auth\ndescription: oauth notes\ntype: project\n---\nbody\n",
    )
    .unwrap();

    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: Some(home.path().join(".claude/projects")),
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
        rephrase_cmd: vec!["claude".to_string(), "-p".to_string()],
        log_file: None,
    };
    (home, cwd, cfg)
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
async fn inspect_all_projects_lists_layers_with_tokens_and_memory() {
    let (_home, _cwd, cfg) = fixture();
    let base = start(cfg).await;
    let body = get(&format!("{base}/api/inspect")).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|_| panic!("json:\n{body}"));

    assert!(v["tokens"].as_u64().unwrap() > 0, "total token estimate present");
    let labels: Vec<&str> = v["layers"].as_array().unwrap().iter().filter_map(|l| l["label"].as_str()).collect();
    assert!(labels.contains(&"global"), "global layer present: {labels:?}");
    // The repo layer is named after the discovered project dir.
    assert!(labels.iter().any(|l| l.starts_with("repo")), "a repo layer present: {labels:?}");

    // Memory attached to the repo layer.
    let mem_present = v["layers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["memory"].as_array().map(|m| !m.is_empty()).unwrap_or(false));
    assert!(mem_present, "project memory surfaced:\n{body}");
}

#[tokio::test]
async fn inspect_single_context_computes_shadowing() {
    let (_home, cwd, cfg) = fixture();
    let base = start(cfg).await;
    let body = get(&format!("{base}/api/inspect?cwd={}", cwd.path().display())).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|_| panic!("json:\n{body}"));

    // The global `review` is shadowed by the repo `review`.
    let global = v["layers"].as_array().unwrap().iter().find(|l| l["label"] == "global").unwrap();
    let g_review = global["skills"].as_array().unwrap().iter().find(|s| s["name"] == "review").unwrap();
    assert_eq!(g_review["wins"], false, "global review is shadowed");

    let repo = v["layers"].as_array().unwrap().iter().find(|l| l["label"] == "repo").unwrap();
    let r_review = repo["skills"].as_array().unwrap().iter().find(|s| s["name"] == "review").unwrap();
    assert_eq!(r_review["wins"], true, "repo review wins");
}
