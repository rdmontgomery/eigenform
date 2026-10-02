//! The plan gate: Claude Code's plan approval, answered from eigenform's pane.
//!
//! Claude Code calls an HTTP `PermissionRequest` hook (matcher `ExitPlanMode`) at
//! `POST /api/hooks/plan-review`. The daemon parks that request as a pending review
//! and holds it open while the human reads, annotates, and decides in the artifact
//! pane. The decision becomes the hook's response:
//!
//! - **approve** → `decision.behavior = "allow"`;
//! - **send back** → `decision.behavior = "deny"` with the compiled critique as
//!   `decision.message`, the plan feedback Claude revises against (spike 17);
//! - **decide in terminal**, a hold timeout, or the hook being cancelled → an empty
//!   2xx, i.e. *no decision*, so Claude Code's own approval prompt takes over.
//!
//! Every failure mode degrades to the normal TUI prompt: a daemon that isn't running
//! is a non-2xx/connection error, which Claude Code treats as non-blocking. The gate
//! can delay a plan; it can never wedge or silently approve one.
//!
//! The decision route is the only write. It refuses any request carrying a non-local
//! Origin and requires a JSON body, so a cross-site form post (a CORS "simple request")
//! can't approve a plan.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;

/// How long a review is held before falling back to the terminal prompt when the
/// config doesn't say. Just under the 1800s hook timeout the install snippet sets, so
/// the daemon answers (no decision) before Claude Code gives up on it.
pub const DEFAULT_HOLD: Duration = Duration::from_secs(1740);

/// What the human decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Approve,
    SendBack(String),
    /// No decision: let Claude Code's own prompt handle it.
    Terminal,
}

/// A parked review, as listed to the UI.
#[derive(Debug, Clone)]
pub struct Review {
    pub id: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub tool_use_id: Option<String>,
    /// The plan text, from `tool_input.plan`, else read from `tool_input`'s plan file.
    pub plan: String,
    /// Where the plan text came from: the plan file's path, or `"tool_input.plan"`.
    pub source: String,
    pub created_at: String,
}

struct Pending {
    review: Review,
    tx: oneshot::Sender<Decision>,
}

/// The daemon's set of parked reviews.
#[derive(Default)]
pub struct PlanGate {
    pending: Mutex<HashMap<String, Pending>>,
    next: std::sync::atomic::AtomicU64,
}

/// Removes a review when its hook request ends for any reason, including Claude Code
/// cancelling the hook (the handler future is dropped).
struct Parked {
    gate: Arc<PlanGate>,
    id: String,
}

impl Drop for Parked {
    fn drop(&mut self) {
        if let Ok(mut p) = self.gate.pending.lock() {
            p.remove(&self.id);
        }
    }
}

impl PlanGate {
    /// Reviews currently waiting on a human, oldest first.
    pub fn list(&self) -> Vec<Review> {
        let mut v: Vec<Review> = self
            .pending
            .lock()
            .map(|p| p.values().map(|x| x.review.clone()).collect())
            .unwrap_or_default();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        v
    }

    pub fn plan(&self, id: &str) -> Option<String> {
        self.pending
            .lock()
            .ok()?
            .get(id)
            .map(|p| p.review.plan.clone())
    }

    /// Deliver a decision. False when no such review is waiting (already decided,
    /// timed out, or cancelled).
    pub fn decide(&self, id: &str, d: Decision) -> bool {
        let Some(p) = self.pending.lock().ok().and_then(|mut m| m.remove(id)) else {
            return false;
        };
        p.tx.send(d).is_ok()
    }

    /// Park the hook `input` and wait up to `hold` for a decision. Returns the hook
    /// response body: a decision JSON, or "" for no decision.
    pub async fn hold(self: &Arc<Self>, input: &Value, hold: Duration) -> (Option<Review>, String) {
        if input["tool_name"] != "ExitPlanMode" {
            return (None, String::new());
        }
        let (plan, source) = plan_text(&input["tool_input"]);
        let n = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("pr{n}-{}", chrono::Utc::now().timestamp_millis());
        let review = Review {
            id: id.clone(),
            session_id: input["session_id"].as_str().map(str::to_string),
            cwd: input["cwd"].as_str().map(str::to_string),
            tool_use_id: input["tool_use_id"].as_str().map(str::to_string),
            plan,
            source,
            created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(
                id.clone(),
                Pending {
                    review: review.clone(),
                    tx,
                },
            );
        }
        let _parked = Parked {
            gate: Arc::clone(self),
            id,
        };
        let decision = match tokio::time::timeout(hold, rx).await {
            Ok(Ok(d)) => d,
            _ => Decision::Terminal,
        };
        (Some(review), response_body(&decision))
    }
}

