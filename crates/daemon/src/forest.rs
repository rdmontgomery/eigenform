//! The live Forest: `GET /api/forest` snapshot and its `GET /api/watch/forest` push stream.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{AppState, Config};

/// `GET /api/forest` — the corroborated live-Forest snapshot (liveness × JSONL state ×
/// activity spark). Mirrors what `eigenform forest --live` prints.
pub(crate) async fn forest_route(State(state): State<AppState>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        forest_json(&state.config),
    )
        .into_response()
}

/// `GET /api/watch/forest` — SSE that pushes the snapshot whenever it changes.
pub(crate) async fn forest_watch_route(State(state): State<AppState>) -> Response {
    forest_sse(state.config)
}

/// Compute the live-Forest snapshot as a JSON string. Empty array if the dirs aren't set.
fn forest_json(cfg: &Config) -> String {
    let (Some(projects), Some(sessions), Some(state)) =
        (&cfg.projects_dir, &cfg.sessions_dir, &cfg.state_dir)
    else {
        return "[]".to_string();
    };
    let rows: Vec<serde_json::Value> =
        eigenform_forest::live_forest(projects, sessions, state, chrono::Utc::now())
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "uuid": s.uuid,
                    "title": s.title,
                    "cwd": s.cwd.display().to_string(),
                    "recency": s.recency.to_rfc3339(),
                    "live": s.live,
                    "state": s.state.as_str(),
                    "spark": s.spark,
                    "headless": s.headless,
                    "pid": s.pid,
                })
            })
            .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

/// Quiet period after a filesystem event before the forest is rescanned (see [`forest_sse`]).
const FOREST_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// SSE pushing the live-Forest snapshot on change. Triggers: filesystem events on the
/// sessions + projects dirs (snappy: activity, new sessions) ∪ a coarse 3s tick (catches
/// pid exits, which aren't filesystem events). Emits only when the snapshot's hash changes,
/// so the tick is silent when nothing moved. The payload travels in the event (no refetch).
///
/// A live session appends to its JSONL many times a second, so filesystem events are
/// coalesced: after one arrives the loop waits [`FOREST_DEBOUNCE`] and drains the rest
/// before rescanning, bounding the full-forest scan to a few per second under load.
fn forest_sse(cfg: Arc<Config>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
    tokio::spawn(async move {
        // A dedicated thread owns the notify watcher; it pings on any write under the
        // watched dirs. Same reaping discipline as [`watch_channel`]: it polls with a 1s
        // timeout and exits once the consumer is gone, so a disconnected client can't
        // strand a watcher (and its inotify instance) until the next filesystem event.
        let (evt_tx, mut evt_rx) = tokio::sync::mpsc::channel::<()>(1);
        let watch_dirs: Vec<PathBuf> = [cfg.sessions_dir.clone(), cfg.projects_dir.clone()]
            .into_iter()
            .flatten()
            .collect();
        std::thread::spawn(move || {
            let (raw_tx, raw_rx) = std::sync::mpsc::channel();
            let Ok(mut watcher) = notify::recommended_watcher(move |res| {
                let _ = raw_tx.send(res);
            }) else {
                return;
            };
            for d in &watch_dirs {
                let _ = notify::Watcher::watch(&mut watcher, d, notify::RecursiveMode::Recursive);
            }
            loop {
                match raw_rx.recv_timeout(std::time::Duration::from_secs(1)) {
                    // A full channel already means "rescan pending" — never block on it.
                    Ok(_) => {
                        if let Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) =
                            evt_tx.try_send(())
                        {
                            break; // SSE gone; drop the watcher
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if evt_tx.is_closed() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        let mut last_hash: u64 = 0;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3));
        loop {
            let cfg2 = Arc::clone(&cfg);
            let json = tokio::task::spawn_blocking(move || forest_json(&cfg2))
                .await
                .unwrap_or_else(|_| "[]".to_string());
            let h = hash_str(&json);
            if h != last_hash {
                last_hash = h;
                if tx.send(json).await.is_err() {
                    break; // client gone
                }
            }
            tokio::select! {
                _ = tick.tick() => {}
                r = evt_rx.recv() => {
                    if r.is_none() { break; }
                    // Coalesce a burst of writes into one rescan.
                    tokio::time::sleep(FOREST_DEBOUNCE).await;
                    while evt_rx.try_recv().is_ok() {}
                }
                _ = tx.closed() => break,
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|json| {
        Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data(json))
    });
    // Keep-alive comments force a periodic write so a disconnected client is detected
    // (the write fails) and the stream + connection are dropped promptly. Without it, a
    // dead SSE connection to a quiet endpoint lingers until the next real event — which
    // may never come — leaking ESTABLISHED sockets against the browser's per-origin cap.
    axum::response::sse::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(10)),
        )
        .into_response()
}
