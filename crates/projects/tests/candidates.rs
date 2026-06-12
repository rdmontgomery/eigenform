use std::fs;
use std::path::PathBuf;

use eigenform_projects::{immediate_subdirs, merge_candidates, Candidate};
use tempfile::tempdir;

#[test]
fn immediate_subdirs_returns_only_dirs_sorted_by_name() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("zebra")).unwrap();
    fs::create_dir(root.path().join("apple")).unwrap();
    fs::write(root.path().join("loose-file.txt"), "x").unwrap();

    let subdirs = immediate_subdirs(root.path()).unwrap();

    assert_eq!(
        subdirs,
        vec![root.path().join("apple"), root.path().join("zebra")]
    );
}

#[test]
fn immediate_subdirs_errors_when_root_missing() {
    let root = tempdir().unwrap();
    let missing = root.path().join("does-not-exist");
    assert!(immediate_subdirs(&missing).is_err());
}

#[test]
fn merge_candidates_recents_first_then_unseen_subdirs() {
    let recents = vec![PathBuf::from("/home/u/projects/eigen"), PathBuf::from("/tmp/scratch")];
    let subdirs = vec![
        PathBuf::from("/home/u/projects/eigen"), // already a recent — must not duplicate
        PathBuf::from("/home/u/projects/woland"),
    ];

    let merged = merge_candidates(&recents, &subdirs);

    assert_eq!(
        merged,
        vec![
            Candidate { path: PathBuf::from("/home/u/projects/eigen"), recent: true },
            Candidate { path: PathBuf::from("/tmp/scratch"), recent: true },
            Candidate { path: PathBuf::from("/home/u/projects/woland"), recent: false },
        ]
    );
}

#[test]
fn merge_candidates_dedups_repeated_recents_keeping_first_order() {
    let recents = vec![
        PathBuf::from("/a"),
        PathBuf::from("/b"),
        PathBuf::from("/a"), // duplicate recent
    ];
    let merged = merge_candidates(&recents, &[]);

    assert_eq!(
        merged,
        vec![
            Candidate { path: PathBuf::from("/a"), recent: true },
            Candidate { path: PathBuf::from("/b"), recent: true },
        ]
    );
}