/// The plan's text and where it came from. `tool_input.plan` when present; otherwise
/// any `*path*`/`*file*` string field naming a readable markdown file (the plan-file
/// shape of newer Claude Code versions; spike 17 pins which one ships).
pub fn plan_text(tool_input: &Value) -> (String, String) {
    if let Some(p) = tool_input["plan"].as_str().filter(|p| !p.trim().is_empty()) {
        return (p.to_string(), "tool_input.plan".to_string());
    }
    if let Some(obj) = tool_input.as_object() {
        for (k, v) in obj {
            let k = k.to_ascii_lowercase();
            let Some(path) = v.as_str() else { continue };
            if (k.contains("path") || k.contains("file")) && path.ends_with(".md") {
                if let Ok(text) = std::fs::read_to_string(path) {
                    return (text, path.to_string());
                }
            }
        }
    }
    (String::new(), "none".to_string())
}

/// The hook's response body for a decision ("" = no decision).
pub fn response_body(d: &Decision) -> String {
    let decision = match d {
        Decision::Approve => json!({ "behavior": "allow" }),
        Decision::SendBack(msg) => json!({ "behavior": "deny", "message": msg }),
        Decision::Terminal => return String::new(),
    };
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision,
        }
    })
    .to_string()
}

/// The `~/.claude/settings.json` fragment that routes plan approval through the gate.
pub fn settings_snippet(port: u16) -> Value {
    json!({
        "hooks": {
            "PermissionRequest": [{
                "matcher": "ExitPlanMode",
                "hooks": [{
                    "type": "http",
                    "url": format!("http://127.0.0.1:{port}/api/hooks/plan-review"),
                    "timeout": 1800
                }]
            }]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(plan: &str) -> Value {
        json!({
            "session_id": "s1",
            "cwd": "/w",
            "hook_event_name": "PermissionRequest",
            "permission_mode": "plan",
            "tool_name": "ExitPlanMode",
            "tool_use_id": "toolu_1",
            "tool_input": { "plan": plan }
        })
    }

    #[tokio::test]
    async fn approve_and_send_back_become_hook_decisions() {
        let gate = Arc::new(PlanGate::default());
        let g = Arc::clone(&gate);
        let held =
            tokio::spawn(async move { g.hold(&hook("# Plan\n1. ship"), DEFAULT_HOLD).await });
        // Wait for it to park.
        let id = loop {
            if let Some(r) = gate.list().first() {
                break r.id.clone();
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(gate.plan(&id).as_deref(), Some("# Plan\n1. ship"));
        assert!(gate.decide(
            &id,
            Decision::SendBack("1. \"ship\" → behind a flag".into())
        ));
        let (review, body) = held.await.unwrap();
        assert_eq!(review.unwrap().session_id.as_deref(), Some("s1"));
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(
            v["hookSpecificOutput"]["decision"]["message"],
            "1. \"ship\" → behind a flag"
        );
        assert!(gate.list().is_empty(), "decided reviews are gone");
        assert!(
            !gate.decide(&id, Decision::Approve),
            "a second decision is refused"
        );

        assert!(response_body(&Decision::Approve).contains(r#""behavior":"allow""#));
    }

    #[tokio::test]
    async fn timeout_and_other_tools_mean_no_decision() {
        let gate = Arc::new(PlanGate::default());
        let (_, body) = gate.hold(&hook("x"), Duration::from_millis(20)).await;
        assert_eq!(body, "", "timeout → no decision → the terminal prompt");
        assert!(gate.list().is_empty());

        let mut bash = hook("x");
        bash["tool_name"] = json!("Bash");
        let (review, body) = gate.hold(&bash, DEFAULT_HOLD).await;
        assert!(review.is_none());
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn a_cancelled_hook_unparks_its_review() {
        let gate = Arc::new(PlanGate::default());
        let g = Arc::clone(&gate);
        let held = tokio::spawn(async move { g.hold(&hook("x"), DEFAULT_HOLD).await });
        while gate.list().is_empty() {
            tokio::task::yield_now().await;
        }
        held.abort(); // Claude Code cancelled the hook (its timeout, or the user hit Esc).
        let _ = held.await;
        assert!(gate.list().is_empty());
    }

    #[test]
    fn plan_text_falls_back_to_a_plan_file_field() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("brave-plan.md");
        std::fs::write(&f, "# From file").unwrap();
        let (text, src) = plan_text(&json!({ "planFilePath": f.to_str().unwrap() }));
        assert_eq!(text, "# From file");
        assert_eq!(src, f.to_str().unwrap());
        assert_eq!(plan_text(&json!({})).1, "none");
    }
}
