//! `POST /api/pty`: start a fresh claude session with no browser attached.
//!
//! Never runs real claude: every test puts a fake `claude` (an `sh` script that records
//! its argv into its cwd, then sleeps) first on PATH before any daemon starts.

mod helpers;

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

use eigenform_daemon::{app, Config};

const JSON: (&str, &str) = ("content-type", "application/json");

static FAKE_CLAUDE: Once = Once::new();

/// Put the fake `claude` first on PATH. Every test calls this first, so the one PATH
/// write happens before any test thread reads the environment.
fn fake_claude() {
    FAKE_CLAUDE.call_once(|| {
        let bin =
            std::env::temp_dir().join(format!("eigenform-fake-claude-{}", std::process::id()));
        std::fs::create_dir_all(&bin).unwrap();
        let script = bin.join("claude");
        std::fs::write(
            &script,
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > .claude-args\n\
             touch .claude-ran\nexec sleep 60\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
    });
}

async fn serve(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

/// A daemon with a projects dir (so `new` means claude). Returns `(base, projects)`.
async fn start() -> (String, tempfile::TempDir) {
    fake_claude();
    let projects = tempfile::tempdir().unwrap();
    let base = serve(Config {
        program: "sh".into(),
        projects_dir: Some(projects.path().to_path_buf()),
        ..Default::default()
    })
    .await;
    (base, projects)
}

async fn create(base: &str, body: &serde_json::Value) -> (u16, String) {
    helpers::http_post(base, "/api/pty", &[JSON], &body.to_string()).await
}

async fn roster(base: &str) -> Vec<serde_json::Value> {
    serde_json::from_str(&helpers::http_get(base, "/api/pty").await).unwrap()
}

/// The argv the fake claude recorded in `dir`, once it has run.
async fn recorded_args(dir: &Path) -> Vec<String> {
    for _ in 0..100 {
        if dir.join(".claude-ran").exists() {
            let text = std::fs::read_to_string(dir.join(".claude-args")).unwrap();
            return text.lines().map(str::to_string).collect();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("fake claude never ran in {}", dir.display());
}

#[tokio::test]
async fn spawns_claude_headless_with_the_prompt_and_lists_it() {
    let (base, projects) = start().await;
    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().to_str().unwrap().to_string();

    // A prompt that looks like a flag must still arrive as the message.
    let (status, body) = create(
        &base,
        &serde_json::json!({ "new": cwd, "prompt": "--fix the flaky test" }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["cwd"], cwd.as_str());

    assert_eq!(
        recorded_args(work.path()).await,
        vec!["--".to_string(), "--fix the flaky test".to_string()]
    );

    // No browser, yet it's on the roster the rail polls.
    let row = roster(&base)
        .await
        .into_iter()
        .find(|r| r["id"] == id.as_str())
        .expect("headless pty is listed");
    assert_eq!(row["cwd"], cwd.as_str());

    // The fresh-session watcher still adopts the uuid. Keep writing new transcripts in
    // case the first lands before the watcher took its baseline.
    let pdir: PathBuf = projects.path().join(cwd.replace('/', "-"));
    std::fs::create_dir_all(&pdir).unwrap();
    let mut written = Vec::new();
    let mut adopted = None;
    for n in 0..20 {
        let uuid = format!("abcd0000-0000-4000-8000-{n:012}");
        std::fs::write(pdir.join(format!("{uuid}.jsonl")), "{}\n").unwrap();
        written.push(uuid);
        tokio::time::sleep(Duration::from_millis(250)).await;
        adopted = roster(&base)
            .await
            .into_iter()
            .find(|r| r["id"] == id.as_str())
            .and_then(|r| r["uuid"].as_str().map(str::to_string));
        if adopted.is_some() {
            break;
        }
    }
    let adopted = adopted.expect("uuid adopted from the new transcript");
    assert!(written.contains(&adopted));

    // A browser can open it like any other live pty.
    let host = base.strip_prefix("http://").unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{host}/pty?attach={id}"))
        .await
        .unwrap();
    let hello = loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                break serde_json::from_str::<serde_json::Value>(&t).unwrap()
            }
            Ok(Some(Ok(_))) => continue,
            other => panic!("no hello: {other:?}"),
        }
    };
    assert_eq!(hello["type"], "pty");
    assert_eq!(hello["id"], id.as_str());
    ws.close(None).await.ok();

    assert_eq!(
        helpers::http_delete(&base, &format!("/api/pty/{id}")).await,
        204
    );
}

#[tokio::test]
async fn no_prompt_starts_a_bare_session_and_create_makes_the_dir() {
    let (base, _projects) = start().await;
    let root = tempfile::tempdir().unwrap();
    let fresh = root.path().join("brand-new");

    // Missing dir without `create`: refused, nothing made or spawned.
    let before = roster(&base).await.len();
    let (status, _) = create(&base, &serde_json::json!({ "new": fresh })).await;
    assert_eq!(status, 404);
    assert!(!fresh.exists());
    assert_eq!(roster(&base).await.len(), before);

    // With `create`, and a blank prompt: the dir appears and claude gets no message.
    let (status, body) = create(
        &base,
        &serde_json::json!({ "new": fresh, "prompt": "  ", "create": true }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert!(fresh.is_dir());
    assert!(recorded_args(&fresh).await.is_empty());

    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        helpers::http_delete(&base, &format!("/api/pty/{id}")).await,
        204
    );
}

#[tokio::test]
async fn refuses_bad_requests_without_spawning() {
    let (base, _projects) = start().await;
    let work = tempfile::tempdir().unwrap();
    let ok_body = serde_json::json!({ "new": work.path() }).to_string();

    // Not JSON-typed: a cross-origin form post can't trigger a spawn.
    let (status, _) = helpers::http_post(&base, "/api/pty", &[], &ok_body).await;
    assert_eq!(status, 415);
    // A browser page from elsewhere.
    let (status, _) = helpers::http_post(
        &base,
        "/api/pty",
        &[JSON, ("origin", "https://evil.example")],
        &ok_body,
    )
    .await;
    assert_eq!(status, 403);
    // Junk, and a body without `new`.
    let (status, _) = helpers::http_post(&base, "/api/pty", &[JSON], "nope").await;
    assert_eq!(status, 400);
    let (status, _) = create(&base, &serde_json::json!({ "prompt": "hi" })).await;
    assert_eq!(status, 400);

    assert!(roster(&base).await.is_empty());
    assert!(!work.path().join(".claude-args").exists());

    // No projects dir: the daemon can't run claude, and must not hand back a shell.
    let bare = serve(Config {
        program: "sh".into(),
        ..Default::default()
    })
    .await;
    let (status, _) = helpers::http_post(&bare, "/api/pty", &[JSON], &ok_body).await;
    assert_eq!(status, 503);
    assert!(roster(&bare).await.is_empty());
}
