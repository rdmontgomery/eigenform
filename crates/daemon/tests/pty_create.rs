//! /pty?new=<cwd>&create=1 — directory policy for fresh-session spawns.
//!
//! Policy (full-freedom + confirm; the launcher gates `create=1` behind a user
//! "Create <path>?" prompt, and the local-origin check is the CSRF guard):
//!   - create=1 + missing dir → `fs::create_dir_all` it (ANYWHERE), then spawn.
//!   - create=0 + missing dir → refuse: close the socket, do NOT create or spawn.
//!   - existing dir           → spawn as-is, no mkdir, regardless of the flag.
//!
//! Wire format: `&create=1` (non-zero integer = true, absent or `0` = false).
//!
//! Tests use program: "sh" so a spawned pty runs a real shell. Never claude.

#[path = "helpers/mod.rs"]
mod helpers;

use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

use eigenform_daemon::{app, Config};

/// Spawn the daemon with `program: "sh"` and a configured `workspace_root`.
/// (workspace_root still feeds `/api/candidates`; it no longer cages creation.)
/// Returns `(base_url, workspace_root TempDir)`.
async fn start_with_workspace() -> (String, tempfile::TempDir) {
    let workspace = tempfile::tempdir().unwrap();
    let cfg = Config {
        program: "sh".to_string(),
        args: vec![],
        cwd: None,
        web_dir: None,
        term_dir: None,
        projects_dir: None,
        sessions_dir: None,
        state_dir: None,
        workspace_root: Some(workspace.path().to_path_buf()),
        dev: false,
        log_file: None,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    (format!("http://{addr}"), workspace)
}

fn ws_url(base: &str, query: &str) -> String {
    let host = base.strip_prefix("http://").unwrap();
    if query.is_empty() {
        format!("ws://{host}/pty")
    } else {
        format!("ws://{host}/pty?{query}")
    }
}

/// Wait for the first text frame (pty hello) or return None if the socket closes first.
async fn first_text_or_close(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> Option<serde_json::Value> {
    for _ in 0..30 {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                return Some(serde_json::from_str(&t).expect("text frame is JSON"));
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => return None,
            Ok(Some(Ok(_))) => continue,
            _ => return None,
        }
    }
    None
}

/// Drain until a Close frame; return its reason string (empty if none/text-first).
async fn close_reason(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> Option<String> {
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Close(Some(frame))))) => return Some(frame.reason.to_string()),
            Ok(Some(Ok(Message::Close(None)))) | Ok(None) => return Some(String::new()),
            Ok(Some(Ok(Message::Text(_)))) => return None, // pty hello = spawned, not closed
            Ok(Some(Ok(_))) => continue,
            _ => return None,
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Case (a): create=1 + missing under workspace → dir created and pty spawns
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_creates_dir_and_spawns_pty() {
    let (base, workspace) = start_with_workspace().await;

    let new_path = workspace.path().join("fresh-project");
    assert!(!new_path.exists(), "pre: dir must not exist");

    let new_str = new_path.to_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={new_str}&create=1"),
    ))
    .await
    .expect("ws upgrade ok");

    let hello = first_text_or_close(&mut ws).await;
    assert!(
        hello.is_some(),
        "daemon must send a pty hello frame (directory created + spawn succeeded)"
    );
    assert_eq!(hello.unwrap()["type"], "pty", "hello frame type must be 'pty'");
    assert!(
        new_path.exists(),
        "daemon must have created the directory before spawning"
    );

    ws.close(None).await.ok();
}

// ---------------------------------------------------------------------------
// Case (b): create=1 + missing OUTSIDE workspace → now ALLOWED (full freedom).
// The cage is gone: creation is permitted anywhere, gated only by create=1.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_outside_workspace_root_is_allowed() {
    let (base, _workspace) = start_with_workspace().await;

    // A path outside the workspace tempdir, in its own throwaway tempdir.
    let outside_root = tempfile::tempdir().unwrap();
    let outside = outside_root.path().join("made-outside-workspace");
    assert!(!outside.exists(), "pre: dir must not exist");

    let outside_str = outside.to_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={outside_str}&create=1"),
    ))
    .await
    .expect("ws upgrade ok");

    let hello = first_text_or_close(&mut ws).await;
    assert!(
        hello.is_some(),
        "create=1 outside the workspace must now succeed (full-freedom policy)"
    );
    assert!(
        outside.is_dir(),
        "daemon must create the directory outside workspace_root"
    );

    ws.close(None).await.ok();
}

// ---------------------------------------------------------------------------
// Case (b2): `..` in the path is normalized (not a cage to escape) and created.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_normalizes_dotdot_and_creates() {
    let (base, _workspace) = start_with_workspace().await;

    // <tmp>/a/../b normalizes to <tmp>/b.
    let root = tempfile::tempdir().unwrap();
    let dotted = root.path().join("a").join("..").join("eigen-normalized");
    let resolved = root.path().join("eigen-normalized");
    assert!(!resolved.exists(), "pre: normalized target must not exist");

    let dotted_str = dotted.to_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={dotted_str}&create=1"),
    ))
    .await
    .expect("ws upgrade ok");

    let hello = first_text_or_close(&mut ws).await;
    assert!(hello.is_some(), "normalized `..` path must spawn");
    assert!(
        resolved.is_dir(),
        "daemon must create the normalized directory: {resolved:?}"
    );

    ws.close(None).await.ok();
}

// ---------------------------------------------------------------------------
// Case (c): create=0 + missing dir → socket closes "no such directory",
// dir stays missing (we won't spawn claude in a cwd that isn't there).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_create_flag_missing_dir_is_refused() {
    let (base, workspace) = start_with_workspace().await;

    let new_path = workspace.path().join("nonexistent-no-create");
    assert!(!new_path.exists(), "pre: dir must not exist");

    let new_str = new_path.to_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={new_str}"),
    ))
    .await
    .expect("ws upgrade ok");

    let reason = close_reason(&mut ws).await;
    assert!(
        matches!(&reason, Some(r) if r.contains("no such directory")),
        "create=0 + missing dir must close with 'no such directory', got {reason:?}"
    );
    assert!(
        !new_path.exists(),
        "directory must NOT be created when the create flag is absent"
    );
}

// ---------------------------------------------------------------------------
// Case (d): create=1 + EXISTING dir → spawns as-is (create_dir_all is idempotent),
// the dir is untouched. Using the workspace root itself as the (existing) target.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_with_existing_dir_spawns() {
    let (base, workspace) = start_with_workspace().await;

    let root_str = workspace.path().to_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={root_str}&create=1"),
    ))
    .await
    .expect("ws upgrade ok");

    let hello = first_text_or_close(&mut ws).await;
    assert!(
        hello.is_some(),
        "create=1 on an existing dir must spawn (idempotent mkdir)"
    );
    assert!(workspace.path().is_dir(), "the existing dir must remain");

    ws.close(None).await.ok();
}
