# mystatusline — Rust Core Port Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `mystatusline`'s `Hello World` placeholder with a real statusline renderer covering model name, context-usage bar, active todo task, and directory name — a faithful subset of `~/projects/status-line/bin/statusline.py`.

**Architecture:** Four small, independently-testable modules (`payload`, `layout`, `context_bar`, `todo`) each with colocated unit tests, wired together in `main.rs` in the final task. `main.rs` reads stdin, parses JSON, and silently produces empty output on any error — no panics, no stderr.

**Tech Stack:** Rust (existing `mystatusline` crate), `serde` + `serde_json` for JSON (runtime deps), `tempfile` + `filetime` (dev-deps only, for deterministic filesystem-based tests).

## Global Constraints

- Runtime dependencies limited to `serde` (with `derive` feature) and `serde_json` — no YAML, regex, or datetime crate (spec: Dependencies).
- `$HOME` resolution via `std::env::var("HOME")` directly — no `dirs` crate (spec: Dependencies).
- Final stdout output has **no trailing newline** — use `print!`, never `println!` (spec: In scope / Risks).
- Every module degrades silently on malformed/missing input: return `None`/empty `String`, never panic, never write to stderr (spec: Error handling).
- Output formatting (ANSI codes, separators, spacing) must exactly match the Python original's "end" layout for every in-scope element (spec: In scope, Data flow).

---

### Task 1: `payload` module — stdin JSON shape + defaults

**Files:**
- Modify: `Cargo.toml` (add `serde = { version = "1", features = ["derive"] }` and `serde_json = "1"` under `[dependencies]`)
- Create: `src/payload.rs`
- Modify: `src/main.rs:1` (add `mod payload;` as the first line; do not change any other existing behavior yet — the current stdin-drain-then-print-"Hello World" logic stays as-is until Task 5)

**Interfaces:**
- Consumes: nothing (first module).
- Produces: `pub struct payload::Payload` (implements `serde::Deserialize`), with:
  - `pub fn model_name(&self) -> String`
  - `pub fn cwd(&self, fallback: &str) -> String`
  - `pub fn session_id(&self) -> String`
  - `pub fn remaining_percentage(&self) -> Option<f64>`
  - `pub fn total_tokens(&self) -> f64`

- [ ] **Step 1: Write the failing tests**

Create `src/payload.rs` with only the struct definitions and a test module (no accessor method bodies yet — write them as `todo!()` so the crate compiles enough for `cargo test` to report failures, not compile errors):

```rust
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct Payload {
    #[serde(default)]
    pub model: Option<Model>,
    #[serde(default)]
    pub workspace: Option<Workspace>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub context_window: Option<ContextWindow>,
}

#[derive(Deserialize, Default)]
pub struct Model {
    pub display_name: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct Workspace {
    pub current_dir: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ContextWindow {
    pub remaining_percentage: Option<f64>,
    pub total_tokens: Option<f64>,
}

impl Payload {
    pub fn model_name(&self) -> String {
        todo!()
    }

    pub fn cwd(&self, _fallback: &str) -> String {
        todo!()
    }

    pub fn session_id(&self) -> String {
        todo!()
    }

    pub fn remaining_percentage(&self) -> Option<f64> {
        todo!()
    }

    pub fn total_tokens(&self) -> f64 {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_name_defaults_to_claude_when_missing() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.model_name(), "Claude");
    }

    #[test]
    fn model_name_defaults_to_claude_when_empty_string() {
        let payload: Payload =
            serde_json::from_str(r#"{"model":{"display_name":""}}"#).unwrap();
        assert_eq!(payload.model_name(), "Claude");
    }

    #[test]
    fn model_name_uses_display_name_when_present() {
        let payload: Payload =
            serde_json::from_str(r#"{"model":{"display_name":"Sonnet"}}"#).unwrap();
        assert_eq!(payload.model_name(), "Sonnet");
    }

    #[test]
    fn cwd_falls_back_when_missing() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.cwd("/fallback"), "/fallback");
    }

    #[test]
    fn cwd_uses_workspace_current_dir_when_present() {
        let payload: Payload =
            serde_json::from_str(r#"{"workspace":{"current_dir":"/tmp/foo"}}"#).unwrap();
        assert_eq!(payload.cwd("/fallback"), "/tmp/foo");
    }

    #[test]
    fn session_id_defaults_to_empty_string() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.session_id(), "");
    }

    #[test]
    fn remaining_percentage_none_when_missing() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.remaining_percentage(), None);
    }

    #[test]
    fn remaining_percentage_present() {
        let payload: Payload = serde_json::from_str(
            r#"{"context_window":{"remaining_percentage":42.5}}"#,
        )
        .unwrap();
        assert_eq!(payload.remaining_percentage(), Some(42.5));
    }

    #[test]
    fn total_tokens_defaults_to_one_million() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.total_tokens(), 1_000_000.0);
    }

    #[test]
    fn total_tokens_uses_value_when_present() {
        let payload: Payload =
            serde_json::from_str(r#"{"context_window":{"total_tokens":50000}}"#).unwrap();
        assert_eq!(payload.total_tokens(), 50_000.0);
    }

    #[test]
    fn total_tokens_falls_back_when_zero() {
        let payload: Payload =
            serde_json::from_str(r#"{"context_window":{"total_tokens":0}}"#).unwrap();
        assert_eq!(payload.total_tokens(), 1_000_000.0);
    }
}
```

