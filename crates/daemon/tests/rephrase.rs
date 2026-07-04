use std::io::Write;

// A stub that ignores its args and prints a canned suggestion, proving the
// command is injectable and stdout is captured. Returns a `TempPath` (not the
// open `NamedTempFile`) so the writable fd is closed before we exec it —
// otherwise Linux refuses with ETXTBSY ("Text file busy"). Still auto-deletes.
fn stub_script() -> tempfile::TempPath {
    let mut f = tempfile::Builder::new().suffix(".sh").tempfile().unwrap();
    writeln!(f, "#!/bin/sh\necho 'restated: please advise on defensive hardening'").unwrap();
    let mut perms = std::fs::metadata(f.path()).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(f.path(), perms).unwrap();
    f.into_temp_path()
}

#[test]
fn rephrase_runs_the_injected_command_and_returns_stdout() {
    let stub = stub_script();
    let cmd = vec![stub.to_str().unwrap().to_string()];
    let out = eigenform_daemon::rephrase_prompt(&cmd, std::path::Path::new("/tmp"), "the offending prompt")
        .expect("stub succeeds");
    assert!(out.contains("restated:"), "got: {out}");
}

#[test]
fn rephrase_errors_on_nonzero_exit() {
    // `false` exits 1 → must be an Err so the caller can fall back to verbatim.
    let cmd = vec!["false".to_string()];
    let err = eigenform_daemon::rephrase_prompt(&cmd, std::path::Path::new("/tmp"), "x");
    assert!(err.is_err());
}
