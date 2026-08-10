use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The config directory under `home`, matching `paths.rs`'s platform
/// resolution. Tests must use this instead of hardcoding `.config/ferrisbar`
/// or `Library/Application Support/ferrisbar`, because the binary resolves
/// differently on each OS and the test's `HOME` is a tempdir.
fn config_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join("ferrisbar")
    }
    #[cfg(windows)]
    {
        home.join("ferrisbar")
    }
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        home.join(".config").join("ferrisbar")
    }
}

/// The data directory under `home`, matching `paths.rs`'s platform
/// resolution. On macOS this is the same as `config_dir`.
fn data_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join("ferrisbar")
    }
    #[cfg(windows)]
    {
        home.join("ferrisbar")
    }
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        home.join(".local").join("share").join("ferrisbar")
    }
}

/// The log file path under `home`.
fn log_file(home: &Path) -> PathBuf {
    data_dir(home).join("logs").join("ferrisbar.jsonl")
}

/// Env vars whose presence in the developer's real shell could route a
/// spawned child into real state instead of a tempdir. `FERRISBAR_LOG_PATH`
/// and `FERRISBAR_LOG_LEVEL` beat the config file outright (`src/main.rs`),
/// so an exported one of these would send the suite's logging to a real
/// path, or suppress the warn record some tests assert on.
///
/// This is the single source of truth for that list — do not inline a
/// second copy of it, since a second copy is exactly what drifted before
/// (the concurrency test and `stdout_is_identical_with_and_without_a_config_file`
/// each grew their own, slightly different, set of removals).
const OVERRIDABLE_ENV_VARS: &[&str] = &[
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "FERRISBAR_LOG_PATH",
    "FERRISBAR_LOG_LEVEL",
    "FERRISBAR_COST_TTL_SECONDS",
];

/// A `Command` rooted at `home` for `HOME`/`APPDATA`/`LOCALAPPDATA`, with
/// every var in `OVERRIDABLE_ENV_VARS` removed. Used by `isolated()` and by
/// the handful of tests that need a second (or Nth) process sharing an
/// already-created home directory.
///
/// `FERRISBAR_COST_TTL_SECONDS=0` disables the daily-cost background
/// refresh by default: nearly every test here is unrelated to the cost
/// feature, and a spawned detached process per render is both slow and a
/// source of cross-run nondeterminism (see
/// `stdout_is_identical_with_and_without_a_config_file`). Tests that
/// exercise the cost feature itself override this var back off explicitly.
fn command_with_home(home: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ferrisbar"));
    cmd.env("HOME", home)
        .env("APPDATA", home)
        .env("LOCALAPPDATA", home);
    for var in OVERRIDABLE_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd.env("FERRISBAR_COST_TTL_SECONDS", "0");
    cmd
}

/// Most tests in this file should get their `Command` from here. The
/// exceptions are tests that deliberately construct their own `Command`
/// because they need an env shape this helper cannot produce — most often,
/// an unresolvable `HOME` (see `an_unresolvable_home_still_renders` and
/// `no_stray_directory_is_created_when_home_is_unset`). Those tests must
/// still remove every var in `OVERRIDABLE_ENV_VARS` themselves.
///
/// The returned `TempDir` must be bound to a named variable (`_home`, not
/// bare `_`) for the duration of the test; bare `_` drops it immediately,
/// deleting the directory before the child process runs.
fn isolated() -> (std::process::Command, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let cmd = command_with_home(home.path());
    (cmd, home)
}

