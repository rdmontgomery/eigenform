//! The session digest's facets: what a transcript worked on vs only mentioned, and the
//! incremental cache that parses only what was appended since the last refresh.

use eigenform_forest::{cached_digest, session_digest};
use serde_json::json;
use std::path::Path;

/// A checkout at `dir` whose origin is `url`.
fn repo(dir: &Path, url: &str) {
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(
        dir.join(".git/config"),
        format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"),
    )
    .unwrap();
}

fn line(v: serde_json::Value) -> String {
    format!("{v}\n")
}

fn user(cwd: &Path, text: &str) -> String {
    line(json!({"type":"user","cwd":cwd,"gitBranch":"feat/x",
        "message":{"role":"user","content":text}}))
}

fn assistant_text(cwd: &Path, text: &str) -> String {
    line(json!({"type":"assistant","cwd":cwd,"gitBranch":"feat/x",
        "message":{"role":"assistant","content":[{"type":"text","text":text}],
                   "usage":{"output_tokens":10}}}))
}

fn tool_use(cwd: &Path, id: &str, name: &str, input: serde_json::Value) -> String {
    line(json!({"type":"assistant","cwd":cwd,"gitBranch":"feat/x",
        "message":{"role":"assistant","content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}))
}

fn tool_result(cwd: &Path, id: &str, out: &str) -> String {
    line(json!({"type":"user","cwd":cwd,"gitBranch":"feat/x",
        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":out}]}}))
}

fn turn_end() -> String {
    line(json!({"type":"system","subtype":"turn_duration"}))
}

/// A session in a checkout of `me/app` that opens a PR, reads another, pushes, and talks
/// about a third.
fn transcript(cwd: &Path) -> Vec<String> {
    vec![
        user(
            cwd,
            "look at https://github.com/other/lib/pull/9 and fix #12 here",
        ),
        tool_use(
            cwd,
            "t1",
            "Bash",
            json!({"command":"git push -u origin feat/x"}),
        ),
        tool_result(
            cwd,
            "t1",
            "remote: Create a pull request for 'feat/x' on GitHub by visiting:\n\
             remote:      https://github.com/me/app/pull/new/feat/x\n\
             To https://github.com/me/app\n * [new branch]      feat/x -> feat/x",
        ),
        tool_use(
            cwd,
            "t2",
            "Bash",
            json!({"command":"gh pr create --title \"Fix 12\" --body \"$(cat <<'EOF'\nsee #40\nEOF\n)\""}),
        ),
        tool_result(cwd, "t2", "https://github.com/me/app/pull/57\n"),
        tool_use(
            cwd,
            "t3",
            "mcp__github__pull_request_read",
            json!({"owner":"Upstream","repo":"Core","pullNumber":101,"method":"get"}),
        ),
        tool_result(
            cwd,
            "t3",
            "{\"body\":\"see https://github.com/upstream/core/pull/5\"}",
        ),
        assistant_text(
            cwd,
            "Opened `me/app#57`; https://github.com/me/app/pull/57 is up. Upstream ref is \
             other/lib#9, colour `#333`.",
        ),
        turn_end(),
    ]
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[test]
fn work_is_what_the_session_acted_on_and_mentions_are_the_rest() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("app");
    repo(&cwd, "git@github.com:Me/app.git");
    let p = home.path().join("s.jsonl");
    std::fs::write(&p, transcript(&cwd).concat()).unwrap();

    let d = session_digest(&p);
    assert_eq!(d.spark, vec![10]);
    assert_eq!(
        sorted(d.facets.work),
        sorted(vec![
            "repo:me/app".into(),
            "branch:feat/x".into(),
            "pr:me/app#57".into(),
            "repo:upstream/core".into(),
            "pr:upstream/core#101".into(),
        ]),
        "cwd repo, branch, the PR the create printed, the PR a GitHub tool read"
    );
    assert_eq!(
        sorted(d.facets.mention),
        sorted(vec![
            "repo:other/lib".into(),
            "pr:other/lib#9".into(),
            "ref:me/app#12".into(),
        ]),
        "a pasted URL, a bare #12 resolved to the cwd repo; #57 is work so not repeated, \
         other/lib#9 is already known as a PR, #333 is code, #40 sat in a create's body, \
         and a read tool's output is not a mention"
    );
}

#[test]
fn gh_commands_with_a_number_are_work_on_that_pr() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("app");
    repo(&cwd, "https://github.com/me/app");
    let p = home.path().join("s.jsonl");
    std::fs::write(
        &p,
        [
            tool_use(
                &cwd,
                "a",
                "Bash",
                json!({"command":"gh pr checks 8 && gh issue comment 3 --body \"done\""}),
            ),
            tool_use(
                &cwd,
                "b",
                "Bash",
                json!({"command":"gh pr merge 4 -R Elsewhere/Thing --squash"}),
            ),
        ]
        .concat(),
    )
    .unwrap();
    let work = session_digest(&p).facets.work;
    for f in [
        "pr:me/app#8",
        "issue:me/app#3",
        "pr:elsewhere/thing#4",
        "repo:elsewhere/thing",
    ] {
        assert!(work.contains(&f.to_string()), "{f} in {work:?}");
    }
}

#[test]
fn a_worktree_resolves_to_its_main_checkouts_remote() {
    let home = tempfile::tempdir().unwrap();
    let main = home.path().join("app");
    repo(&main, "https://github.com/me/app.git");
    let gd = main.join(".git/worktrees/wt");
    std::fs::create_dir_all(&gd).unwrap();
    std::fs::write(gd.join("commondir"), "../..\n").unwrap();
    let wt = home.path().join("wt");
    std::fs::create_dir_all(wt.join("src")).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();

    let p = home.path().join("s.jsonl");
    std::fs::write(&p, user(&wt.join("src"), "hi")).unwrap();
    assert!(session_digest(&p)
        .facets
        .work
        .contains(&"repo:me/app".to_string()));
}

#[test]
fn the_cache_parses_only_the_appended_tail_and_matches_a_full_read() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let cwd = home.path().join("app");
    repo(&cwd, "https://github.com/me/app");
    let p = home.path().join("s.jsonl");
    let lines = transcript(&cwd);

    // Split between `gh pr create` and its result: the pending call must survive.
    for cut in 1..lines.len() {
        let _ = std::fs::remove_file(state.path().join("sid.json"));
        std::fs::write(&p, lines[..cut].concat()).unwrap();
        cached_digest(state.path(), "sid", &p);
        std::fs::write(&p, lines.concat()).unwrap();
        assert_eq!(
            cached_digest(state.path(), "sid", &p),
            session_digest(&p),
            "cut at {cut}"
        );
    }

    // Prove the second read resumed from the offset rather than re-reading: tamper
    // with what the cache holds, append, and the tampered value survives.
    let sp = state.path().join("sid.json");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sp).unwrap()).unwrap();
    doc["work"]
        .as_array_mut()
        .unwrap()
        .push(json!("repo:planted/marker"));
    std::fs::write(&sp, doc.to_string()).unwrap();
    std::fs::write(&p, format!("{}{}", lines.concat(), user(&cwd, "more"))).unwrap();
    assert!(cached_digest(state.path(), "sid", &p)
        .facets
        .work
        .contains(&"repo:planted/marker".to_string()));
}

#[test]
fn a_half_written_last_line_waits_for_the_next_refresh() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let p = home.path().join("s.jsonl");
    let full = assistant_text(home.path(), "see https://github.com/a/b/pull/1");
    let (head, tail) = full.split_at(full.len() / 2);
    std::fs::write(&p, head).unwrap();
    assert!(cached_digest(state.path(), "sid", &p)
        .facets
        .mention
        .is_empty());
    std::fs::write(&p, format!("{head}{tail}")).unwrap();
    assert!(cached_digest(state.path(), "sid", &p)
        .facets
        .mention
        .contains(&"pr:a/b#1".to_string()));
}

#[test]
fn a_rewritten_shorter_file_and_an_old_spark_only_cache_are_rebuilt() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let p = home.path().join("s.jsonl");

    // A pre-facets cache with a matching stamp is not trusted: it has no facets.
    std::fs::write(&p, user(home.path(), "https://github.com/a/b/issues/2")).unwrap();
    let m = std::fs::metadata(&p).unwrap();
    let mtime = m
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    std::fs::write(
        state.path().join("sid.json"),
        json!({"source_mtime": mtime, "source_len": m.len(), "spark": [5], "total": 5}).to_string(),
    )
    .unwrap();
    let d = cached_digest(state.path(), "sid", &p);
    assert_eq!(d.spark, Vec::<u32>::new());
    assert!(d.facets.mention.contains(&"issue:a/b#2".to_string()));

    std::fs::write(&p, user(home.path(), "x")).unwrap();
    assert!(cached_digest(state.path(), "sid", &p)
        .facets
        .mention
        .is_empty());
}
