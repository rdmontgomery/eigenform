//! GET /api/session/:uuid/json attaches a resolved subagent transcript to the Agent
//! tool call that spawned it, discovered from `<uuid>/subagents/agent-<id>.jsonl`.

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

const UUID: &str = "bbbb2222-0000-4000-8000-000000000002";
const AGENT_ID: &str = "ac884004cd3d8238f";

fn fixture() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let pdir = dir.path().join("-home-me-p");
    std::fs::create_dir_all(&pdir).unwrap();

    let parent_lines = [
        format!(r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"cwd":"/home/me/p","timestamp":"2026-06-03T10:00:00Z","sessionId":"{UUID}","message":{{"role":"user","content":"survey branches for PR candidates"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"a1","parentUuid":"u1","isSidechain":false,"sessionId":"{UUID}","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"toolu_agent","name":"Agent","input":{{"description":"Survey branches"}}}}]}}}}"#),
        format!(r#"{{"type":"user","uuid":"r1","parentUuid":"a1","isSidechain":false,"sessionId":"{UUID}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_agent","content":[{{"type":"text","text":"Async agent launched successfully."}}]}}]}},"toolUseResult":{{"isAsync":true,"agentId":"{AGENT_ID}"}}}}"#),
        format!(r#"{{"type":"last-prompt","lastPrompt":"survey branches for PR candidates","leafUuid":"r1","sessionId":"{UUID}"}}"#),
    ];
    std::fs::write(pdir.join(format!("{UUID}.jsonl")), parent_lines.join("\n") + "\n").unwrap();

    let sub_dir = pdir.join(UUID).join("subagents");
    std::fs::create_dir_all(&sub_dir).unwrap();
    let sub_lines = [
        r#"{"type":"user","uuid":"su1","isSidechain":true,"sessionId":"sub","message":{"role":"user","content":"survey the branches"}}"#.to_string(),
        r#"{"type":"assistant","uuid":"sa1","isSidechain":true,"sessionId":"sub","message":{"role":"assistant","content":[{"type":"text","text":"three branches have unopened PRs"}]}}"#.to_string(),
    ];
    std::fs::write(sub_dir.join(format!("agent-{AGENT_ID}.jsonl")), sub_lines.join("\n") + "\n").unwrap();
    std::fs::write(
        sub_dir.join(format!("agent-{AGENT_ID}.meta.json")),
        r#"{"agentType":"general-purpose","description":"Survey branches for PR candidates","toolUseId":"toolu_agent","spawnDepth":1}"#,
    ).unwrap();

    let cfg = Config {
        program: "cat".into(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: Some(dir.path().to_path_buf()),
        sessions_dir: None,
        state_dir: None,
        workspace_root: None,
        dev: false,
        rephrase_cmd: vec!["claude".to_string(), "-p".to_string()],
        log_file: None,
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
async fn session_json_route_attaches_the_resolved_subagent() {
    let (_d, cfg) = fixture();
    let base = start(cfg).await;
    let body = helpers::http_get(&base, "/api/session/bbbb2222/json").await;
    let doc: serde_json::Value = serde_json::from_str(&body).expect("valid json");

    let tool = doc["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["tool"]["kind"] == "Agent")
        .expect("an Agent tool exchange")["tool"]
        .clone();

    assert_eq!(tool["subagent"]["agentType"], "general-purpose");
    assert_eq!(tool["subagent"]["description"], "Survey branches for PR candidates");
    let sub_exchanges = tool["subagent"]["exchanges"].as_array().expect("exchanges array");
    assert_eq!(sub_exchanges[0]["assistant"], "three branches have unopened PRs");
}
