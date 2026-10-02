//! The plan gate end to end over HTTP: Claude Code's ExitPlanMode hook parks a plan,
//! the pane reads it and decides, and the decision comes back as the hook response.

#[path = "helpers/mod.rs"]
mod helpers;

use eigenform_daemon::{app, Config};

async fn start(cfg: Config) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(cfg)).await.unwrap();
    });
    format!("http://{addr}")
}

const HOOK: &str = r##"{"session_id":"s-1","cwd":"/w","hook_event_name":"PermissionRequest","permission_mode":"plan","tool_name":"ExitPlanMode","tool_use_id":"toolu_9","tool_input":{"plan":"# Plan\n1. ship the gate"}}"##;

async fn wait_for_review(base: &str) -> serde_json::Value {
    for _ in 0..200 {
        let list: serde_json::Value =
            serde_json::from_str(&helpers::http_get(base, "/api/plan-reviews").await).unwrap();
        if let Some(r) = list.as_array().and_then(|a| a.first()) {
            return r.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("review never parked");
}

#[tokio::test]
async fn send_back_returns_a_deny_with_the_critique() {
    let base = start(Config {
        program: "cat".into(),
        ..Default::default()
    })
    .await;
    let b2 = base.clone();
    let hook = tokio::spawn(async move {
        helpers::http_post(
            &b2,
            "/api/hooks/plan-review",
            &[("Content-Type", "application/json")],
            HOOK,
        )
        .await
    });
    let review = wait_for_review(&base).await;
    assert_eq!(review["sessionId"], "s-1");
    assert_eq!(review["source"], "tool_input.plan");
    let id = review["id"].as_str().unwrap();
    assert_eq!(
        helpers::http_get(&base, &format!("/api/plan-reviews/{id}/plan")).await,
        "# Plan\n1. ship the gate"
    );

    // CSRF: a cross-site page, or a non-JSON "simple request", can't decide.
    let decision = r#"{"decision":"approve"}"#;
    let (st, _) = helpers::http_post(
        &base,
        &format!("/api/plan-reviews/{id}/decision"),
        &[
            ("Content-Type", "application/json"),
            ("Origin", "https://evil.example"),
        ],
        decision,
    )
    .await;
    assert_eq!(st, 403);
    let (st, _) = helpers::http_post(
        &base,
        &format!("/api/plan-reviews/{id}/decision"),
        &[("Content-Type", "text/plain")],
        decision,
    )
    .await;
    assert_eq!(st, 403);

    let (st, _) = helpers::http_post(
        &base,
        &format!("/api/plan-reviews/{id}/decision"),
        &[("Content-Type", "application/json"), ("Origin", &base)],
        r#"{"decision":"send_back","message":"1. \"ship the gate\" → behind a flag"}"#,
    )
    .await;
    assert_eq!(st, 204);

    let (st, body) = hook.await.unwrap();
    assert_eq!(st, 200);
    let v: serde_json::Value = serde_json::from_str(body.trim()).expect("hook response is JSON");
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
    assert_eq!(
        v["hookSpecificOutput"]["decision"]["message"],
        "1. \"ship the gate\" → behind a flag"
    );
    let list = helpers::http_get(&base, "/api/plan-reviews").await;
    assert_eq!(list.trim(), "[]");
}

#[tokio::test]
async fn hold_timeout_answers_no_decision_and_cross_site_hooks_are_refused() {
    let base = start(Config {
        program: "cat".into(),
        plan_review_hold_secs: 1,
        ..Default::default()
    })
    .await;
    let (st, body) = helpers::http_post(
        &base,
        "/api/hooks/plan-review",
        &[("Content-Type", "application/json")],
        HOOK,
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(body.trim(), "", "no decision → Claude Code's own prompt");

    let (st, _) = helpers::http_post(
        &base,
        "/api/hooks/plan-review",
        &[
            ("Content-Type", "text/plain"),
            ("Origin", "https://evil.example"),
        ],
        HOOK,
    )
    .await;
    assert_eq!(st, 403);
}
