//! enumerate_subagents: discover the async subagent transcripts a session spawned via the
//! Agent tool. They live at `<session_path stem>/subagents/agent-<id>.jsonl`, one level
//! below the parent session file, with a sibling `agent-<id>.meta.json` carrying
//! `{agentType, description, toolUseId, spawnDepth}`.

use eigenform_forest::enumerate_subagents;

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("-proj")).unwrap();
    std::fs::write(root.join("-proj/session-uuid.jsonl"), "{}\n").unwrap();
    dir
}

fn write_subagent(root: &std::path::Path, agent_id: &str, meta: Option<&str>) {
    let dir = root.join("-proj/session-uuid/subagents");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("agent-{agent_id}.jsonl")), "{}\n").unwrap();
    if let Some(meta) = meta {
        std::fs::write(dir.join(format!("agent-{agent_id}.meta.json")), meta).unwrap();
    }
}

#[test]
fn no_subagents_dir_yields_empty() {
    let dir = fixture();
    let session_path = dir.path().join("-proj/session-uuid.jsonl");
    assert!(enumerate_subagents(&session_path).is_empty());
}

#[test]
fn discovers_a_subagent_with_its_meta() {
    let dir = fixture();
    write_subagent(
        dir.path(),
        "ac884004cd3d8238f",
        Some(r#"{"agentType":"general-purpose","description":"Survey branches for PR candidates","toolUseId":"toolu_01","spawnDepth":1}"#),
    );
    let session_path = dir.path().join("-proj/session-uuid.jsonl");

    let subs = enumerate_subagents(&session_path);
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].agent_id, "ac884004cd3d8238f");
    assert!(subs[0].path.ends_with("subagents/agent-ac884004cd3d8238f.jsonl"));
    assert_eq!(subs[0].agent_type.as_deref(), Some("general-purpose"));
    assert_eq!(subs[0].description.as_deref(), Some("Survey branches for PR candidates"));
}

#[test]
fn discovers_multiple_subagents() {
    let dir = fixture();
    write_subagent(dir.path(), "one", Some(r#"{"agentType":"claude","description":"first"}"#));
    write_subagent(dir.path(), "two", Some(r#"{"agentType":"claude","description":"second"}"#));
    let session_path = dir.path().join("-proj/session-uuid.jsonl");

    let mut ids: Vec<String> = enumerate_subagents(&session_path).into_iter().map(|s| s.agent_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["one", "two"]);
}

#[test]
fn missing_meta_file_still_yields_a_stub() {
    // The jsonl is the source of truth for "a subagent transcript exists"; a missing or
    // malformed meta file degrades to None fields rather than hiding the transcript.
    let dir = fixture();
    write_subagent(dir.path(), "no-meta", None);
    let session_path = dir.path().join("-proj/session-uuid.jsonl");

    let subs = enumerate_subagents(&session_path);
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].agent_id, "no-meta");
    assert!(subs[0].agent_type.is_none());
    assert!(subs[0].description.is_none());
}

#[test]
fn malformed_meta_file_still_yields_a_stub() {
    let dir = fixture();
    write_subagent(dir.path(), "bad-meta", Some("not json"));
    let session_path = dir.path().join("-proj/session-uuid.jsonl");

    let subs = enumerate_subagents(&session_path);
    assert_eq!(subs.len(), 1);
    assert!(subs[0].agent_type.is_none());
}
