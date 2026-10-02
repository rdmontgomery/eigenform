//! Filesystem-watch SSE: per-session change pings and the dev live-reload stream.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use futures_util::StreamExt;
use std::path::PathBuf;

use crate::AppState;

/// `GET /api/watch/:uuid` — Server-Sent Events: a `change` event each time the session's
/// JSONL is written (the live-follow signal for the right pane).
pub(crate) async fn watch_route(
    AxumPath(uuid): AxumPath<String>,
    State(state): State<AppState>,
) -> Response {
    let cfg = &state.config;
    let Some(dir) = &cfg.projects_dir else {
        return (StatusCode::NOT_FOUND, "no projects dir configured").into_response();
    };
    let path = match eigenform_forest::resolve(dir, &uuid) {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "no such session").into_response(),
    };
    let watch_dir = path.parent().unwrap_or(&path).to_path_buf();
    let target = path.file_name().map(|n| n.to_os_string());
    watch_sse(watch_dir, target)
}

/// `GET /` in dev mode: the eigenform index with a live-reload hook injected.
pub(crate) async fn dev_index(State(state): State<AppState>) -> Response {
    let cfg = &state.config;
    let Some(term_dir) = &cfg.term_dir else {
        return (StatusCode::NOT_FOUND, "no term dir").into_response();
    };
    let Ok(html) = std::fs::read_to_string(term_dir.join("index.html")) else {
        return (StatusCode::NOT_FOUND, "no index.html").into_response();
    };
    let injected = html.replacen(
        "<head>",
        "<head>\n    <meta name=\"eigenform-dev\" content=\"1\" />",
        1,
    );
    Html(injected).into_response()
}

/// `GET /api/dev/reload` — SSE that fires whenever the built frontend bundle changes.
pub(crate) async fn dev_reload(State(state): State<AppState>) -> Response {
    let cfg = &state.config;
    let Some(term_dir) = &cfg.term_dir else {
        return (StatusCode::NOT_FOUND, "no term dir").into_response();
    };
    watch_sse(term_dir.join("dist"), None)
}

/// Whether a filesystem event is a real change. notify ≥7's inotify backend also
/// reports *access* (open / close-without-write), so the daemon's own reads of a
/// transcript would otherwise look like writes — and every SSE ping triggers a
/// client refetch, i.e. another read: an unbounded request loop.
pub(crate) fn is_change(event: &notify::Event) -> bool {
    !matches!(event.kind, notify::EventKind::Access(_))
}

