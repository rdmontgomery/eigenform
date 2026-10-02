//! Codex threads join the forest, render in the drawer, and nest under the parent
//! Claude session's Bash call that spawned them (`codex-thread: <id>` marker).

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

const PARENT: &str = "cccc3333-0000-4000-8000-000000000003";
const THREAD: &str = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

fn fixture() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();

    // A Claude parent whose Bash call spawned a Codex worker.
    let projects = dir.path().join("projects");
    let pdir = projects.join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();
    let parent = [
        format!(
            r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"cwd":"/home/me/p","timestamp":"2026-10-02T09:00:00Z","sessionId":"{PARENT}","message":{{"role":"user","content":"get a second opinion from codex"}}}}"#
        ),
        format!(
            r#"{{"type":"assistant","uuid":"a1","parentUuid":"u1","isSidechain":false,"sessionId":"{PARENT}","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"toolu_bash","name":"Bash","input":{{"command":"codex-worker spawn retry-fix -- 'fix the flaky retry test'"}}}}]}}}}"#
        ),
        format!(
            r#"{{"type":"user","uuid":"r1","parentUuid":"a1","isSidechain":false,"sessionId":"{PARENT}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_bash","content":"worker: retry-fix\ncodex-thread: {THREAD}\nworktree: /w/repo-wt"}}]}}}}"#
        ),
    ];
    std::fs::write(
        pdir.join(format!("{PARENT}.jsonl")),
        parent.join("\n") + "\n",
    )
    .unwrap();

    // The Codex worker's rollout.
    let codex = dir.path().join("codex");
    let day = codex.join("sessions/2026/10/02");
    std::fs::create_dir_all(&day).unwrap();
    let rollout = [
        format!(r#"{{"timestamp":"2026-10-02T09:00:05.000Z","type":"session_meta","payload":{{"id":"{THREAD}","timestamp":"2026-10-02T09:00:05.000Z","cwd":"/w/repo-wt","originator":"codex_exec","cli_version":"0.130.0","source":"exec"}}}}"#),
        r#"{"timestamp":"2026-10-02T09:00:05.100Z","type":"event_msg","payload":{"type":"user_message","message":"fix the flaky retry test"}}"#.to_string(),
        r#"{"timestamp":"2026-10-02T09:00:09.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"fixed: backoff instead of a fixed sleep"}]}}"#.to_string(),
    ];
    std::fs::write(
        day.join(format!("rollout-2026-10-02T09-00-05-{THREAD}.jsonl")),
        rollout.join("\n") + "\n",
    )
    .unwrap();

    let cfg = Config {
        program: "cat".into(),
        projects_dir: Some(projects),
        sessions_dir: Some(dir.path().join("sessions")),
        state_dir: Some(dir.path().join("state")),
        codex_home: Some(codex),
        ..Default::default()
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
async fn forest_lists_the_codex_thread_as_a_headless_codex_row() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let rows: serde_json::Value =
        serde_json::from_str(&helpers::http_get(&base, "/api/forest").await).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["engine"] == "codex")
        .expect("a codex row");
    assert_eq!(row["uuid"], THREAD);
    assert_eq!(row["title"], "fix the flaky retry test");
    assert_eq!(row["cwd"], "/w/repo-wt");
    assert_eq!(row["headless"], true);
    assert_eq!(row["live"], false);
    assert_eq!(row["spark"].as_array().unwrap().len(), 1, "one user turn");
}

#[tokio::test]
async fn session_route_renders_a_codex_thread() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let doc: serde_json::Value =
        serde_json::from_str(&helpers::http_get(&base, "/api/session/0199a1b2/json").await)
            .unwrap();
    assert_eq!(doc["engine"], "codex");
    assert_eq!(
        doc["exchanges"][0]["assistant"],
        "fixed: backoff instead of a fixed sleep"
    );
}

#[tokio::test]
async fn parent_bash_call_nests_the_codex_worker_transcript() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let doc: serde_json::Value =
        serde_json::from_str(&helpers::http_get(&base, "/api/session/cccc3333/json").await)
            .unwrap();
    let tool = doc["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["tool"]["kind"] == "Bash")
        .expect("the Bash exchange")["tool"]
        .clone();
    assert_eq!(tool["subagent"]["agentType"], "codex");
    assert_eq!(tool["subagent"]["threadId"], THREAD);
    assert_eq!(tool["subagent"]["description"], "fix the flaky retry test");
    assert_eq!(
        tool["subagent"]["exchanges"][0]["assistant"],
        "fixed: backoff instead of a fixed sleep"
    );
}
