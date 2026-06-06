//! session_spark: output_tokens per completed turn (the activity sparkline), and its
//! ~/.eigen/state cache (parse-on-change, keyed by the source's mtime+len).

use eigen_forest::{cached_spark, session_spark};

/// One completed turn: an assistant message carrying usage, closed by a turn_duration row.
fn turn(out: u32) -> String {
    format!(
        "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"output_tokens\":{out}}}}}}}\n\
         {{\"type\":\"system\",\"subtype\":\"turn_duration\"}}\n"
    )
}

#[test]
fn spark_is_output_tokens_per_completed_turn() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.jsonl");
    std::fs::write(&p, format!("{}{}{}", turn(100), turn(250), turn(40))).unwrap();
    assert_eq!(session_spark(&p), vec![100, 250, 40]);
}

#[test]
fn spark_sums_multi_message_turns_before_the_close() {
    // a turn with two assistant messages (tool-use → continuation) sums before turn_duration
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.jsonl");
    let multi = "{\"type\":\"assistant\",\"message\":{\"usage\":{\"output_tokens\":30}}}\n\
                 {\"type\":\"assistant\",\"message\":{\"usage\":{\"output_tokens\":70}}}\n\
                 {\"type\":\"system\",\"subtype\":\"turn_duration\"}\n";
    std::fs::write(&p, multi).unwrap();
    assert_eq!(session_spark(&p), vec![100]);
}

#[test]
fn cached_spark_recomputes_when_the_jsonl_grows() {
    let proj = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let p = proj.path().join("s.jsonl");
    std::fs::write(&p, turn(100)).unwrap();
    assert_eq!(cached_spark(state.path(), "sid", &p), vec![100]);
    assert!(state.path().join("sid.json").exists(), "state file persisted");

    std::fs::write(&p, format!("{}{}", turn(100), turn(5))).unwrap();
    assert_eq!(cached_spark(state.path(), "sid", &p), vec![100, 5]);
}

#[test]
fn cached_spark_serves_the_stored_value_when_source_is_unchanged() {
    let proj = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let p = proj.path().join("s.jsonl");
    std::fs::write(&p, turn(100)).unwrap();
    assert_eq!(cached_spark(state.path(), "sid", &p), vec![100]);

    // Tamper the stored spark but keep its source stamp. An unchanged source must serve
    // the stored value (proving the cache is read, not the JSONL re-parsed).
    let sp = state.path().join("sid.json");
    let mut doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&sp).unwrap()).unwrap();
    doc["spark"] = serde_json::json!([999]);
    std::fs::write(&sp, doc.to_string()).unwrap();

    assert_eq!(cached_spark(state.path(), "sid", &p), vec![999], "served from cache");
}
