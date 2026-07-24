use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn run_with_env(payload: &str, envs: &[(&str, &str)]) -> String {
    let exe = env!("CARGO_BIN_EXE_ferrisbar");
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("failed to spawn ferrisbar");
    child
        .stdin
        .take()
        .expect("child stdin handle")
        .write_all(payload.as_bytes())
        .expect("failed to write to child stdin");
    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn minimal_payload_shows_model_and_dirname_only() {
    let payload = r#"{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"session_id":"sess1"}"#;
    let empty_todos = tempfile::tempdir().unwrap();
    let out = run_with_env(
        payload,
        &[("CLAUDE_CONFIG_DIR", empty_todos.path().to_str().unwrap())],
    );
    assert_eq!(out, "\x1b[2mSonnet\x1b[0m │ \x1b[2mmyproject\x1b[0m");
}

#[test]
fn invalid_json_produces_empty_output() {
    let empty_todos = tempfile::tempdir().unwrap();
    let out = run_with_env(
        "not json",
        &[("CLAUDE_CONFIG_DIR", empty_todos.path().to_str().unwrap())],
    );
    assert_eq!(out, "");
}

#[test]
fn missing_model_defaults_to_claude() {
    let payload = r#"{"workspace":{"current_dir":"/tmp/myproject"}}"#;
    let empty_todos = tempfile::tempdir().unwrap();
    let out = run_with_env(
        payload,
        &[("CLAUDE_CONFIG_DIR", empty_todos.path().to_str().unwrap())],
    );
    assert_eq!(out, "\x1b[2mClaude\x1b[0m │ \x1b[2mmyproject\x1b[0m");
}

#[test]
fn context_bar_rendered_when_context_window_present() {
    let payload = r#"{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"context_window":{"remaining_percentage":100.0,"total_tokens":1000000}}"#;
    let empty_todos = tempfile::tempdir().unwrap();
    let out = run_with_env(
        payload,
        &[("CLAUDE_CONFIG_DIR", empty_todos.path().to_str().unwrap())],
    );
    assert_eq!(
        out,
        "\x1b[2mSonnet\x1b[0m │ \x1b[2mmyproject\x1b[0m \x1b[2m│\x1b[0m \x1b[32m░░░░░░░░░░ 0%\x1b[0m"
    );
}

#[test]
fn active_todo_shown_in_bold() {
    let payload = r#"{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"session_id":"sess42"}"#;
    let todos_root = tempfile::tempdir().unwrap();
    let todos_dir = todos_root.path().join("todos");
    fs::create_dir_all(&todos_dir).unwrap();
    let mut file = File::create(todos_dir.join("sess42-agent-1.json")).unwrap();
    file.write_all(br#"[{"status":"in_progress","activeForm":"Fixing bug"}]"#)
        .unwrap();
    let out = run_with_env(
        payload,
        &[("CLAUDE_CONFIG_DIR", todos_root.path().to_str().unwrap())],
    );
    assert_eq!(
        out,
        "\x1b[2mSonnet\x1b[0m │ \x1b[1mFixing bug\x1b[0m │ \x1b[2mmyproject\x1b[0m"
    );
}

#[test]
fn drains_large_stdin_payload() {
    // Regression coverage for the stdin-drain behavior asserted in the v1
    // hello-world tests (see 08b6841): a large, invalid-JSON payload must
    // still be fully read without the process hanging or erroring, yielding
    // the same "invalid JSON" empty-output result as a small malformed body.
    let empty_todos = tempfile::tempdir().unwrap();
    let payload = "x".repeat(1024 * 1024);
    let out = run_with_env(
        &payload,
        &[("CLAUDE_CONFIG_DIR", empty_todos.path().to_str().unwrap())],
    );
    assert_eq!(out, "");
}

fn run_command(args: &[&str], envs: &[(&str, &str)], cwd: Option<&Path>) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_ferrisbar");
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().expect("failed to spawn ferrisbar");
    drop(child.stdin.take());
    child.wait_with_output().expect("failed to wait on child")
}

#[test]
fn setup_writes_user_level_settings_file() {
    let home = tempfile::tempdir().unwrap();
    let output = run_command(&["setup"], &[("HOME", home.path().to_str().unwrap())], None);

    assert!(output.status.success());
    let settings_path = home.path().join(".claude").join("settings.json");
    let contents = fs::read_to_string(&settings_path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(value["statusLine"]["type"], "command");
    assert_eq!(
        value["statusLine"]["command"].as_str().unwrap(),
        env!("CARGO_BIN_EXE_ferrisbar")
    );
}

#[test]
fn setup_project_writes_local_settings_file() {
    let project_dir = tempfile::tempdir().unwrap();
    let output = run_command(&["setup", "--project"], &[], Some(project_dir.path()));

    assert!(output.status.success());
    let settings_path = project_dir
        .path()
        .join(".claude")
        .join("settings.local.json");
    let contents = fs::read_to_string(&settings_path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(
        value["statusLine"]["command"].as_str().unwrap(),
        env!("CARGO_BIN_EXE_ferrisbar")
    );
}

#[test]
fn unknown_subcommand_exits_nonzero_without_hanging() {
    let output = run_command(&["badsubcommand"], &[], None);

    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
}

#[test]
fn setup_honors_claude_config_dir_over_home() {
    let config_dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let output = run_command(
        &["setup"],
        &[
            ("CLAUDE_CONFIG_DIR", config_dir.path().to_str().unwrap()),
            ("HOME", home.path().to_str().unwrap()),
        ],
        None,
    );

    assert!(output.status.success());

    let settings_path = config_dir.path().join("settings.json");
    let contents = fs::read_to_string(&settings_path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(
        value["statusLine"]["command"].as_str().unwrap(),
        env!("CARGO_BIN_EXE_ferrisbar")
    );

    let bogus_home_settings_path = home.path().join(".claude").join("settings.json");
    assert!(!bogus_home_settings_path.exists());
}

#[test]
fn setup_fails_loudly_when_config_dir_unresolvable() {
    let exe = env!("CARGO_BIN_EXE_ferrisbar");
    let mut cmd = Command::new(exe);
    cmd.args(["setup"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("HOME")
        .env_remove("CLAUDE_CONFIG_DIR");
    let mut child = cmd.spawn().expect("failed to spawn ferrisbar");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("failed to wait on child");

    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
}
