# mystatusline `setup` Subcommand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `mystatusline setup [--project]` subcommand that points Claude Code's `statusLine.command` at the currently-running binary, replacing the manual JSON-editing steps in the README.

**Architecture:** A new `src/setup.rs` module owns the settings-file read/mutate/write logic behind a pure, path-parameterized function (unit-testable against tempdirs). `src/main.rs` grows a thin argument dispatch in front of its existing stdin-render logic, which stays completely unchanged for the no-args case.

**Tech Stack:** Rust (existing `mystatusline` crate), `serde_json` (already a dependency — gains the `preserve_order` feature), `tempfile` (already a dev-dependency) for tests.

## Global Constraints

- No `clap` or any CLI-parsing crate — manual `std::env::args()` matching for the two recognized subcommand shapes.
- No interactive confirmation prompt — invoking `setup` is the user's confirmation.
- `setup --project` always targets `.claude/settings.local.json` under the current directory — never the shared, typically-committed `.claude/settings.json`.
- If the target settings file exists but fails to parse as JSON, or parses to a non-object JSON value, abort **without writing anything** and report a clear error — never blind-overwrite a file whose structure isn't understood.
- No special-case symlink-handling code — `fs::write`/`File::create` already follow symlinks on Unix by default.
- `serde_json` must have its `preserve_order` feature enabled so rewriting the settings file preserves the user's existing key order instead of resorting it alphabetically.
- The no-args statusline-render path must remain byte-for-byte unchanged — Claude Code's `statusLine` hook never passes CLI arguments, and all existing tests exercising that path must keep passing unmodified.

---

### Task 1: `setup` module — settings-file read/mutate/write logic

**Files:**
- Modify: `Cargo.toml` (change the `serde_json = "1"` line under `[dependencies]` to `serde_json = { version = "1", features = ["preserve_order"] }`)
- Create: `src/setup.rs`
- Modify: `src/main.rs` (add `mod setup;` between the existing `mod payload;` and `mod todo;` lines — alphabetical order, matching this file's existing convention)

**Interfaces:**
- Consumes: nothing beyond `serde_json` (already a dependency).
- Produces: `pub fn setup::run(project_scope: bool) -> Result<(), String>` — resolves the settings path for the given scope, resolves the running binary's path via `env::current_exe()`, applies the update, and prints the before/after report described in the spec. Returns `Err(message)` on any failure (nothing printed, no file written on error paths). Task 2 (not yours) wires this into `main()`'s argument dispatch — it's normal and expected that `cargo build`/`cargo test` will show a dead-code warning for `run` until then.

- [ ] **Step 1: Write the failing tests**

First, edit `Cargo.toml`'s `[dependencies]` section so the `serde_json` line reads:

```toml
serde_json = { version = "1", features = ["preserve_order"] }
```

Then create `src/setup.rs`:

```rust
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn resolve_settings_path(project_scope: bool) -> PathBuf {
    if project_scope {
        env::current_dir()
            .unwrap_or_default()
            .join(".claude")
            .join("settings.local.json")
    } else {
        let home = env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".claude").join("settings.json")
    }
}

fn apply_statusline_update(_settings_path: &Path, _new_command: &str) -> Result<Option<String>, String> {
    todo!()
}

pub fn run(_project_scope: bool) -> Result<(), String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_settings_file_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let previous = apply_statusline_update(&path, "/usr/local/bin/mystatusline").unwrap();

        assert_eq!(previous, None);
        let contents = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            value["statusLine"],
            json!({"type": "command", "command": "/usr/local/bin/mystatusline"})
        );
    }

    #[test]
    fn preserves_unrelated_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"env":{"FOO":"bar"},"theme":"dark"}"#).unwrap();

        apply_statusline_update(&path, "/bin/mystatusline").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["env"]["FOO"], "bar");
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["statusLine"]["command"], "/bin/mystatusline");
    }

    #[test]
    fn captures_previous_statusline_command() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"statusLine":{"type":"command","command":"/old/path"}}"#,
        )
        .unwrap();

        let previous = apply_statusline_update(&path, "/new/path").unwrap();

        assert_eq!(previous, Some("/old/path".to_string()));
    }

    #[test]
    fn rejects_invalid_json_without_modifying_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = "{not valid json";
        fs::write(&path, original).unwrap();

        let result = apply_statusline_update(&path, "/bin/mystatusline");

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn rejects_non_object_root_without_modifying_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = "[1, 2, 3]";
        fs::write(&path, original).unwrap();

        let result = apply_statusline_update(&path, "/bin/mystatusline");

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn preserves_key_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"zeta":1,"alpha":2,"mid":3}"#).unwrap();

        apply_statusline_update(&path, "/bin/mystatusline").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let zeta_pos = contents.find("zeta").unwrap();
        let alpha_pos = contents.find("alpha").unwrap();
        let mid_pos = contents.find("mid").unwrap();
        assert!(zeta_pos < alpha_pos);
        assert!(alpha_pos < mid_pos);
    }
}
```

Add `mod setup;` to `src/main.rs`, positioned alphabetically:

```rust
mod context_bar;
mod layout;
mod payload;
mod setup;
mod todo;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test setup`
Expected: compiles, then every test panics with `not yet implemented` (from the `todo!()` in `apply_statusline_update`).

- [ ] **Step 3: Implement `apply_statusline_update` and `run`**

Replace the `todo!()` bodies in `src/setup.rs`:

```rust
fn apply_statusline_update(settings_path: &Path, new_command: &str) -> Result<Option<String>, String> {
    let existing = match fs::read_to_string(settings_path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(e) => return Err(format!("failed to read {}: {e}", settings_path.display())),
    };

    let mut root: Value = serde_json::from_str(&existing)
        .map_err(|e| format!("{} contains invalid JSON: {e}", settings_path.display()))?;

    let map = root.as_object_mut().ok_or_else(|| {
        format!(
            "{} does not contain a JSON object at its root",
            settings_path.display()
        )
    })?;

    let previous = map
        .get("statusLine")
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    map.insert(
        "statusLine".to_string(),
        json!({ "type": "command", "command": new_command }),
    );

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    let serialized = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    fs::write(settings_path, serialized)
        .map_err(|e| format!("failed to write {}: {e}", settings_path.display()))?;

    Ok(previous)
}

pub fn run(project_scope: bool) -> Result<(), String> {
    let settings_path = resolve_settings_path(project_scope);
    let new_command = env::current_exe()
        .map_err(|e| format!("failed to resolve the current executable path: {e}"))?
        .to_string_lossy()
        .into_owned();

    let previous = apply_statusline_update(&settings_path, &new_command)?;

    println!("Updated statusLine in {}", settings_path.display());
    match previous {
        Some(before) => println!("  before: {before}"),
        None => println!("  before: (none)"),
    }
    println!("  after:  {new_command}");
    println!("Start a new Claude Code session for the change to take effect.");

    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test setup`
Expected: all 6 tests in `setup::tests` PASS. You'll see a `dead_code` warning for `run` — expected until Task 2 wires it into `main()`'s dispatch; not a failure.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/setup.rs src/main.rs
git commit -m "feat: add setup module for updating statusLine settings"
```

---

### Task 2: Wire `setup` into `main.rs`'s argument dispatch, add integration tests

**Files:**
- Modify: `src/main.rs` (add argument dispatch at the top of `main()`; the rest of `main()`'s body is untouched)
- Modify: `tests/cli.rs` (add a `run_command` helper plus 3 new integration tests)

**Interfaces:**
- Consumes: `setup::run(project_scope: bool) -> Result<(), String>` (Task 1) — exact signature as declared.
- Produces: the `mystatusline` binary's final CLI contract (no further consumers in this plan).

- [ ] **Step 1: Write the failing integration tests**

Add these imports to the top of `tests/cli.rs` (alongside whatever's already there):

```rust
use std::path::Path;
```

Append these to the end of `tests/cli.rs`:

```rust
fn run_command(args: &[&str], envs: &[(&str, &str)], cwd: Option<&Path>) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_mystatusline");
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
    let mut child = cmd.spawn().expect("failed to spawn mystatusline");
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
        env!("CARGO_BIN_EXE_mystatusline")
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
        env!("CARGO_BIN_EXE_mystatusline")
    );
}

