use std::fs;
use std::path::PathBuf;

use eigen_projects::enumerate_projects;
use tempfile::tempdir;

fn write_jsonl_with_cwd(path: &std::path::Path, cwd: &str) {
    fs::write(
        path,
        format!(
            "{{\"type\":\"last-prompt\",\"sessionId\":\"abc\"}}\n\
             {{\"sessionId\":\"abc\",\"cwd\":\"{cwd}\",\"parentUuid\":null}}\n"
        ),
    )
    .unwrap();
}

#[test]
fn enumerate_projects_reads_cwd_from_jsonl() {
    let projects_dir = tempdir().unwrap();
    let pdir = projects_dir.path().join("-tmp-foo-bar");
    fs::create_dir(&pdir).unwrap();
    write_jsonl_with_cwd(&pdir.join("aaa.jsonl"), "/tmp/foo/bar");

    let projects = enumerate_projects(projects_dir.path()).unwrap();

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].dir_name, "-tmp-foo-bar");
    assert_eq!(projects[0].cwd, PathBuf::from("/tmp/foo/bar"));
}

#[test]
fn enumerate_projects_skips_dirs_with_no_jsonl() {
    let projects_dir = tempdir().unwrap();
    fs::create_dir(projects_dir.path().join("-empty-dir")).unwrap();

    let projects = enumerate_projects(projects_dir.path()).unwrap();
    assert!(projects.is_empty());
}

#[test]
fn enumerate_projects_skips_jsonl_with_no_cwd() {
    let projects_dir = tempdir().unwrap();
    let pdir = projects_dir.path().join("-no-cwd");
    fs::create_dir(&pdir).unwrap();
    fs::write(
        pdir.join("ses.jsonl"),
        r#"{"type":"last-prompt","sessionId":"x"}
{"sessionId":"x","parentUuid":null}
"#,
    )
    .unwrap();

    let projects = enumerate_projects(projects_dir.path()).unwrap();
    assert!(projects.is_empty());
}

#[test]
fn project_for_cwd_finds_matching_project() {
    let projects_dir = tempdir().unwrap();
    let pdir = projects_dir.path().join("-tmp-x-y");
    fs::create_dir(&pdir).unwrap();
    write_jsonl_with_cwd(&pdir.join("s.jsonl"), "/tmp/x/y");

    let p = eigen_projects::project_for_cwd(projects_dir.path(), &PathBuf::from("/tmp/x/y"))
        .unwrap()
        .expect("expected to find the matching project");
    assert_eq!(p.dir_name, "-tmp-x-y");
}

#[test]
fn project_for_cwd_returns_none_when_no_match() {
    let projects_dir = tempdir().unwrap();
    let p = eigen_projects::project_for_cwd(projects_dir.path(), &PathBuf::from("/no/such")).unwrap();
    assert!(p.is_none());
}

#[test]
fn enumerate_projects_returns_alphabetic_by_dir_name() {
    let projects_dir = tempdir().unwrap();
    for d in ["-zebra", "-apple", "-mango"] {
        let pdir = projects_dir.path().join(d);
        fs::create_dir(&pdir).unwrap();
        write_jsonl_with_cwd(&pdir.join("s.jsonl"), &format!("/p{d}"));
    }

    let names: Vec<String> = enumerate_projects(projects_dir.path())
        .unwrap()
        .into_iter()
        .map(|p| p.dir_name)
        .collect();
    assert_eq!(names, vec!["-apple", "-mango", "-zebra"]);
}
