//! Turn tracking: can a claude pty be killed without losing work? (spike 18)
//!
//! Every claude the daemon spawns gets HTTP hooks (`--settings`, merged with the
//! user's own) that POST to `/api/hooks/turn/<token>`. The token is per spawn and
//! unguessable, so the route needs no other auth and a hook can never be credited to
//! the wrong pty. From those events the tracker knows whether a turn is open and how
//! many background subagents are running. `SessionStart` (forwarded by a command
//! hook) marks a session that was opened but never prompted as seen and idle.
//!
//! Hooks alone are not enough (spike 18): an Esc interrupt fires no hook (the
//! transcript records it), and `Stop` fires while background Bash is still running
//! (the process tree shows it). [`SessionTurns::kill_safety`] combines all three, and
//! anything it can't establish counts as unsafe, so a close falls back to detaching.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::host::PtyId;
use crate::AppState;

/// The hook events that drive [`TurnState`]. Other events carry nothing the safety
/// check needs (a permission prompt is already inside an open turn).
const EVENTS: [&str; 5] = [
    "UserPromptSubmit",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
];

/// The interrupt marker Claude Code appends to the transcript on Esc (spike 18).
const INTERRUPT_MARKER: &str = "[Request interrupted by user";

/// What the hooks have said about one spawned claude.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TurnState {
    /// At least one hook arrived: the hooks are installed and reaching us.
    pub seen: bool,
    /// `UserPromptSubmit` opened a turn and no `Stop`/`StopFailure` has closed it.
    pub turn_open: bool,
    /// Background subagents started and not yet stopped, by `agent_id`.
    pub subagents: HashSet<String>,
    /// The session transcript, from the hook payload's `transcript_path`.
    pub transcript: Option<PathBuf>,
}

impl TurnState {
    /// Apply one hook payload.
    pub fn apply(&mut self, event: &serde_json::Value) {
        self.seen = true;
        if let Some(path) = event.get("transcript_path").and_then(|v| v.as_str()) {
            self.transcript = Some(PathBuf::from(path));
        }
        let agent = event.get("agent_id").and_then(|v| v.as_str());
        match event.get("hook_event_name").and_then(|v| v.as_str()) {
            Some("UserPromptSubmit") => self.turn_open = true,
            Some("Stop" | "StopFailure") => self.turn_open = false,
            Some("SubagentStart") => {
                if let Some(a) = agent {
                    self.subagents.insert(a.to_string());
                }
            }
            Some("SubagentStop") => {
                if let Some(a) = agent {
                    self.subagents.remove(a);
                }
            }
            _ => {}
        }
    }
}

/// Why a pty is not safe to kill. The `Display` text goes back to the client.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unsafe {
    #[error("no turn hooks from this session (not a claude the daemon spawned, or hooks are off)")]
    NoHooks,
    #[error("a turn is in progress")]
    TurnOpen,
    #[error("{0} background subagent(s) running")]
    Subagents(usize),
    #[error("{0} shell(s) still running")]
    Shells(usize),
    #[error("couldn't inspect the process tree")]
    NoProcessTree,
}

/// The safety rule from spike 18, over already-gathered facts. `interrupted` and
/// `shells` are only consulted when the cheaper checks before them pass.
pub fn check(
    state: Option<&TurnState>,
    interrupted: impl FnOnce(&Path) -> bool,
    shells: impl FnOnce() -> Option<usize>,
) -> Result<(), Unsafe> {
    let state = state.filter(|s| s.seen).ok_or(Unsafe::NoHooks)?;
    if state.turn_open {
        // An Esc fires no hook; the transcript is the only record that the turn ended.
        let closed = state.transcript.as_deref().is_some_and(interrupted);
        if !closed {
            return Err(Unsafe::TurnOpen);
        }
    }
    if !state.subagents.is_empty() {
        return Err(Unsafe::Subagents(state.subagents.len()));
    }
    match shells() {
        None => Err(Unsafe::NoProcessTree),
        Some(0) => Ok(()),
        Some(n) => Err(Unsafe::Shells(n)),
    }
}

