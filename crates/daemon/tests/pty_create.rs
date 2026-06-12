//! /pty?new=<cwd>&create=1 — mkdir-under-workspace gate tests.
//!
//! The `create=1` flag tells the daemon to `fs::create_dir_all` the requested
//! cwd BEFORE spawning the pty. This is only allowed when the new path is under
//! the configured `workspace_root`; anything outside is rejected via the POLICY
//! close-frame pattern (same as attach-miss). Traversal via `..` is also rejected.
//!
//! Wire format: `&create=1` (non-zero integer = true, absent or `0` = false).
//!
//! Three pinned cases:
//!   (a) ?new=<root>/fresh&create=1 → dir created, pty spawns.
//!   (b) ?new=<outside>&create=1    → socket closes (POLICY), dir NOT created.
//!   (c) ?new=<missing>             → current no-create behavior pinned (dir stays missing).
//!
//! Tests use program: "sh" so a spawned pty runs a real shell. Never claude.

#[path = "helpers/mod.rs"]
mod helpers;

use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

use eigen_daemon::{app, Config};

/// Spawn the daemon with `program: "sh"` and a configured `workspace_root`.
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

// ---------------------------------------------------------------------------
// Case (a): ?new=<root>/fresh&create=1 → dir created and pty spawns
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_creates_dir_and_spawns_pty() {
    let (base, workspace) = start_with_workspace().await;

    let new_path = workspace.path().join("fresh-project");
    // The directory must NOT exist before the call.
    assert!(!new_path.exists(), "pre: dir must not exist");

    let new_str = new_path.to_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={new_str}&create=1"),
    ))
    .await
    .expect("ws upgrade ok");

    // Expect the pty hello frame — the spawn succeeded.
    let hello = first_text_or_close(&mut ws).await;
    assert!(
        hello.is_some(),
        "daemon must send a pty hello frame (directory created + spawn succeeded)"
    );
    let hello = hello.unwrap();
    assert_eq!(hello["type"], "pty", "hello frame type must be 'pty'");

    // The directory must now exist.
    assert!(
        new_path.exists(),
        "daemon must have created the directory before spawning"
    );

    ws.close(None).await.ok();
}

// ---------------------------------------------------------------------------
// Case (b): ?new=<outside>&create=1 → POLICY close, dir NOT created
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_outside_workspace_root_is_rejected() {
    let (base, _workspace) = start_with_workspace().await;

    // Use a path that cannot be under the workspace tempdir.
    let outside = "/tmp/evil-eigen-test-dir";
    // Clean up any pre-existing path from a prior run.
    let _ = std::fs::remove_dir_all(outside);

    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={outside}&create=1"),
    ))
    .await
    .expect("ws upgrade ok even for an outside path");

    // Expect the socket to close with a POLICY reason — not a pty hello.
    let mut got_policy_close = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Close(Some(frame))))) => {
                got_policy_close = frame.reason.contains("workspace root");
                break;
            }
            Ok(Some(Ok(Message::Close(None)))) | Ok(None) => {
                // Plain close without a reason frame: also a rejection.
                got_policy_close = true;
                break;
            }
            Ok(Some(Ok(Message::Text(_)))) => {
                // If we got a pty hello, the spawn happened — fail the test.
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(
        got_policy_close,
        "create outside workspace_root must close the socket with a POLICY reason"
    );

    // The directory must NOT have been created.
    assert!(
        !std::path::Path::new(outside).exists(),
        "daemon must NOT create a directory outside workspace_root"
    );
}

// ---------------------------------------------------------------------------
// Case (b2): `..` traversal — ?new=<root>/../evil&create=1 → rejected
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_dotdot_traversal_is_rejected() {
    let (base, workspace) = start_with_workspace().await;

    // Build a path that uses `..` to escape the workspace root.
    // e.g. if workspace is /tmp/abc123, this is /tmp/abc123/../evil-traverse.
    let traversal = workspace
        .path()
        .join("..")
        .join("eigen-evil-traverse-test");
    let traversal_str = traversal.to_str().unwrap();
    let _ = std::fs::remove_dir_all(traversal_str);

    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={traversal_str}&create=1"),
    ))
    .await
    .expect("ws upgrade ok");

    let mut got_close = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {
                got_close = true;
                break;
            }
            Ok(Some(Ok(Message::Text(_)))) => {
                // Got pty hello — traversal was allowed. Test fails.
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(
        got_close,
        "create with .. traversal must close the socket (rejected)"
    );

    // The traversal target must NOT have been created.
    // The `..` resolves: workspace.parent() / "eigen-evil-traverse-test".
    let parent = workspace.path().parent().unwrap();
    let actual = parent.join("eigen-evil-traverse-test");
    assert!(
        !actual.exists(),
        "daemon must NOT create a directory via .. traversal: {actual:?}"
    );
}

// ---------------------------------------------------------------------------
// Case (c): no create flag + missing dir → current behavior pinned
//
// When `create` is absent and the cwd doesn't exist, the spawn may fail
// (portable-pty returns an error) or succeed depending on the OS. Either
// way the directory must NOT have been created by the daemon.
// We only pin that the directory stays missing — not the spawn outcome.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_create_flag_missing_dir_stays_missing() {
    let (base, workspace) = start_with_workspace().await;

    let new_path = workspace.path().join("nonexistent-no-create");
    assert!(!new_path.exists(), "pre: dir must not exist");

    let new_str = new_path.to_str().unwrap();
    // Connect WITHOUT &create=1; consume any frame (pty hello or close).
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={new_str}"),
    ))
    .await
    .expect("ws upgrade ok");

    // Drain a few frames (whatever the daemon sends — we don't prescribe outcome).
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_millis(500), ws.next()).await {
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    ws.close(None).await.ok();

    // The directory must remain missing — no create without the flag.
    assert!(
        !new_path.exists(),
        "directory must NOT be created when create flag is absent"
    );
}

// ---------------------------------------------------------------------------
// Case (d): ?new=<workspace_root>&create=1 → rejected (root itself is the target)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_flag_with_workspace_root_as_target_is_rejected() {
    let (base, workspace) = start_with_workspace().await;

    // new= is the workspace root itself — within containment, but equal to root.
    let root_str = workspace.path().to_str().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url(
        &base,
        &format!("new={root_str}&create=1"),
    ))
    .await
    .expect("ws upgrade ok");

    // Expect a POLICY close — not a pty hello.
    let mut got_policy_close = false;
    for _ in 0..20 {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Close(Some(frame))))) => {
                got_policy_close = frame.reason.contains("workspace root");
                break;
            }
            Ok(Some(Ok(Message::Close(None)))) | Ok(None) => {
                got_policy_close = true;
                break;
            }
            Ok(Some(Ok(Message::Text(_)))) => {
                // pty hello received — spawn happened. Test fails.
                break;
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(
        got_policy_close,
        "create with new=<workspace_root> must close the socket with a POLICY reason"
    );
}