fn run_with_env(payload: &str, envs: &[(&str, &str)]) -> String {
    let (mut cmd, _home) = isolated();
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
fn session_cost_rendered_next_to_context_bar() {
    let payload = r#"{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"context_window":{"remaining_percentage":100.0,"total_tokens":1000000},"cost":{"total_cost_usd":0.4213}}"#;
    let empty_todos = tempfile::tempdir().unwrap();
    let out = run_with_env(
        payload,
        &[("CLAUDE_CONFIG_DIR", empty_todos.path().to_str().unwrap())],
    );
    assert_eq!(
        out,
        "\x1b[2mSonnet\x1b[0m │ \x1b[2mmyproject\x1b[0m \x1b[2m│\x1b[0m \x1b[32m░░░░░░░░░░ 0%\x1b[0m \x1b[2m│\x1b[0m \x1b[2m$0.42\x1b[0m"
    );
}

#[test]
fn session_cost_omitted_when_no_context_window() {
    let payload = r#"{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"cost":{"total_cost_usd":0.42}}"#;
    let empty_todos = tempfile::tempdir().unwrap();
    let out = run_with_env(
        payload,
        &[("CLAUDE_CONFIG_DIR", empty_todos.path().to_str().unwrap())],
    );
    assert!(
        !out.contains('$'),
        "no context bar means nothing to attach the cost chip to"
    );
}

#[test]
fn session_cost_omitted_when_show_session_disabled_in_config() {
    let payload = r#"{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"context_window":{"remaining_percentage":100.0,"total_tokens":1000000},"cost":{"total_cost_usd":0.42}}"#;
    let (mut cmd, home) = isolated();
    let dir = config_dir(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[cost]\nshow_session = false\n").unwrap();

    let (out, ok) = run(&mut cmd, payload);

    assert!(ok);
    assert!(!out.contains('$'));
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
    let (mut cmd, _home) = isolated();
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
    let (mut cmd, _home) = isolated();
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

fn run(cmd: &mut std::process::Command, stdin: &str) -> (String, bool) {
    use std::io::Write as _;
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

const PAYLOAD: &str = r#"{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp/demo"},"session_id":"s1"}"#;

#[test]
fn a_normal_render_creates_the_config_file() {
    let (mut cmd, home) = isolated();
    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Claude"));
    assert!(
        config_dir(home.path()).join("config.toml").exists(),
        "the config file is created on first run"
    );
}

#[test]
fn a_malformed_config_still_renders_and_is_left_untouched() {
    let (mut cmd, home) = isolated();
    let dir = config_dir(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    let original = "not = = toml";
    std::fs::write(dir.join("config.toml"), original).unwrap();

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(
        stdout.contains("Claude"),
        "a broken config must not blank the statusline"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml")).unwrap(),
        original
    );

    let log = log_file(home.path());
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("config_parse_failed"),
        "the failure is diagnosable"
    );
}

#[test]
fn stdout_is_identical_with_and_without_a_config_file() {
    let (mut first, home_a) = isolated();
    let (baseline, _) = run(&mut first, PAYLOAD);

    // Second run: the config file now exists from the first run.
    let mut second = command_with_home(home_a.path());
    let (with_config, _) = run(&mut second, PAYLOAD);

    assert_eq!(
        baseline, with_config,
        "config presence must not alter output"
    );
}

#[test]
fn logging_disabled_creates_no_log_directory() {
    let (mut cmd, home) = isolated();
    let dir = config_dir(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[log]\nenabled = false\n").unwrap();

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Claude"));
    assert!(!data_dir(home.path()).join("logs").exists());
}

/// A `Command` with `HOME`/`APPDATA`/`LOCALAPPDATA` and every var in
/// `OVERRIDABLE_ENV_VARS` removed — the opposite of `command_with_home`,
/// for the tests that deliberately exercise an unresolvable base directory.
fn command_without_home() -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ferrisbar"));
    cmd.env_remove("HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA");
    for var in OVERRIDABLE_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn an_unresolvable_home_still_renders() {
    let mut cmd = command_without_home();

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Claude"));
}

#[test]
fn no_stray_directory_is_created_when_home_is_unset() {
    let scratch = tempfile::tempdir().unwrap();
    let mut cmd = command_without_home();
    cmd.current_dir(scratch.path());

    let (_, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(
        !scratch.path().join("ferrisbar").exists(),
        "an empty HOME must never produce a relative path that lands in the user's repo"
    );
}

#[test]
fn env_var_beats_the_config_file_for_the_claude_dir() {
    let (mut cmd, home) = isolated();
    let dir = config_dir(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "[claude]\nconfig_dir = \"{}\"\n",
            home.path().join("from-file").display()
        ),
    )
    .unwrap();

    // The env var points at a todos dir containing an active task; the
    // file points somewhere empty. The task appearing proves env won.
    let from_env = home.path().join("from-env");
    std::fs::create_dir_all(from_env.join("todos")).unwrap();
    std::fs::write(
        from_env.join("todos").join("s1-agent-s1.json"),
        r#"[{"content":"Ship it","status":"in_progress","activeForm":"Shipping it"}]"#,
    )
    .unwrap();
    cmd.env("CLAUDE_CONFIG_DIR", &from_env);

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(
        stdout.contains("Shipping it"),
        "CLAUDE_CONFIG_DIR must still win"
    );
}

#[test]
fn ferrisbar_log_level_env_var_beats_the_config_file() {
    let (mut cmd, home) = isolated();
    let dir = config_dir(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[log]\nlevel = \"off\"\n").unwrap();
    cmd.env("FERRISBAR_LOG_LEVEL", "debug");

    let (_, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    let log = log_file(home.path());
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("\"event\":\"render\""),
        "FERRISBAR_LOG_LEVEL must override level = \"off\" in the file"
    );
}

/// Duplicates `cost::civil_from_days`/`today_utc_date` for fixture purposes
/// only. `tests/cli.rs` drives the compiled binary as a black box — it has
/// no visibility into `src/cost.rs`'s internals — so a cache fixture that
/// needs to look like "today" to the binary has to compute "today" the same
/// way. The algorithm itself is unit-tested in `src/cost.rs`; this copy only
/// needs to agree with it well enough to seed a fixture.
fn today_utc_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    #[allow(clippy::cast_possible_wrap)]
    let days = (secs as i64).div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    #[allow(clippy::cast_sign_loss)]
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    #[allow(clippy::cast_possible_wrap)]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    #[allow(clippy::cast_possible_truncation)]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[test]
fn daily_cost_chip_reads_a_preseeded_cache() {
    let (mut cmd, home) = isolated();
    cmd.env("FERRISBAR_COST_TTL_SECONDS", "90");
    std::fs::create_dir_all(data_dir(home.path())).unwrap();
    std::fs::write(
        data_dir(home.path()).join("cost-cache.json"),
        format!(
            r#"{{"date":"{}","total_usd":4.2,"by_model":[["claude-sonnet-5",4.2]]}}"#,
            today_utc_date()
        ),
    )
    .unwrap();

    let (out, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(out.contains("$4.20"), "got: {out:?}");
    assert!(out.contains("today"));
    assert!(out.contains("Sonnet"));
    assert!(out.contains('\n'), "the daily total is on its own line");
}

#[test]
fn daily_cost_chip_ignores_a_stale_dated_cache() {
    let (mut cmd, home) = isolated();
    cmd.env("FERRISBAR_COST_TTL_SECONDS", "90");
    std::fs::create_dir_all(data_dir(home.path())).unwrap();
    std::fs::write(
        data_dir(home.path()).join("cost-cache.json"),
        r#"{"date":"2000-01-01","total_usd":4.2,"by_model":[]}"#,
    )
    .unwrap();

    let (out, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(
        !out.contains('\n'),
        "a previous day's cache is not shown: {out:?}"
    );
}

#[test]
fn daily_cost_chip_absent_with_no_cache_and_zero_ttl() {
    // command_with_home() defaults FERRISBAR_COST_TTL_SECONDS to "0".
    let (mut cmd, _home) = isolated();

    let (out, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(!out.contains('\n'));
}

/// Shells out to `id -u` rather than an `unsafe extern "C" geteuid` binding —
/// this is test-only code and the project otherwise has zero unsafe in its
/// dependency-light build.
#[cfg(unix)]
fn running_as_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.trim() == "0")
}

/// spec:500 — a non-writable data directory still renders. Unix-only:
/// Windows has no equivalent of a mode-based read-only directory reachable
/// from a normal test process. Skipped (not asserted vacuously) under root,
/// which bypasses directory permission checks entirely.
#[cfg(unix)]
#[test]
fn a_non_writable_data_directory_still_renders() {
    use std::os::unix::fs::PermissionsExt;

    if running_as_root() {
        eprintln!("skipping: running as root, permission checks do not apply");
        return;
    }

    let (mut cmd, home) = isolated();
    let data_dir = data_dir(home.path());
    std::fs::create_dir_all(&data_dir).unwrap();
    let original_mode = std::fs::metadata(&data_dir).unwrap().permissions().mode();
    std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    // Force an actual write attempt. At the default `warn` level the only
    // event a clean payload generates is the `render` debug event, which
    // Logger::log filters out before ever calling append — so nothing would
    // touch the read-only directory and the chmod above would be inert.
    cmd.env("FERRISBAR_LOG_LEVEL", "debug");

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    // Restore before the tempdir is cleaned up.
    std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(original_mode)).unwrap();

    assert!(ok);
    assert!(
        stdout.contains("Claude"),
        "logging must degrade silently rather than blank the statusline"
    );
}

#[test]
fn ferrisbar_log_level_env_var_re_enables_logging_disabled_in_the_file() {
    let (mut cmd, home) = isolated();
    let dir = config_dir(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[log]\nenabled = false\n").unwrap();
    cmd.env("FERRISBAR_LOG_LEVEL", "debug");

    let (_, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    let log = log_file(home.path());
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("\"event\":\"render\""),
        "FERRISBAR_LOG_LEVEL must be able to re-enable logging even when \
         enabled = false in the file — env beats file, including for the \
         user turning logging back on for one session"
    );
}

#[test]
fn concurrent_renders_never_produce_a_corrupt_archive() {
    // Division of labor with the unit test suite: `O_EXCL` exclusivity of
    // the rotation lock is pinned by `log.rs`'s
    // `a_held_lock_defers_rotation_without_losing_the_line`, which fails
    // under a `create_new` -> `create` mutation. This e2e test cannot detect
    // that same mutation — a losing writer under a broken lock still
    // produces a valid-but-misnumbered archive, and the real race window is
    // microseconds wide against millisecond-scale process spawn — so it is
    // not this test's job to prove exclusivity. Its job is the thing only a
    // multi-process test can show: twelve real processes contending on the
    // same log directory produce at least two genuine rotations, every
    // resulting archive decompresses cleanly, and no lock file is left
    // behind.
    use std::io::{Read as _, Write as _};

    let home = tempfile::tempdir().unwrap();
    let config_dir = config_dir(home.path());
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[log]\nlevel = \"debug\"\nmax_size_bytes = 4096\nmax_archives = 5\n",
    )
    .unwrap();

    // Pre-seed the log past max_size_bytes so the very first render rotates.
    // Without this the test passes trivially with zero archives.
    let logs = data_dir(home.path()).join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let log_file = logs.join("ferrisbar.jsonl");
    std::fs::write(&log_file, format!("{}\n", "s".repeat(8192))).unwrap();

    // A model name long enough that the "render" debug line it produces
    // (which embeds `model={name}`) alone exceeds max_size_bytes. With a
    // short payload only the first process ever rotates (it consumes the
    // pre-seed; the other eleven see a small, freshly-rotated file and
    // never attempt rotation), so `archives` is trivially 1 and the lock
    // sees no real contention. The fat payload makes every rotation
    // re-oversize the log file it recreates, producing several genuine
    // rotation episodes across the 12 processes.
    let fat_payload = format!(
        r#"{{"model":{{"display_name":"{}"}},"workspace":{{"current_dir":"/tmp/demo"}},"session_id":"s1"}}"#,
        "M".repeat(6000)
    );

    let mut children = Vec::new();
    for _ in 0..12 {
        let mut cmd = command_with_home(home.path());
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(fat_payload.as_bytes())
            .unwrap();
        children.push(child);
    }
    for child in children {
        // wait_with_output, not wait: the children have piped stdout, and
        // waiting without draining the pipe can deadlock.
        assert!(child.wait_with_output().unwrap().status.success());
    }

    let mut archives = 0;
    for entry in std::fs::read_dir(&logs).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "gz") {
            archives += 1;
            let mut out = String::new();
            flate2::read::GzDecoder::new(std::fs::File::open(&path).unwrap())
                .read_to_string(&mut out)
                .unwrap_or_else(|e| panic!("{} is corrupt: {e}", path.display()));
        }
    }
    assert!(
        archives >= 2,
        "expected multiple concurrent rotations, got {archives} archive(s) — \
         if this is 1, the processes are not contending and the test is vacuous"
    );
    assert!(
        !logs.join(".rotate.lock").exists(),
        "no lock may be left behind"
    );
}
