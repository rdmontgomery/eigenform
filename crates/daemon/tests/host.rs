//! The session host's spawn/attach/detach pump, driven against dummy shells
//! (no claude, no tokens — sh/printf/cat only).

use std::time::Duration;

use eigen_daemon::host::{Outbound, SessionHost};

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
        .spawn("sh", &["-c", "printf 'EARLY\\n'; sleep 30"], None, (24, 80))
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
    let pty = host.spawn("sh", &[], None, (24, 80)).unwrap();
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
    let pty = host.spawn("sh", &[], None, (24, 80)).unwrap();
    pty.write_input(b"printf WHILE_AWAY\n").unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (snapshot, _rx) = pty.attach(); // first-ever attach: must still contain it
    assert!(
        String::from_utf8_lossy(&snapshot).contains("WHILE_AWAY"),
        "output produced before any attach must be in the snapshot"
    );
}