/// Per-spawn turn state, keyed by hook token, plus which pty owns which token.
#[derive(Default)]
pub struct SessionTurns {
    by_token: Mutex<HashMap<String, TurnState>>,
    token_of: Mutex<HashMap<PtyId, String>>,
    counter: AtomicU64,
}

impl SessionTurns {
    /// A fresh unguessable token: `RandomState` is seeded from OS randomness per
    /// process, mixed with a counter and the clock so two calls never collide.
    pub fn new_token(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let word = |salt: u64| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(n);
            h.write_u64(salt);
            h.write_u128(nanos);
            h.finish()
        };
        format!("{:016x}{:016x}", word(1), word(2))
    }

    /// Tie `token` to the pty spawned with it.
    pub fn bind(&self, pty: PtyId, token: String) {
        self.token_of
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(pty, token);
    }

    /// Forget a pty (it was killed) and its turn state.
    pub fn forget(&self, pty: PtyId) {
        let token = self
            .token_of
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&pty);
        if let Some(t) = token {
            self.by_token
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&t);
        }
    }

    /// Apply a hook payload to the spawn that owns `token`.
    pub fn record(&self, token: &str, event: &serde_json::Value) {
        self.by_token
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(token.to_string())
            .or_default()
            .apply(event);
    }

    /// The turn state of `pty`, if it was spawned with hooks and any have arrived.
    pub fn state_of(&self, pty: PtyId) -> Option<TurnState> {
        let token = self
            .token_of
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&pty)
            .cloned()?;
        self.by_token
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&token)
            .cloned()
    }

    /// Is it safe to kill `pty`, whose child is `child_pid`?
    pub fn kill_safety(&self, pty: PtyId, child_pid: u32) -> Result<(), Unsafe> {
        check(
            self.state_of(pty).as_ref(),
            transcript_ends_interrupted,
            || tool_shells_under(child_pid),
        )
    }
}

/// The `--settings` value that installs the turn hooks for one spawn.
///
/// `SessionStart` accepts only command hooks, so it forwards its payload with `curl`
/// (stdin is the hook JSON; `-o /dev/null` because a SessionStart hook's stdout becomes
/// context for Claude). It is what lets a session that was opened and never prompted
/// count as seen. Without curl it simply never fires, and such a session is kept.
pub fn settings_arg(port: u16, token: &str) -> String {
    let url = format!("http://127.0.0.1:{port}/api/hooks/turn/{token}");
    let mut hooks: serde_json::Map<String, serde_json::Value> = EVENTS
        .iter()
        .map(|e| {
            (
                e.to_string(),
                serde_json::json!([{ "hooks": [{ "type": "http", "url": url, "timeout": 2 }] }]),
            )
        })
        .collect();
    let forward = format!(
        "curl -s -m 2 -o /dev/null -X POST -H 'content-type: application/json' --data-binary @- {url}"
    );
    hooks.insert(
        "SessionStart".to_string(),
        serde_json::json!([{ "hooks": [{ "type": "command", "command": forward, "timeout": 3 }] }]),
    );
    serde_json::json!({ "hooks": hooks }).to_string()
}

/// Does the transcript's last conversational entry say the user interrupted the turn?
/// Reads only the tail: the marker is always the newest user entry when it applies.
pub fn transcript_ends_interrupted(path: &Path) -> bool {
    const TAIL: u64 = 64 * 1024;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).is_err() {
        return false;
    }
    // Lossy: a tail read can start inside a multibyte char.
    let mut bytes = Vec::new();
    if f.read_to_end(&mut bytes).is_err() {
        return false;
    }
    let buf = String::from_utf8_lossy(&bytes);
    ends_interrupted(&buf)
}

