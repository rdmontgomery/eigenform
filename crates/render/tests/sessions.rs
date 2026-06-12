//! sessions_view: a flat recent-list, newest at the bottom, with short uuid, relative
//! time, and title.

use chrono::{DateTime, Utc};
use eigenform_forest::SessionRef;
use eigenform_render::{render_text, sessions_view};

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn sref(uuid: &str, recency: &str, title: &str) -> SessionRef {
    SessionRef {
        uuid: uuid.to_string(),
        path: "/x.jsonl".into(),
        cwd: "/home/me/p".into(),
        recency: at(recency),
        title: Some(title.to_string()),
    }
}

#[test]
fn newest_session_is_at_the_bottom() {
    let now = at("2026-06-03T12:00:00Z");
    // recent-first, as forest::list returns
    let sessions = vec![
        sref("aaaa1111-x", "2026-06-03T10:00:00Z", "recent one"),
        sref("bbbb2222-x", "2026-06-01T10:00:00Z", "older one"),
    ];
    let out = render_text(&sessions_view(&sessions, now, false));
    let body: Vec<&str> = out.lines().skip(1).collect(); // skip header
    assert!(body[0].contains("bbbb2222"), "older at top:\n{out}");
    assert!(body[1].contains("aaaa1111"), "newest at bottom:\n{out}");
    assert!(body[1].contains("recent one"));
}

#[test]
fn shows_short_uuid_and_relative_time() {
    let now = at("2026-06-03T12:00:00Z");
    let sessions = vec![sref("277a983f-aaaa-bbbb-cccc-dddddddddddd", "2026-06-03T10:00:00Z", "my title")];
    let out = render_text(&sessions_view(&sessions, now, false));
    assert!(out.contains("277a983f"), "short uuid:\n{out}");
    assert!(!out.contains("277a983f-aaaa"), "not the full uuid:\n{out}");
    assert!(out.contains("2h ago"), "relative time:\n{out}");
    assert!(out.contains("my title"));
}

#[test]
fn untitled_sessions_get_a_placeholder() {
    let now = at("2026-06-03T12:00:00Z");
    let mut s = sref("c0ffee00-x", "2026-06-03T11:00:00Z", "");
    s.title = None;
    let out = render_text(&sessions_view(&[s], now, false));
    assert!(out.contains("(untitled)"), "got:\n{out}");
}
