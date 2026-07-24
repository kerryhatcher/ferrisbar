use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn reads_stdin_and_prints_hello_world() {
    let exe = env!("CARGO_BIN_EXE_mystatusline");
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn mystatusline");

    child
        .stdin
        .take()
        .expect("child stdin handle")
        .write_all(br#"{"session_id":"abc","model":{"display_name":"Test"}}"#)
        .expect("failed to write to child stdin");

    let output = child.wait_with_output().expect("failed to wait on child");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello World\n");
}

#[test]
fn drains_large_stdin_payload() {
    let exe = env!("CARGO_BIN_EXE_mystatusline");
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn mystatusline");

    let payload = vec![b'x'; 1024 * 1024];
    child
        .stdin
        .take()
        .expect("child stdin handle")
        .write_all(&payload)
        .expect("child must drain stdin");

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello World\n");
}