/// [`transcript_ends_interrupted`] over JSONL text already in memory.
pub fn ends_interrupted(jsonl: &str) -> bool {
    for line in jsonl.lines().rev() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue; // the first line of a tail read is usually cut off
        };
        match entry.get("type").and_then(|v| v.as_str()) {
            Some("user") => {
                let content = entry.pointer("/message/content");
                let text = match content {
                    Some(serde_json::Value::String(s)) => Some(s.as_str()),
                    Some(serde_json::Value::Array(parts)) => parts
                        .iter()
                        .find_map(|p| p.get("text").and_then(|t| t.as_str())),
                    _ => None,
                };
                return text.is_some_and(|t| t.starts_with(INTERRUPT_MARKER));
            }
            Some("assistant") => return false,
            _ => {} // attachments, system notes, summaries: not part of the turn
        }
    }
    false
}

/// How many Bash tool shells are running under `pid`. Claude Code launches every one
/// (foreground or background) as `bash -c source …/shell-snapshots/snapshot-…`, which
/// keeps MCP servers, also children of claude, out of the count. `None` = `ps` failed.
pub fn tool_shells_under(pid: u32) -> Option<usize> {
    let out = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,args="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(count_tool_shells(
        &String::from_utf8_lossy(&out.stdout),
        pid,
    ))
}

/// [`tool_shells_under`] over `ps -o pid=,ppid=,args=` output.
pub fn count_tool_shells(ps: &str, root: u32) -> usize {
    let mut children: HashMap<u32, Vec<(u32, &str)>> = HashMap::new();
    for line in ps.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (it.next(), it.next()) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
            continue;
        };
        // args is the rest of the line after the two numeric columns.
        let args = line
            .trim_start()
            .splitn(3, char::is_whitespace)
            .nth(2)
            .unwrap_or("");
        children.entry(ppid).or_default().push((pid, args));
    }
    let mut count = 0;
    let mut stack = vec![root];
    let mut visited = HashSet::new();
    while let Some(p) = stack.pop() {
        if !visited.insert(p) {
            continue;
        }
        for &(child, args) in children.get(&p).map(Vec::as_slice).unwrap_or(&[]) {
            if args.contains("shell-snapshots/snapshot-") {
                count += 1;
            }
            stack.push(child);
        }
    }
    count
}