/// Spawn a filesystem watcher on `watch_dir`, returning a receiver that yields `()` each time a
/// file (matching `target`, if set) is written, plus the watcher thread's handle.
///
/// The thread exits promptly when the receiver is dropped: it polls with a 1s timeout and checks
/// whether the consumer is gone (`tx.is_closed()`) on every tick. This matters because the old
/// implementation only noticed a disconnected client *after the next filesystem event* — so an
/// EventSource that reconnected while the session's directory was quiet stranded its thread +
/// inotify instance indefinitely. Those leaked watchers accumulated toward the per-user inotify
/// cap (e.g. 128); once near it, `recommended_watcher()` starts failing and new SSE subscriptions
/// stream nothing forever, silently freezing the reach map and transcript.
fn watch_channel(
    watch_dir: PathBuf,
    target: Option<std::ffi::OsString>,
) -> (tokio::sync::mpsc::Receiver<()>, std::thread::JoinHandle<()>) {
    let (tx, rx) = tokio::sync::mpsc::channel::<()>(8);
    let handle = std::thread::spawn(move || {
        let (raw_tx, raw_rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        }) {
            Ok(w) => w,
            Err(_) => return,
        };
        if notify::Watcher::watch(
            &mut watcher,
            &watch_dir,
            notify::RecursiveMode::NonRecursive,
        )
        .is_err()
        {
            return;
        }
        loop {
            match raw_rx.recv_timeout(std::time::Duration::from_secs(1)) {
                // Reads are not changes (see [`is_change`]).
                Ok(Ok(event)) if !is_change(&event) => {}
                Ok(Ok(event)) => {
                    let touches = match &target {
                        Some(name) => event
                            .paths
                            .iter()
                            .any(|p| p.file_name() == Some(name.as_os_str())),
                        None => true,
                    };
                    if touches && tx.blocking_send(()).is_err() {
                        break; // SSE connection gone; drop the watcher
                    }
                }
                // A watcher-level error: ignore this one and keep watching.
                Ok(Err(_)) => {}
                // No fs events this tick — still check whether the client left, so a
                // disconnected consumer is reaped within ~1s instead of leaking until the
                // next (possibly never) filesystem event.
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if tx.is_closed() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (rx, handle)
}

/// SSE that emits a `change` event when files in `watch_dir` are written. If `target` is
/// set, only that filename triggers; otherwise any change in the dir does. Backed by
/// [`watch_channel`], whose thread self-terminates when this stream (and thus the receiver)
/// is dropped — including when the client disconnects while the directory is quiet.
fn watch_sse(watch_dir: PathBuf, target: Option<std::ffi::OsString>) -> Response {
    let (rx, _handle) = watch_channel(watch_dir, target);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|_| {
        Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data("change"))
    });
    // See the forest watch above: keep-alive comments let the daemon notice and reap a
    // disconnected client within the interval instead of leaking the connection until the
    // session's next filesystem write (which, for an idle session, may never arrive).
    axum::response::sse::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(10)),
        )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{Duration, Instant};

    /// Poll `rx` for up to `dur`, returning true if a signal arrives.
    fn recv_within(rx: &mut tokio::sync::mpsc::Receiver<()>, dur: Duration) -> bool {
        let deadline = Instant::now() + dur;
        loop {
            match rx.try_recv() {
                Ok(()) => return true,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Join `handle`, returning true if it finishes within `dur` (false = still running/hung).
    fn join_within(handle: std::thread::JoinHandle<()>, dur: Duration) -> bool {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = handle.join();
            let _ = done_tx.send(());
        });
        done_rx.recv_timeout(dur).is_ok()
    }

    #[test]
    fn watch_channel_emits_on_matching_file_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, b"start\n").unwrap();
        let (mut rx, _h) = watch_channel(dir.path().to_path_buf(), Some("session.jsonl".into()));
        // Let the watcher arm (the SSE response returns before watch() completes).
        std::thread::sleep(Duration::from_millis(300));

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"more\n").unwrap();
        f.flush().unwrap();

        assert!(
            recv_within(&mut rx, Duration::from_secs(2)),
            "appending to the watched file should signal a change",
        );
    }

    #[test]
    fn watch_channel_ignores_non_target_files() {
        let dir = tempfile::tempdir().unwrap();
        let (mut rx, _h) = watch_channel(dir.path().to_path_buf(), Some("session.jsonl".into()));
        std::thread::sleep(Duration::from_millis(300));

        std::fs::write(dir.path().join("other.txt"), b"noise\n").unwrap();

        assert!(
            !recv_within(&mut rx, Duration::from_millis(800)),
            "writes to non-target files must not signal",
        );
    }

    #[test]
    fn watch_channel_ignores_reads_of_the_target() {
        // The daemon reads the transcript to serve /api/session/:uuid/json. If a read
        // signalled a change, each SSE ping would trigger a refetch → another read →
        // another ping: an unbounded request loop. Only writes may signal.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("session.jsonl");
        std::fs::write(&target, b"{}\n").unwrap();
        let (mut rx, _h) = watch_channel(dir.path().to_path_buf(), Some("session.jsonl".into()));
        std::thread::sleep(Duration::from_millis(300));

        for _ in 0..3 {
            let _ = std::fs::read_to_string(&target).unwrap();
        }

        assert!(
            !recv_within(&mut rx, Duration::from_millis(800)),
            "reading the target file must not signal a change",
        );
    }

    #[test]
    fn watch_thread_exits_when_consumer_drops_even_if_dir_is_quiet() {
        // Regression test for the inotify/thread leak: when an SSE client disconnects, the
        // watcher must clean up promptly WITHOUT waiting for a filesystem event. The old
        // implementation blocked on the event channel and only noticed the dead client after
        // the next write, stranding the thread + inotify instance for quiet sessions.
        let dir = tempfile::tempdir().unwrap();
        let (rx, handle) = watch_channel(dir.path().to_path_buf(), Some("session.jsonl".into()));
        std::thread::sleep(Duration::from_millis(200)); // let it arm

        drop(rx); // client gone; directory stays quiet (no writes follow)

        assert!(
            join_within(handle, Duration::from_secs(3)),
            "watcher thread must exit within ~1s of the consumer dropping, with no fs events",
        );
    }
}
