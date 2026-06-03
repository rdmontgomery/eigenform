//! resolve: find a session by uuid (exact or unique prefix) across ALL projects.

use std::path::Path;

use eigen_forest::{resolve, ResolveError};
use tempfile::tempdir;

const A1: &str = "aaaa1111-0000-4000-8000-000000000001";
const A2: &str = "aaaa2222-0000-4000-8000-000000000002";
const B3: &str = "bbbb3333-0000-4000-8000-000000000003";

/// A projects dir with two projects: proj-a has A1+A2, proj-b has B3.
fn fixture() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let root = dir.path();
    for (proj, uuids) in [("-p-a", [A1, A2].as_slice()), ("-p-b", [B3].as_slice())] {
        let pdir = root.join(proj);
        std::fs::create_dir_all(&pdir).unwrap();
        for uuid in uuids {
            let line = format!(
                r#"{{"type":"user","uuid":"{uuid}","cwd":"/home/me/{proj}","sessionId":"{uuid}"}}"#
            );
            std::fs::write(pdir.join(format!("{uuid}.jsonl")), line + "\n").unwrap();
        }
    }
    dir
}

#[test]
fn resolve_exact_uuid_returns_its_path() {
    let dir = fixture();
    let path = resolve(dir.path(), A1).expect("resolve");
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), format!("{A1}.jsonl"));
    assert!(path.exists());
}

#[test]
fn resolve_finds_across_projects() {
    let dir = fixture();
    // B3 lives in a different project than the others; resolve is machine-wide.
    let path = resolve(dir.path(), B3).expect("resolve");
    assert!(Path::new(&path).ends_with(format!("{B3}.jsonl")));
}

#[test]
fn resolve_unique_prefix_works() {
    let dir = fixture();
    let path = resolve(dir.path(), "bbbb").expect("unique prefix");
    assert!(path.to_str().unwrap().contains(B3));
}

#[test]
fn resolve_ambiguous_prefix_lists_candidates() {
    let dir = fixture();
    let err = resolve(dir.path(), "aaaa").unwrap_err();
    match err {
        ResolveError::Ambiguous(candidates) => {
            let mut uuids: Vec<_> = candidates.iter().map(|c| c.uuid.clone()).collect();
            uuids.sort();
            assert_eq!(uuids, vec![A1.to_string(), A2.to_string()]);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn resolve_unknown_is_not_found() {
    let dir = fixture();
    assert!(matches!(
        resolve(dir.path(), "zzzz").unwrap_err(),
        ResolveError::NotFound(_)
    ));
}
