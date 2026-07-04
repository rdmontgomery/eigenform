//! A tiny structured event bus for daemon observability.
//!
//! The daemon has no logging otherwise: when a fork's staged prompt never lands,
//! a resume refuses, or a pty dies, there is nothing to look at. This bus records
//! structured events into a bounded ring buffer, fans them out to SSE subscribers,
//! and (optionally) appends each as one JSON line to a `--log-file`.
//!
//! # Design constraints
//! - `kind` is a plain `String` (kebab-case by convention), deliberately open-ended
//!   so future work can emit new kinds (e.g. `downgrade-detected`, `rephrase-fallback`)
//!   without touching this module or the wire schema.
//! - `data` is arbitrary JSON — usually carrying a `ptyId` and/or session `uuid`.
//! - `record` is called from many threads (the pty pump, filesystem-watcher threads,
//!   route handlers), so the hot path is a `std::Mutex<VecDeque>` plus a non-blocking
//!   broadcast send. Cheap and lock-safe; no async required to record.
//! - Logging to disk is strictly best-effort: a write failure warns once to stderr
//!   and is thereafter silent. It must never crash or block the daemon.

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tokio::sync::broadcast;

/// How many recent events the ring buffer retains. Old events past this cap are
/// dropped oldest-first; `GET /api/events` serves what remains (oldest → newest).
pub const RING_CAP: usize = 500;

/// Fan-out channel depth. A slow SSE subscriber that falls this far behind is
/// `Lagged` (the stream skips the gap); `/api/events` remains the source of truth.
const BROADCAST_CAP: usize = 256;

/// One structured event. Serializes to the `/api/events` wire shape directly.
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    /// Monotonic sequence number (1-based). Lets `?since=<seq>` page forward and
    /// lets the client dedup the history fetch against the live stream.
    pub seq: u64,
    /// ISO-8601 UTC timestamp (rfc3339).
    pub at: String,
    /// Event kind, kebab-case by convention. Open-ended on purpose (see module docs).
    pub kind: String,
    /// Arbitrary structured payload (often includes `ptyId` and/or session `uuid`).
    pub data: serde_json::Value,
}

/// The shared event bus: ring buffer + broadcast + optional JSONL log file.
pub struct EventBus {
    ring: Mutex<VecDeque<Event>>,
    next_seq: AtomicU64,
    tx: broadcast::Sender<Event>,
    /// Append-only JSONL sink, when `--log-file` is set and the file opened.
    log: Option<Mutex<std::fs::File>>,
    /// Set once we've warned about a log write failure, so we warn at most once.
    log_warned: AtomicBool,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(None)
    }
}