Also add the two dependency lines to `Cargo.toml`'s `[dependencies]` table (create the table if it doesn't exist yet).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib payload`
Expected: compiles, then every test panics with `not yet implemented` (from the `todo!()` bodies).

- [ ] **Step 3: Implement the accessor methods**

Replace each `todo!()` body in `src/payload.rs`:

```rust
impl Payload {
    pub fn model_name(&self) -> String {
        self.model
            .as_ref()
            .and_then(|m| m.display_name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Claude".to_string())
    }

    pub fn cwd(&self, fallback: &str) -> String {
        self.workspace
            .as_ref()
            .and_then(|w| w.current_dir.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.to_string())
    }

    pub fn session_id(&self) -> String {
        self.session_id.clone().unwrap_or_default()
    }

    pub fn remaining_percentage(&self) -> Option<f64> {
        self.context_window
            .as_ref()
            .and_then(|c| c.remaining_percentage)
    }

    pub fn total_tokens(&self) -> f64 {
        self.context_window
            .as_ref()
            .and_then(|c| c.total_tokens)
            .filter(|&t| t > 0.0)
            .unwrap_or(1_000_000.0)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib payload`
Expected: all tests in the `payload::tests` module PASS. (You'll see unrelated `dead_code`/unused warnings for `Payload` and its methods — expected until Task 5 wires them into `main`; not a failure.)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/payload.rs src/main.rs
git commit -m "feat: add payload module for stdin JSON parsing"
```

---

### Task 2: `layout` module — ANSI constants + line composition

**Files:**
- Create: `src/layout.rs`
- Modify: `src/main.rs:2` (add `mod layout;` on the next line after `mod payload;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub const layout::DIM: &str`, `BOLD`, `RESET`, `GREEN`, `YELLOW`, `ORANGE`, `BLINK_RED` (all `&'static str` ANSI escape sequences)
  - `pub fn layout::compose_statusline(model: &str, ctx: &str, task: Option<&str>, dirname: &str) -> String`

- [ ] **Step 1: Write the failing tests**

Create `src/layout.rs`:

```rust
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const ORANGE: &str = "\x1b[38;5;208m";
pub const BLINK_RED: &str = "\x1b[5;31m";

pub fn compose_statusline(_model: &str, _ctx: &str, _task: Option<&str>, _dirname: &str) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_without_task_or_ctx() {
        let out = compose_statusline("Claude", "", None, "myproject");
        assert_eq!(out, format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET}"));
    }

    #[test]
    fn composes_with_task_no_ctx() {
        let out = compose_statusline("Claude", "", Some("Fix bug"), "myproject");
        assert_eq!(
            out,
            format!("{DIM}Claude{RESET} │ {BOLD}Fix bug{RESET} │ {DIM}myproject{RESET}")
        );
    }

    #[test]
    fn composes_with_ctx_no_task() {
        let ctx = format!(" {GREEN}████░░░░░░ 42%{RESET}");
        let out = compose_statusline("Claude", &ctx, None, "myproject");
        assert_eq!(
            out,
            format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET} {DIM}│{RESET}{ctx}")
        );
    }

    #[test]
    fn composes_with_task_and_ctx() {
        let ctx = format!(" {GREEN}████░░░░░░ 42%{RESET}");
        let out = compose_statusline("Claude", &ctx, Some("Fix bug"), "myproject");
        assert_eq!(
            out,
            format!(
                "{DIM}Claude{RESET} │ {BOLD}Fix bug{RESET} │ {DIM}myproject{RESET} {DIM}│{RESET}{ctx}"
            )
        );
    }

    #[test]
    fn empty_task_treated_as_no_task() {
        let out = compose_statusline("Claude", "", Some(""), "myproject");
        assert_eq!(out, format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET}"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib layout`
Expected: compiles, then every test panics with `not yet implemented`.

- [ ] **Step 3: Implement `compose_statusline`**

```rust
pub fn compose_statusline(model: &str, ctx: &str, task: Option<&str>, dirname: &str) -> String {
    let model_seg = format!("{DIM}{model}{RESET}");
    let dir_seg = format!("{DIM}{dirname}{RESET}");
    let ctx_seg = if ctx.is_empty() {
        String::new()
    } else {
        format!(" {DIM}│{RESET}{ctx}")
    };
    match task {
        Some(t) if !t.is_empty() => {
            format!("{model_seg} │ {BOLD}{t}{RESET} │ {dir_seg}{ctx_seg}")
        }
        _ => format!("{model_seg} │ {dir_seg}{ctx_seg}"),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib layout`
Expected: all tests in the `layout::tests` module PASS.

- [ ] **Step 5: Commit**

```bash
git add src/layout.rs src/main.rs
git commit -m "feat: add layout module for statusline composition"
```

---

### Task 3: `context_bar` module — usage percentage + colored bar

**Files:**
- Create: `src/context_bar.rs`
- Modify: `src/main.rs:3` (add `mod context_bar;` on the next line after `mod layout;`)

**Interfaces:**
- Consumes: `layout::{GREEN, YELLOW, ORANGE, BLINK_RED, RESET}` (from Task 2).
- Produces:
  - `pub fn context_bar::compute_used(remaining_percentage: f64, total_tokens: f64, acw_env: f64) -> u8`
  - `pub fn context_bar::render(remaining_percentage: Option<f64>, total_tokens: f64, acw_env: f64) -> String`

- [ ] **Step 1: Write the failing tests**

Create `src/context_bar.rs`:

```rust
use crate::layout::{BLINK_RED, GREEN, ORANGE, RESET, YELLOW};

pub fn compute_used(_remaining_percentage: f64, _total_tokens: f64, _acw_env: f64) -> u8 {
    todo!()
}

fn render_bar(_used: u8) -> String {
    todo!()
}

pub fn render(_remaining_percentage: Option<f64>, _total_tokens: f64, _acw_env: f64) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_empty_when_no_remaining_percentage() {
        assert_eq!(render(None, 1_000_000.0, 0.0), "");
    }

    #[test]
    fn compute_used_full_remaining_is_zero_used() {
        assert_eq!(compute_used(100.0, 1_000_000.0, 0.0), 0);
    }

    #[test]
    fn compute_used_at_buffer_boundary_is_full() {
        assert_eq!(compute_used(16.5, 1_000_000.0, 0.0), 100);
    }

    #[test]
    fn compute_used_below_buffer_clamps_to_100() {
        assert_eq!(compute_used(0.0, 1_000_000.0, 0.0), 100);
    }

    #[test]
    fn compute_used_honors_acw_env_override() {
        // total=1000, acw=500 -> buffer_pct = (1 - 500/1000) * 100 = 50
        // remaining=75 -> usable=(75-50)/(100-50)*100=50 -> used=50
        assert_eq!(compute_used(75.0, 1000.0, 500.0), 50);
    }

    #[test]
    fn render_green_below_50() {
        let out = render(Some(100.0), 1_000_000.0, 0.0); // used=0
        assert!(out.contains(GREEN));
        assert!(out.contains("░░░░░░░░░░ 0%"));
    }

    #[test]
    fn render_yellow_between_50_and_65() {
        // used=50 -> remaining = 16.5 + 50*0.835 = 58.25
        let out = render(Some(58.25), 1_000_000.0, 0.0);
        assert!(out.contains(YELLOW));
        assert!(out.contains("50%"));
    }

    #[test]
    fn render_orange_between_65_and_80() {
        // used=70 -> remaining = 16.5 + 30*0.835 = 41.55
        let out = render(Some(41.55), 1_000_000.0, 0.0);
        assert!(out.contains(ORANGE));
        assert!(out.contains("70%"));
    }

    #[test]
    fn render_blink_red_at_80_and_above() {
        let out = render(Some(0.0), 1_000_000.0, 0.0); // used=100
        assert!(out.contains(BLINK_RED));
        assert!(out.contains('💀'));
    }

    #[test]
    fn render_ends_with_reset() {
        let out = render(Some(100.0), 1_000_000.0, 0.0);
        assert!(out.ends_with(RESET));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib context_bar`
Expected: compiles, then every test panics with `not yet implemented`.

- [ ] **Step 3: Implement the functions**

```rust
pub fn compute_used(remaining_percentage: f64, total_tokens: f64, acw_env: f64) -> u8 {
    let buffer_pct = if acw_env > 0.0 {
        ((1.0 - acw_env / total_tokens) * 100.0).clamp(0.0, 100.0)
    } else {
        16.5
    };
    let usable_remaining =
        ((remaining_percentage - buffer_pct) / (100.0 - buffer_pct) * 100.0).max(0.0);
    let used = (100.0 - usable_remaining).round();
    used.clamp(0.0, 100.0) as u8
}

fn render_bar(used: u8) -> String {
    let filled = (used / 10) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
}

pub fn render(remaining_percentage: Option<f64>, total_tokens: f64, acw_env: f64) -> String {
    let Some(remaining) = remaining_percentage else {
        return String::new();
    };
    let used = compute_used(remaining, total_tokens, acw_env);
    let bar = render_bar(used);
    if used < 50 {
        format!(" {GREEN}{bar} {used}%{RESET}")
    } else if used < 65 {
        format!(" {YELLOW}{bar} {used}%{RESET}")
    } else if used < 80 {
        format!(" {ORANGE}{bar} {used}%{RESET}")
    } else {
        format!(" {BLINK_RED}💀 {bar} {used}%{RESET}")
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib context_bar`
Expected: all tests in the `context_bar::tests` module PASS.

- [ ] **Step 5: Commit**

```bash
git add src/context_bar.rs src/main.rs
git commit -m "feat: add context_bar module for context-usage rendering"
```

---

### Task 4: `todo` module — active in-progress task lookup

**Files:**
- Modify: `Cargo.toml` (add `[dev-dependencies]` section with `tempfile = "3"` and `filetime = "0.2"`)
- Create: `src/todo.rs`
- Modify: `src/main.rs:4` (add `mod todo;` on the next line after `mod context_bar;`)

**Interfaces:**
- Consumes: nothing beyond `serde_json` (already a dependency from Task 1).
- Produces: `pub fn todo::active_task(session_id: &str, todos_dir: &Path) -> Option<String>`

- [ ] **Step 1: Write the failing tests**

Add the two dev-dependency lines to `Cargo.toml`. Then create `src/todo.rs`:

```rust
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Deserialize)]
struct TodoItem {
    status: Option<String>,
    #[serde(rename = "activeForm")]
    active_form: Option<String>,
    content: Option<String>,
}

pub fn active_task(_session_id: &str, _todos_dir: &Path) -> Option<String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_todo_file(dir: &Path, name: &str, contents: &str, mtime_secs: i64) {
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        set_file_mtime(&path, FileTime::from_unix_time(mtime_secs, 0)).unwrap();
    }

    #[test]
    fn none_when_dir_missing() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(active_task("abc", &missing), None);
    }

    #[test]
    fn none_when_session_id_empty() {
        let dir = tempdir().unwrap();
        assert_eq!(active_task("", dir.path()), None);
    }

    #[test]
    fn none_when_no_matching_files() {
        let dir = tempdir().unwrap();
        write_todo_file(dir.path(), "other-session-agent-1.json", "[]", 1000);
        assert_eq!(active_task("abc", dir.path()), None);
    }

    #[test]
    fn ignores_files_missing_agent_marker() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-notit.json",
            r#"[{"status":"in_progress","content":"x"}]"#,
            1000,
        );
        assert_eq!(active_task("abc", dir.path()), None);
    }

    #[test]
    fn picks_newest_file_by_mtime() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-old.json",
            r#"[{"status":"in_progress","content":"old task"}]"#,
            1000,
        );
        write_todo_file(
            dir.path(),
            "abc-agent-new.json",
            r#"[{"status":"in_progress","content":"new task"}]"#,
            2000,
        );
        assert_eq!(active_task("abc", dir.path()), Some("new task".to_string()));
    }

    #[test]
    fn none_when_no_in_progress_entry() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.json",
            r#"[{"status":"completed","content":"done"}]"#,
            1000,
        );
        assert_eq!(active_task("abc", dir.path()), None);
    }

    #[test]
    fn prefers_active_form_over_content() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.json",
            r#"[{"status":"in_progress","activeForm":"Doing thing","content":"do thing"}]"#,
            1000,
        );
        assert_eq!(
            active_task("abc", dir.path()),
            Some("Doing thing".to_string())
        );
    }

    #[test]
    fn falls_back_to_content_when_active_form_empty() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.json",
            r#"[{"status":"in_progress","activeForm":"","content":"do thing"}]"#,
            1000,
        );
        assert_eq!(active_task("abc", dir.path()), Some("do thing".to_string()));
    }

    #[test]
    fn none_when_file_is_malformed_json() {
        let dir = tempdir().unwrap();
        write_todo_file(dir.path(), "abc-agent-1.json", "not json", 1000);
        assert_eq!(active_task("abc", dir.path()), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib todo`
Expected: compiles, then every test panics with `not yet implemented`.

- [ ] **Step 3: Implement `active_task`**

```rust
pub fn active_task(session_id: &str, todos_dir: &Path) -> Option<String> {
    if session_id.is_empty() {
        return None;
    }
    let entries = fs::read_dir(todos_dir).ok()?;

    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(session_id) || !name.contains("-agent-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(mtime) = metadata.modified() else {
            continue;
        };
        if latest.as_ref().map_or(true, |(t, _)| mtime > *t) {
            latest = Some((mtime, entry.path()));
        }
    }

    let (_, path) = latest?;
    let content = fs::read_to_string(path).ok()?;
    let todos: Vec<TodoItem> = serde_json::from_str(&content).ok()?;
    let in_progress = todos
        .into_iter()
        .find(|t| t.status.as_deref() == Some("in_progress"))?;

    in_progress
        .active_form
        .filter(|s| !s.is_empty())
        .or(in_progress.content)
        .filter(|s| !s.is_empty())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib todo`
Expected: all tests in the `todo::tests` module PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/todo.rs src/main.rs
git commit -m "feat: add todo module for active in-progress task lookup"
```

---

### Task 5: Wire `main.rs`, rewrite integration tests, update README

**Files:**
- Modify: `src/main.rs` (full rewrite of `main()`, keeping the four `mod` declarations from Tasks 1-4)
- Modify: `tests/cli.rs` (full rewrite, replacing the v1 hello-world assertions)
- Modify: `README.md:20-24` (the "should print `Hello World`" verification snippet)

**Interfaces:**
- Consumes: `payload::Payload` (Task 1), `layout::compose_statusline` (Task 2), `context_bar::render` (Task 3), `todo::active_task` (Task 4) — exact signatures as declared in those tasks.
- Produces: the `mystatusline` binary's final stdout contract (no further consumers in this plan).

- [ ] **Step 1: Write the failing integration tests**

Replace the entire contents of `tests/cli.rs`:

```rust
use std::fs::{self, File};
use std::io::Write;
use std::process::{Command, Stdio};

fn run_with_env(payload: &str, envs: &[(&str, &str)]) -> String {
    let exe = env!("CARGO_BIN_EXE_mystatusline");
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("failed to spawn mystatusline");
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test cli`
Expected: FAIL — actual stdout is still the v1 `"Hello World\n"` output (or the tests fail to compile if `tempfile` isn't yet a dependency usable from `tests/`; it already is, added in Task 4).

- [ ] **Step 3: Rewrite `main.rs`**

Replace the entire contents of `src/main.rs`:

```rust
mod context_bar;
mod layout;
mod payload;
mod todo;

use payload::Payload;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};

fn resolve_todos_dir() -> PathBuf {
    let claude_dir = match env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home = env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".claude")
        }
    };
    claude_dir.join("todos")
}

fn main() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }

    let Ok(payload) = serde_json::from_str::<Payload>(&input) else {
        return;
    };

    let process_cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cwd = payload.cwd(&process_cwd);
    let model = payload.model_name();
    let session_id = payload.session_id();

    let acw_env: f64 = env::var("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let ctx = context_bar::render(payload.remaining_percentage(), payload.total_tokens(), acw_env);

    let todos_dir = resolve_todos_dir();
    let task = todo::active_task(&session_id, &todos_dir);

    let dirname = Path::new(&cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.clone());

    let output = layout::compose_statusline(&model, &ctx, task.as_deref(), &dirname);
    print!("{output}");
}
```

- [ ] **Step 4: Run all tests to verify they pass**

Run: `cargo test`
Expected: every unit test (`payload`, `layout`, `context_bar`, `todo`) and every integration test in `tests/cli.rs` PASSes. No warnings about unused `pub` items should remain, since `main()` now calls into all four modules.

- [ ] **Step 5: Update README's stale verification snippet**

In `README.md`, replace the "Wiring into Claude Code" verification step (the `echo '{}' | mystatusline` snippet that claims it prints `Hello World`) with:

```markdown
After running `cargo install --path .`, verify the binary works before wiring
it up:

```bash
echo '{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp"}}' | mystatusline
```

This should print a statusline like `Claude │ tmp` (dimmed), reflecting the
model name and directory from the JSON payload. Claude Code sends a much
richer payload at runtime (context window usage, session id, etc.) — see
this repo's `docs/superpowers/specs/` for the full input/output contract.
```

- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/cli.rs README.md
git commit -m "feat: wire statusline modules into main, replace hello-world output"
```