#[test]
fn unknown_subcommand_exits_nonzero_without_hanging() {
    let output = run_command(&["badsubcommand"], &[], None);

    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test cli setup_writes_user_level_settings_file setup_project_writes_local_settings_file unknown_subcommand_exits_nonzero_without_hanging`
Expected: FAIL (or hang, if `main()` still unconditionally reads stdin before the dispatch exists — if a test appears to hang, interrupt it; that itself confirms the dispatch is missing). `cargo test` will report the current `main()` behaves as if `setup`/`badsubcommand` were statusline-render invocations.

- [ ] **Step 3: Add argument dispatch to `main()`**

In `src/main.rs`, add this dispatch as the very first thing inside `fn main()`, before the existing `let mut input = String::new();` line (which stays exactly as it is, along with everything after it):

```rust
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [cmd] if cmd == "setup" => {
            if let Err(e) = setup::run(false) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        [cmd, flag] if cmd == "setup" && flag == "--project" => {
            if let Err(e) = setup::run(true) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        _ => {
            eprintln!("Usage: mystatusline [setup [--project]]");
            std::process::exit(1);
        }
    }

    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    // ... rest of the existing function body is unchanged from here down
}
```

- [ ] **Step 4: Run the full test suite to verify everything passes**

Run: `cargo test`
Expected: every test passes, including all pre-existing tests (the no-args statusline-render path is untouched, so this run also serves as the regression check that dispatch didn't break the default behavior) and the 3 new integration tests from Step 1.

- [ ] **Step 5: Update the README**

In `README.md`, replace the manual "Set the `statusLine` command in `~/.claude/settings.json`..." instructions (the JSON snippet showing how to hand-edit the config) with a pointer to the new subcommand:

```markdown
Set the `statusLine` command automatically:

```bash
mystatusline setup
```

This updates `~/.claude/settings.json` (preserving every other setting) to
point `statusLine.command` at this binary's installed location. Use
`mystatusline setup --project` instead to write `.claude/settings.local.json`
in the current project directory rather than your user-level settings.
```

Keep the rest of the README (the prerequisite/build steps, the machine-specific-path note, the "start a new session" note) — only replace the manual-editing instructions with the above.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/cli.rs README.md
git commit -m "feat: wire setup subcommand into main, document it in README"
```
