//! write: persist a sealed session as <session_id>.jsonl into a projects dir, never
//! overwriting an existing file. Returns the new session id.

use eigen_surgery::{fork_at, write, Session, WriteError};
use tempfile::tempdir;

fn forked() -> Session {
    let text = [
        r#"{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"sessionId":"s-old","message":{"role":"user","content":"hi"}}"#,
        r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","isSidechain":false,"sessionId":"s-old","message":{"role":"assistant","content":[{"type":"text","text":"yo"}]}}"#,
        r#"{"type":"last-prompt","leafUuid":"a1","sessionId":"s-old"}"#,
    ]
    .join("\n")
        + "\n";
    fork_at(&Session::parse_str(&text).unwrap(), "a1").unwrap()
}

#[test]
fn write_creates_named_file_and_returns_the_uuid() {
    let dir = tempdir().unwrap();
    let session = forked();
    let id = write(&session, dir.path()).expect("write");

    assert_eq!(id, session.session_id);
    let path = dir.path().join(format!("{id}.jsonl"));
    assert!(path.exists(), "expected {path:?} to exist");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), session.to_jsonl());
}

#[test]
fn write_refuses_to_clobber_an_existing_session() {
    let dir = tempdir().unwrap();
    let session = forked();
    write(&session, dir.path()).unwrap();

    let err = write(&session, dir.path()).unwrap_err();
    assert!(matches!(err, WriteError::AlreadyExists(_)));
}