/// `POST /api/hooks/turn/:token` — Claude Code's turn hooks. Always an empty 200: the
/// body of a `Stop` or `UserPromptSubmit` hook response would otherwise be read as a
/// decision or as context for Claude.
pub(crate) async fn turn_hook_route(
    AxumPath(token): AxumPath<String>,
    State(state): State<AppState>,
    axum::Json(event): axum::Json<serde_json::Value>,
) -> Response {
    state.turns.record(&token, &event);
    StatusCode::OK.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(name: &str) -> serde_json::Value {
        json!({ "hook_event_name": name, "transcript_path": "/t.jsonl" })
    }

    fn state_after(events: &[serde_json::Value]) -> TurnState {
        let mut s = TurnState::default();
        for e in events {
            s.apply(e);
        }
        s
    }

    #[test]
    fn unseen_is_unsafe() {
        assert_eq!(check(None, |_| true, || Some(0)), Err(Unsafe::NoHooks));
        let blank = TurnState::default();
        assert_eq!(
            check(Some(&blank), |_| true, || Some(0)),
            Err(Unsafe::NoHooks)
        );
    }

    #[test]
    fn closed_turn_with_nothing_running_is_safe() {
        let s = state_after(&[ev("UserPromptSubmit"), ev("Stop")]);
        assert_eq!(check(Some(&s), |_| false, || Some(0)), Ok(()));
    }

    #[test]
    fn open_turn_is_unsafe_unless_transcript_shows_interrupt() {
        let s = state_after(&[ev("UserPromptSubmit")]);
        assert_eq!(
            check(Some(&s), |_| false, || Some(0)),
            Err(Unsafe::TurnOpen)
        );
        assert_eq!(check(Some(&s), |_| true, || Some(0)), Ok(()));
    }

    #[test]
    fn stop_failure_closes_the_turn() {
        let s = state_after(&[ev("UserPromptSubmit"), ev("StopFailure")]);
        assert!(!s.turn_open);
    }

    #[test]
    fn background_subagent_outlives_stop() {
        // Spike 18 scenario D: Stop fires while the subagent is still running.
        let start = json!({ "hook_event_name": "SubagentStart", "agent_id": "a1" });
        let stop = json!({ "hook_event_name": "SubagentStop", "agent_id": "a1" });
        let s = state_after(&[ev("UserPromptSubmit"), start.clone(), ev("Stop")]);
        assert_eq!(
            check(Some(&s), |_| false, || Some(0)),
            Err(Unsafe::Subagents(1))
        );
        let s = state_after(&[ev("UserPromptSubmit"), start, ev("Stop"), stop]);
        assert_eq!(check(Some(&s), |_| false, || Some(0)), Ok(()));
    }

    #[test]
    fn running_shell_or_unreadable_tree_is_unsafe() {
        let s = state_after(&[ev("UserPromptSubmit"), ev("Stop")]);
        assert_eq!(
            check(Some(&s), |_| false, || Some(2)),
            Err(Unsafe::Shells(2))
        );
        assert_eq!(
            check(Some(&s), |_| false, || None),
            Err(Unsafe::NoProcessTree)
        );
    }

    #[test]
    fn tracker_keys_state_by_token_and_pty() {
        let t = SessionTurns::default();
        let a = t.new_token();
        let b = t.new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        t.bind(7, a.clone());
        t.record(&a, &ev("UserPromptSubmit"));
        t.record(&b, &ev("Stop")); // another spawn's hook never touches pty 7
        assert!(t.state_of(7).unwrap().turn_open);
        t.forget(7);
        assert_eq!(t.state_of(7), None);
    }

    #[test]
    fn interrupt_marker_is_read_from_the_last_conversational_entry() {
        // Spike 18 scenario B, transcript tail after Esc.
        let interrupted = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"The user doesn't want to proceed"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#,
            r#"{"type":"attachment","attachment":{}}"#,
        ]
        .join("\n");
        assert!(ends_interrupted(&interrupted));
        assert!(ends_interrupted(&format!(
            "{{cut line\n{}",
            r#"{"type":"user","message":{"content":"[Request interrupted by user]"}}"#
        )));
        let next_prompt = format!(
            "{interrupted}\n{}",
            r#"{"type":"user","message":{"content":"try again"}}"#
        );
        assert!(!ends_interrupted(&next_prompt));
        assert!(!ends_interrupted(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}"#
        ));
    }

    #[test]
    fn counts_only_tool_shells_below_the_root() {
        // Spike 18 scenario C's tree, plus an MCP server and an unrelated shell.
        let ps = "\
  679     1 claude --settings {}
 1218   679 /bin/bash -c source /h/.claude/shell-snapshots/snapshot-bash-1.sh && eval 'python3 x'
 1219  1218 python3 -c import time; time.sleep(100)
 1300   679 node /h/mcp/server.js
 1400     1 /bin/bash -c source /h/.claude/shell-snapshots/snapshot-bash-2.sh
";
        assert_eq!(count_tool_shells(ps, 679), 1);
        assert_eq!(count_tool_shells(ps, 1300), 0);
    }

    #[test]
    fn settings_arg_installs_every_turn_event_on_this_token() {
        let v: serde_json::Value = serde_json::from_str(&settings_arg(4317, "abc")).unwrap();
        for e in EVENTS {
            assert_eq!(
                v["hooks"][e][0]["hooks"][0]["url"],
                "http://127.0.0.1:4317/api/hooks/turn/abc"
            );
            assert_eq!(v["hooks"][e][0]["hooks"][0]["type"], "http");
        }
        let start = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(
            start.contains("-o /dev/null"),
            "stdout would become Claude's context"
        );
        assert!(start.ends_with("http://127.0.0.1:4317/api/hooks/turn/abc"));
    }

    #[test]
    fn session_start_alone_makes_an_unprompted_session_safe() {
        let s = state_after(&[json!({ "hook_event_name": "SessionStart", "source": "resume" })]);
        assert_eq!(check(Some(&s), |_| false, || Some(0)), Ok(()));
    }
}
