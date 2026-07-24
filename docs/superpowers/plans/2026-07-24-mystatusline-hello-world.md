# mystatusline Hello World Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust binary `mystatusline` that Claude Code's statusLine hook can call, printing `Hello World`, and wire it up as the active statusLine command.

**Architecture:** A single-binary Cargo crate. `main.rs` reads and discards all of stdin (matching the real statusLine protocol, which always pipes a JSON payload), then writes `Hello World` to stdout. Installed via `cargo install --path .`; Claude Code's `~/.claude/settings.json` is updated to invoke the installed binary by absolute path.

**Tech Stack:** Rust (cargo 1.97.0, already installed), no external crates required.

## Global Constraints

- Crate/binary name: `mystatusline`.
- Repo root: `~/projects/mystatusline` (existing git repo, GitHub remote `kerryhatcher/mystatusline`, Rust-flavored `.gitignore` already present).
- No JSON parsing or config file support in this version — output is always the literal string `Hello World`.
- Install target: `~/.cargo/bin/mystatusline` via `cargo install --path .`.
- `~/.claude/settings.json` must be referenced by **absolute path** (`~/.cargo/bin/mystatusline`), not a bare command name.
- Every other key in `~/.claude/settings.json` must be preserved unchanged; if it is a symlink, edit the link's target file, not the symlink itself.
- The entry being replaced is the current `statusLine` block pointing at Cognee's `cognee-statusline.sh` — this is the only key to change.

---

### Task 1: Cargo project producing Hello World, with a passing integration test

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `tests/cli.rs`

**Interfaces:**
- Produces: a binary target named `mystatusline`, invocable as `env!("CARGO_BIN_EXE_mystatusline")` in tests, and later at `~/.cargo/bin/mystatusline` once installed (Task 2 depends on this binary existing and behaving correctly).

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "mystatusline"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "mystatusline"
path = "src/main.rs"
```

- [ ] **Step 2: Create a stub `src/main.rs` that does nothing yet**

```rust
fn main() {}
```

- [ ] **Step 3: Write the failing integration test in `tests/cli.rs`**

```rust
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
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test --manifest-path ~/projects/mystatusline/Cargo.toml`
Expected: FAIL — `assertion failed` because stdout is empty, not `"Hello World\n"`.

- [ ] **Step 5: Implement `src/main.rs` to make the test pass**

```rust
use std::io::Read;

fn main() {
    let mut discard = String::new();
    let _ = std::io::stdin().read_to_string(&mut discard);
    println!("Hello World");
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --manifest-path ~/projects/mystatusline/Cargo.toml`
Expected: PASS — `test reads_stdin_and_prints_hello_world ... ok`.

- [ ] **Step 7: Commit**

```bash
cd ~/projects/mystatusline
git add Cargo.toml src/main.rs tests/cli.rs Cargo.lock
git commit -m "feat: add mystatusline binary that prints Hello World"
```

---

### Task 2: Install the binary and wire it into Claude Code's statusLine

**Files:**
- Modify: `~/.claude/settings.json:statusLine` (outside the repo — not a git-tracked change in this project)
- Modify: `~/projects/mystatusline/README.md`

**Interfaces:**
- Consumes: the `mystatusline` binary target produced by Task 1.

- [ ] **Step 1: Install the binary**

Run: `cargo install --path ~/projects/mystatusline`
Expected: output ends with `Installing ~/.cargo/bin/mystatusline` / `Installed package \`mystatusline v0.1.0\` (executable \`mystatusline\`)`.

- [ ] **Step 2: Manually verify the installed binary's behavior**

Run: `echo '{}' | ~/.cargo/bin/mystatusline`
Expected: prints exactly `Hello World`.

- [ ] **Step 3: Check whether `~/.claude/settings.json` is a symlink**

Run: `ls -la ~/.claude/settings.json`
Expected: note the output. If it shows `->`, resolve and edit the link's target file in all following steps instead of the symlink path.

- [ ] **Step 4: Update the `statusLine` entry in `~/.claude/settings.json`**

Using the Edit tool, change only this block (preserving every other key in the file):

```json
  "statusLine": {
    "type": "command",
    "command": "/home/kwhatcher/.claude/plugins/cache/cognee/cognee-memory/0.2.0/scripts/cognee-statusline.sh"
  },
```

to:

```json
  "statusLine": {
    "type": "command",
    "command": "/home/kwhatcher/.cargo/bin/mystatusline"
  },
```

- [ ] **Step 5: Verify the settings file is still valid JSON**

Run: `python3 -c "import json; json.load(open('/home/kwhatcher/.claude/settings.json'))" && echo OK`
Expected: `OK`

- [ ] **Step 6: Update `README.md` with build/install/wiring instructions**

```markdown
# mystatusline
A tool for custom claude status line

## Build & install

```bash
cargo install --path .
```

This installs the binary to `~/.cargo/bin/mystatusline`.

## Wiring into Claude Code

Set the `statusLine` command in `~/.claude/settings.json` (or a project-level
`.claude/settings.json`) to the absolute path of the installed binary:

```json
"statusLine": {
  "type": "command",
  "command": "/home/kwhatcher/.cargo/bin/mystatusline"
}
```

Claude Code reads the statusLine config once at session start, so start a new
session after changing it.
```

- [ ] **Step 7: Commit the README update**

```bash
cd ~/projects/mystatusline
git add README.md
git commit -m "docs: add build, install, and statusLine wiring instructions"
```

- [ ] **Step 8: Tell the user to start a new Claude Code session**

Report back that a new session is required for the statusLine change to take
visible effect, since Claude Code only reads that config at session start.
