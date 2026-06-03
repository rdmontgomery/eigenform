//! The pty bridge unit, driven against dummy commands (no claude, no tokens).

use std::io::Read;

use eigen_daemon::Pty;

/// Read until `needle` appears or the stream ends. Bounded so a wedged child can't hang
/// the test forever.
fn read_until(reader: &mut dyn Read, needle: &str) -> String {
    let mut acc = Vec::new();
    let mut buf = [0u8; 1024];
    for _ in 0..200 {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&acc).contains(needle) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&acc).into_owned()
}

#[test]
fn pty_streams_command_output() {
    let pty = Pty::spawn("printf", &["hello-pty"], None, (80, 24)).expect("spawn");
    let mut reader = pty.reader().expect("reader");
    let out = read_until(&mut reader, "hello-pty");
    assert!(out.contains("hello-pty"), "got: {out:?}");
}

#[test]
fn pty_forwards_input_to_the_child() {
    let mut pty = Pty::spawn("cat", &[], None, (80, 24)).expect("spawn");
    pty.write_input(b"ping\n").expect("write");
    let mut reader = pty.reader().expect("reader");
    // cat echoes the line back (pty also echoes input); either way "ping" appears.
    let out = read_until(&mut reader, "ping");
    assert!(out.contains("ping"), "got: {out:?}");
}

#[test]
fn pty_resize_succeeds() {
    let pty = Pty::spawn("cat", &[], None, (80, 24)).expect("spawn");
    pty.resize(100, 40).expect("resize");
}
