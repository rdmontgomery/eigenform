//! The session host's spawn/attach/detach pump, driven against dummy shells
//! (no claude, no tokens — sh/printf/cat only).

use std::time::Duration;

use eigen_daemon::host::{KillError, Outbound, SessionHost};

/// Drain `Binary` frames from `rx` until the accumulated bytes contain `needle`,
/// or the timeout fires (which fails the test). `Text` frames are ignored — only
/// pty output carries the markers these tests assert on.
async fn drain_until(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Outbound>, needle: &str) {
    let acc = tokio::time::timeout(Duration::from_secs(5), async {
        let mut acc = Vec::new();
        while let Some(frame) = rx.recv().await {
            if let Outbound::Binary(bytes) = frame {
                acc.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&acc).contains(needle) {
                    return acc;
                }
            }
        }
        acc
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"));
    assert!(
        String::from_utf8_lossy(&acc).contains(needle),
        "stream ended before {needle:?}; got: {:?}",
        String::from_utf8_lossy(&acc)
    );
}

#[tokio::test]
async fn attach_after_output_repaints_then_streams() {
    let host = SessionHost::default();
    let pty = host
        .spawn("sh", &["-c", "printf 'EARLY\\n'; sleep 30"], None, (80, 24))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await; // let EARLY land in the model

    let (snapshot, _rx) = pty.attach();
    assert!(
        String::from_utf8_lossy(&snapshot).contains("EARLY"),
        "snapshot must repaint output that arrived before attach"
    );

    pty.write_input(b"").unwrap(); // attach is live: the writer works
    let id = pty.id;
    let (_, rx) = pty.attach();
    drop(rx); // detach
    assert!(host.get(id).is_some(), "detach must not kill the pty");
}

#[tokio::test]
async fn two_viewers_both_receive_output() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &[], None, (80, 24)).unwrap();
    let (_, mut rx1) = pty.attach();
    let (_, mut rx2) = pty.attach();
    pty.write_input(b"printf BOTHSEE\n").unwrap();
    // Both the echoed input and sh's output are Binary frames; the marker lands in each.
    drain_until(&mut rx1, "BOTHSEE").await;
    drain_until(&mut rx2, "BOTHSEE").await;
}

#[tokio::test]
async fn output_while_detached_lands_in_the_next_snapshot() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &[], None, (80, 24)).unwrap();
    pty.write_input(b"printf WHILE_AWAY\n").unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (snapshot, _rx) = pty.attach(); // first-ever attach: must still contain it
    assert!(
        String::from_utf8_lossy(&snapshot).contains("WHILE_AWAY"),
        "output produced before any attach must be in the snapshot"
    );
}

// ── Task 1.5: lifecycle — reap on EOF, explicit kill, GC sweep ──────────────

#[tokio::test]
async fn child_exit_marks_exited_but_keeps_the_entry_briefly() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "true"], None, (80, 24)).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pty.exited_at().is_some(), "pump must mark exited on EOF");
    assert!(host.get(pty.id).is_some(), "kept for a final view");
}

#[tokio::test]
async fn sweep_reaps_long_dead_entries() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "true"], None, (80, 24)).unwrap();
    let id = pty.id;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pty.exited_at().is_some(), "must have exited before sweep");
    // ZERO max-age: any exited entry is "long dead", so it is reaped.
    host.sweep(Duration::ZERO);
    assert!(host.get(id).is_none(), "sweep must reap the long-dead entry");
}

#[tokio::test]
async fn sweep_keeps_live_and_recently_exited_entries() {
    let host = SessionHost::default();
    // A live child: never exited, must survive any sweep.
    let live = host.spawn("sh", &["-c", "sleep 30"], None, (80, 24)).unwrap();
    // A just-exited child: within a generous max-age, must survive.
    let recent = host.spawn("sh", &["-c", "true"], None, (80, 24)).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(recent.exited_at().is_some());

    host.sweep(Duration::from_secs(600));
    assert!(host.get(live.id).is_some(), "live entry must survive sweep");
    assert!(
        host.get(recent.id).is_some(),
        "recently-exited entry must survive sweep"
    );

    host.kill(live.id).unwrap(); // stray-process hygiene: don't leak the sleeper
}

#[tokio::test]
async fn kill_terminates_the_child_and_removes_the_entry() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "sleep 30"], None, (80, 24)).unwrap();
    let id = pty.id;
    host.kill(id).unwrap();
    assert!(host.get(id).is_none(), "kill must remove the entry");
}

#[tokio::test]
async fn kill_of_unknown_id_is_not_found() {
    let host = SessionHost::default();
    // The 404/204 distinction Task 1.7 needs: an unknown id reports NotFound.
    assert!(matches!(host.kill(9999), Err(KillError::NotFound)));
}

#[tokio::test]
async fn attached_subscriber_receives_the_exit_text_frame() {
    let host = SessionHost::default();
    let pty = host.spawn("sh", &["-c", "true"], None, (80, 24)).unwrap();
    let (_, mut rx) = pty.attach();
    // The pump broadcasts an exit Text frame when the child dies.
    let got = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = rx.recv().await {
            if let Outbound::Text(msg) = frame {
                if msg.contains(r#"{"type":"exit"}"#) {
                    return true;
                }
            }
        }
        false
    })
    .await
    .expect("timed out waiting for exit frame");
    assert!(got, "attached subscriber must receive the exit Text frame");
}