impl EventBus {
    /// Build a bus, optionally appending each event as one JSON line to `log_file`.
    /// A path that can't be opened warns once to stderr and disables file logging;
    /// the in-memory bus works regardless (best-effort logging never gates recording).
    pub fn new(log_file: Option<&Path>) -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAP);
        let log = log_file.and_then(|path| {
            match std::fs::OpenOptions::new().create(true).append(true).open(path) {
                Ok(f) => Some(Mutex::new(f)),
                Err(e) => {
                    eprintln!("eigenform: could not open event log {}: {e}", path.display());
                    None
                }
            }
        });
        Self {
            ring: Mutex::new(VecDeque::with_capacity(RING_CAP)),
            next_seq: AtomicU64::new(1),
            tx,
            log,
            log_warned: AtomicBool::new(false),
        }
    }

    /// Record an event: stamp it, push it onto the ring (evicting the oldest past
    /// `RING_CAP`), fan it out to live subscribers, and best-effort append it to the
    /// log file. Callable from any thread; never blocks on a subscriber or the disk.
    pub fn record(&self, kind: impl Into<String>, data: serde_json::Value) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let event = Event {
            seq,
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            kind: kind.into(),
            data,
        };

        {
            // Poison-recovery discipline (mirrors host.rs): a panic elsewhere must not
            // turn the bus into a landmine that cascades on every later record.
            let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            if ring.len() >= RING_CAP {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }

        // No subscribers is not an error — `send` fails only when the channel is closed
        // (never, since the bus holds `tx`), so ignore the result.
        let _ = self.tx.send(event.clone());

        self.append_log(&event);
    }

    /// The buffered events, oldest first, optionally only those with `seq > since`.
    pub fn snapshot(&self, since: Option<u64>) -> Vec<Event> {
        let ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        ring.iter()
            .filter(|e| since.is_none_or(|s| e.seq > s))
            .cloned()
            .collect()
    }

    /// A live subscription to newly-recorded events (for the SSE route).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Best-effort JSONL append. A write error warns once, then stays silent — a
    /// full or unwritable disk must never take the daemon down or block a caller.
    fn append_log(&self, event: &Event) {
        let Some(log) = &self.log else { return };
        let line = match serde_json::to_string(event) {
            Ok(mut s) => {
                s.push('\n');
                s
            }
            Err(_) => return,
        };
        let mut file = log.lock().unwrap_or_else(|e| e.into_inner());
        if file.write_all(line.as_bytes()).is_err() && !self.log_warned.swap(true, Ordering::Relaxed)
        {
            eprintln!("eigenform: event log write failed; further errors suppressed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_stamp_monotonic_sequence_numbers() {
        let bus = EventBus::default();
        bus.record("pty-spawned", serde_json::json!({ "id": "1" }));
        bus.record("pty-exited", serde_json::json!({ "id": "1" }));
        let all = bus.snapshot(None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].seq, 1);
        assert_eq!(all[1].seq, 2);
        assert_eq!(all[0].kind, "pty-spawned");
        assert!(!all[0].at.is_empty(), "each event carries a timestamp");
    }

    #[test]
    fn since_filters_to_newer_events_only() {
        let bus = EventBus::default();
        for i in 0..5 {
            bus.record("tick", serde_json::json!({ "i": i }));
        }
        // seq values are 1..=5; `since=3` yields seq 4 and 5.
        let newer = bus.snapshot(Some(3));
        assert_eq!(newer.len(), 2);
        assert_eq!(newer[0].seq, 4);
        assert_eq!(newer[1].seq, 5);
        // `since` past the end yields nothing (not an error).
        assert!(bus.snapshot(Some(99)).is_empty());
        // No filter yields everything, oldest first.
        assert_eq!(bus.snapshot(None).len(), 5);
    }

    #[test]
    fn ring_buffer_evicts_oldest_past_cap() {
        let bus = EventBus::default();
        // Overfill by 10; the oldest 10 must be evicted, seq numbers keep climbing.
        for i in 0..(RING_CAP + 10) {
            bus.record("fill", serde_json::json!({ "i": i }));
        }
        let all = bus.snapshot(None);
        assert_eq!(all.len(), RING_CAP, "ring is capped at RING_CAP");
        // Oldest retained is the 11th recorded (seq 11); newest is seq RING_CAP+10.
        assert_eq!(all.first().unwrap().seq, 11);
        assert_eq!(all.last().unwrap().seq, (RING_CAP + 10) as u64);
    }

    #[test]
    fn open_ended_kinds_pass_through_untouched() {
        // A future branch's new kind must record without any change to the bus.
        let bus = EventBus::default();
        bus.record("downgrade-detected", serde_json::json!({ "from": "fable", "to": "opus" }));
        let all = bus.snapshot(None);
        assert_eq!(all[0].kind, "downgrade-detected");
        assert_eq!(all[0].data["from"], "fable");
    }

    #[test]
    fn log_file_appends_one_json_line_per_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let bus = EventBus::new(Some(&path));
        bus.record("pty-spawned", serde_json::json!({ "id": "7" }));
        bus.record("pty-exited", serde_json::json!({ "id": "7" }));
        let body = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "one JSON line per event");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["kind"], "pty-spawned");
        assert_eq!(first["seq"], 1);
        assert_eq!(first["data"]["id"], "7");
    }

    #[test]
    fn a_bad_log_path_disables_logging_without_panicking() {
        // A path whose parent directory does not exist can't be opened; the bus must
        // still record in memory (best-effort logging never gates recording).
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("no-such-dir").join("events.jsonl");
        let bus = EventBus::new(Some(&bogus));
        bus.record("pty-spawned", serde_json::json!({ "id": "1" }));
        assert_eq!(bus.snapshot(None).len(), 1);
    }
}
