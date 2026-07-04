//! Cross-version corpus property test (spike 07). Runs the parser's invariants over
//! whatever real Claude Code sessions a machine has, so schema drift across versions
//! surfaces automatically. Strictly bounded and graceful:
//!
//! - Corpus dir: `EIGENFORM_CORPUS_DIR`, else `~/.claude/projects`. Absent/empty → skip+pass.
//! - Validates at most 64 sessions (newest first) under a 128 MiB budget; logs anything
//!   skipped. `EIGENFORM_CORPUS_FULL=1` lifts the count cap.
//! - Per session: parse succeeds · re-emit byte-identical · guarded swap finds 0 stray ·
//!   the resume leaf resolves to a real row uuid.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use eigenform_surgery::{rewrite_session_id, Session};

const MAX_SESSIONS: usize = 64;
const BYTE_BUDGET: u64 = 128 * 1024 * 1024;
const PER_FILE_MAX: u64 = 32 * 1024 * 1024;
const SENTINEL_ID: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";

fn corpus_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("EIGENFORM_CORPUS_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".claude/projects"))
}

/// All `*.jsonl` under `projects/*/`, with size and mtime for sorting/budgeting.
fn collect_sessions(dir: &PathBuf) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut out = Vec::new();
    let Ok(projects) = fs::read_dir(dir) else {
        return out;
    };
    for project in projects.flatten() {
        let Ok(files) = fs::read_dir(project.path()) else {
            continue;
        };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Ok(meta) = f.metadata() {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((path, meta.len(), mtime));
            }
        }
    }
    out
}

/// Every top-level `uuid` field in the file (turns and opaque rows alike).
fn all_row_uuids(contents: &str) -> HashSet<String> {
    contents
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("uuid")?.as_str().map(str::to_string))
        .collect()
}

#[test]
fn corpus_round_trips_and_guards_cleanly_across_versions() {
    let Some(dir) = corpus_dir() else {
        eprintln!("corpus test skipped: no HOME / EIGENFORM_CORPUS_DIR");
        return;
    };
    if !dir.is_dir() {
        eprintln!("corpus test skipped: {dir:?} is not a directory");
        return;
    }

    let mut sessions = collect_sessions(&dir);
    if sessions.is_empty() {
        eprintln!("corpus test skipped: no .jsonl sessions under {dir:?}");
        return;
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.2)); // newest first

    let cap = if std::env::var("EIGENFORM_CORPUS_FULL").is_ok() {
        usize::MAX
    } else {
        MAX_SESSIONS
    };

    let total_found = sessions.len();
    let mut validated = 0usize;
    let mut skipped_size = 0usize;
    let mut spent: u64 = 0;

    for (path, len, _) in sessions.iter() {
        if validated >= cap {
            break;
        }
        if *len > PER_FILE_MAX {
            skipped_size += 1;
            continue;
        }
        if spent + *len > BYTE_BUDGET {
            break;
        }
        spent += *len;

        let contents = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue, // non-UTF8 / unreadable: not our concern here
        };
        let session = Session::parse_str(&contents).expect("parse must not fail");

        // 1. byte-identical round-trip
        assert_eq!(session.to_jsonl(), contents, "round-trip mismatch in {path:?}");

        // 2. guarded swap finds no stray for the file's own id
        if !session.session_id.is_empty() {
            for line in contents.lines() {
                rewrite_session_id(line, &session.session_id, SENTINEL_ID)
                    .unwrap_or_else(|e| panic!("stray session id in {path:?}: {e}"));
            }
        }

        // 3. resume leaf resolves to a real row uuid
        if let Some(leaf) = session.resume_leaf() {
            assert!(
                all_row_uuids(&contents).contains(&leaf),
                "resume leaf {leaf} does not resolve in {path:?}"
            );
        }

        validated += 1;
    }

    let skipped_cap = total_found.saturating_sub(validated + skipped_size);
    eprintln!(
        "corpus: validated {validated}/{total_found} sessions ({:.1} MiB); \
         skipped {skipped_size} over-size, {skipped_cap} over cap/budget \
         (set EIGENFORM_CORPUS_FULL=1 to lift the count cap)",
        spent as f64 / (1024.0 * 1024.0),
    );
    assert!(validated > 0, "corpus present but nothing validated");
}
