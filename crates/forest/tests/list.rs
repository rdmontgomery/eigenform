//! list: enumerate sessions, scope to a project (or all), filter to a recency window,
//! sort recent-first.

use chrono::{DateTime, Duration, Utc};
use eigen_forest::{list, Scope};

fn ts(s: &str) -> &str {
    s
}

fn session(uuid: &str, when: &str) -> String {
    format!(r#"{{"type":"user","uuid":"{uuid}","timestamp":"{}","cwd":"CWD","sessionId":"{uuid}"}}"#, ts(when))
}

/// proj-a (cwd /home/me/a): s_recent (06-03), s_old (05-01).
/// proj-b (cwd /home/me/b): s_b (06-02).
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let write = |proj: &str, cwd: &str, uuid: &str, when: &str| {
        let pdir = root.join(proj);
        std::fs::create_dir_all(&pdir).unwrap();
        let line = session(uuid, when).replace("CWD", cwd);
        std::fs::write(pdir.join(format!("{uuid}.jsonl")), line + "\n").unwrap();
    };
    write("-a", "/home/me/a", "s_recent", "2026-06-03T00:00:00Z");
    write("-a", "/home/me/a", "s_old", "2026-05-01T00:00:00Z");
    write("-b", "/home/me/b", "s_b", "2026-06-02T00:00:00Z");
    dir
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-06-03T12:00:00Z").unwrap().with_timezone(&Utc)
}

#[test]
fn scope_to_a_project_excludes_other_projects() {
    let dir = fixture();
    let got = list(dir.path(), Scope::Project("/home/me/a".into()), None, now()).unwrap();
    let uuids: Vec<_> = got.iter().map(|s| s.uuid.clone()).collect();
    assert_eq!(uuids, vec!["s_recent", "s_old"], "only project a, recent-first");
}

#[test]
fn window_excludes_sessions_older_than_since() {
    let dir = fixture();
    let got = list(
        dir.path(),
        Scope::Project("/home/me/a".into()),
        Some(Duration::days(7)),
        now(),
    )
    .unwrap();
    let uuids: Vec<_> = got.iter().map(|s| s.uuid.clone()).collect();
    assert_eq!(uuids, vec!["s_recent"], "s_old is >30 days back");
}

#[test]
fn all_projects_sorted_recent_first() {
    let dir = fixture();
    let got = list(dir.path(), Scope::AllProjects, Some(Duration::days(7)), now()).unwrap();
    let uuids: Vec<_> = got.iter().map(|s| s.uuid.clone()).collect();
    assert_eq!(uuids, vec!["s_recent", "s_b"], "06-03 before 06-02");
}
