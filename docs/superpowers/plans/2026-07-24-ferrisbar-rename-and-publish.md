# Rename to `ferrisbar` and Publish to crates.io Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement Task 1 task-by-task. The Post-Task Operational Steps section below is executed directly by the controller (not dispatched to a subagent) — see that section for why.

**Goal:** Rename the `mystatusline` crate/binary to `ferrisbar` and publish it to crates.io so it installs anywhere with `cargo install ferrisbar`.

**Architecture:** One mechanical rename task (Cargo.toml, tests, main.rs's usage string, README) verified entirely by the existing 62-test suite — no new tests needed, since renaming doesn't change behavior. Followed by operational steps (GitHub repo rename, dry-run, explicit publish confirmation) that are not code changes and are executed directly, not through subagent dispatch.

**Tech Stack:** Rust (existing `mystatusline`/`ferrisbar` crate), `gh` CLI, `cargo publish`.

## Global Constraints

- The crate/bin rename in `Cargo.toml`, `tests/cli.rs`, and `src/main.rs`, plus the `README.md` update, land in **one commit** — this is one coherent change, not several (per the spec's Data flow section).
- `src/setup.rs`'s test fixture path strings (`"/bin/mystatusline"` etc.) are **not** renamed — they're arbitrary example paths unrelated to the crate's own name.
- The local directory (`~/projects/mystatusline`) and historical `docs/superpowers/` spec/plan documents referencing the old name are **not** renamed or rewritten.
- `cargo publish` (the real one, not `--dry-run`) requires explicit user go-ahead at that exact step — never run it as part of an automated sequence.

---

### Task 1: Rename the crate to `ferrisbar`

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs:36` (the usage-message match arm)
- Modify: `tests/cli.rs` (6 occurrences of `CARGO_BIN_EXE_mystatusline`, plus matching `.expect(...)` message text)
- Modify: `README.md` (full rewrite)

**Interfaces:**
- Consumes: nothing new — this task touches existing code, it doesn't add functionality.
- Produces: nothing new — same public behavior as before, under a new name. No other task in this plan depends on this task's internals (the Post-Task Operational Steps depend on this task being *committed*, not on any specific function signature).

- [ ] **Step 1: Rename the package and binary in `Cargo.toml`**

Replace the entire contents of `Cargo.toml`:

```toml
[package]
name = "ferrisbar"
version = "0.1.0"
edition = "2021"
description = "A Claude Code statusline renderer, written in Rust"
license = "MIT"
repository = "https://github.com/kerryhatcher/ferrisbar"
readme = "README.md"
keywords = ["claude-code", "statusline", "cli"]
categories = ["command-line-utilities"]

[[bin]]
name = "ferrisbar"
path = "src/main.rs"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }

[dev-dependencies]
tempfile = "3"
filetime = "0.2"
```

- [ ] **Step 2: Make the usage message derive the program name from `argv[0]`**

In `src/main.rs`, find the final `_ =>` arm of the dispatch `match` (currently reads `eprintln!("Usage: mystatusline [setup [--project]]"); std::process::exit(1);`) and replace just that arm's body:

```rust
        _ => {
            let program = env::args().next().unwrap_or_default();
            let program_name = Path::new(&program)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "ferrisbar".to_string());
            eprintln!("Usage: {program_name} [setup [--project]]");
            std::process::exit(1);
        }
```

(`Path` is already imported at the top of `src/main.rs` via `use std::path::{Path, PathBuf};` — no new imports needed.)

- [ ] **Step 3: Rename the `CARGO_BIN_EXE_*` references in `tests/cli.rs`**

Cargo generates this env var from the `[[bin]]` target's name, so every occurrence of `env!("CARGO_BIN_EXE_mystatusline")` must become `env!("CARGO_BIN_EXE_ferrisbar")` — there are 6 occurrences in `tests/cli.rs`. Also update the 3 occurrences of `.expect("failed to spawn mystatusline")` to `.expect("failed to spawn ferrisbar")` for consistency (cosmetic, but already being touched). Use a find-and-replace across the file for both strings — every occurrence of `mystatusline` in this file is one of these two patterns, so a blanket replace of `mystatusline` → `ferrisbar` in `tests/cli.rs` is safe and complete.

- [ ] **Step 4: Run the full test suite to verify the rename didn't break anything**

Run: `cargo test`
Expected: all 62 tests pass (51 unit + 11 integration) — the existing suite is the correctness check for this rename; no new tests are needed since no behavior changed.

- [ ] **Step 5: Rewrite `README.md`**

Replace the entire contents of `README.md`:

```markdown
# ferrisbar
A Claude Code statusline renderer, written in Rust.

## Install

```bash
cargo install ferrisbar
```

This installs the binary to `~/.cargo/bin/ferrisbar`.

### Building from source instead

```bash
git clone https://github.com/kerryhatcher/ferrisbar.git
cd ferrisbar
cargo install --path .
```

## Wiring into Claude Code

Prerequisite: a working Rust toolchain (`cargo`/`rustc`) is required either way.

After installing, verify the binary works before wiring it up:

```bash
echo '{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp"}}' | ferrisbar
```

This should print a statusline like `Claude │ tmp` (dimmed), reflecting the
model name and directory from the JSON payload. Claude Code sends a much
richer payload at runtime (context window usage, session id, etc.) — see
this repo's `docs/superpowers/specs/` for the full input/output contract.

Set the `statusLine` command automatically:

```bash
ferrisbar setup
```

This updates `~/.claude/settings.json` (preserving every other setting) to
point `statusLine.command` at this binary's installed location. Use
`ferrisbar setup --project` instead to write `.claude/settings.local.json`
in the current project directory rather than your user-level settings.

Claude Code reads the statusLine config once at session start, so start a new
session after changing it.
```

- [ ] **Step 6: Verify the build one more time after the README change**

Run: `cargo build && cargo test`
Expected: builds cleanly, all 62 tests still pass (README changes don't affect compilation, but this confirms nothing else was accidentally touched).

- [ ] **Step 7: Commit everything together**

```bash
git add Cargo.toml Cargo.lock src/main.rs tests/cli.rs README.md
git commit -m "rename: mystatusline -> ferrisbar"
```

---

## Post-Task Operational Steps

**Why these aren't a dispatched Task:** everything below is either an external operation with no code diff to review (renaming a GitHub repo) or a step the spec explicitly requires a direct, in-the-moment human go-ahead for (the real `cargo publish`). Neither fits the "implementer writes code, reviewer reads a diff" model the rest of this plan uses. These run in the controller's own session, narrated to the user as they happen — not delegated to a fresh subagent.

- [ ] **Step 1: Rename the GitHub repository**

From inside `/home/kwhatcher/projects/mystatusline` (so the local `origin` remote updates automatically):

```bash
gh repo rename ferrisbar
```

Confirm the remote updated:

```bash
git remote -v
```

Expected: both `origin` lines now show `https://github.com/kerryhatcher/ferrisbar.git`.

- [ ] **Step 2: Validate the package with a dry run**

```bash
cargo publish --dry-run
```

Expected: succeeds, listing the files that would be packaged, with no errors about missing required metadata (`description`/`license`) or invalid category slugs.

If this fails, fix the reported issue in `Cargo.toml` and re-run — do not proceed to Step 3 until this passes cleanly.

- [ ] **Step 3: Stop and get explicit confirmation before the real publish**

Tell the user the dry run passed, and that the next step is the actual, permanent `cargo publish` (crates.io names can never be reused, even after a `cargo yank`). Wait for their explicit go-ahead before running:

```bash
cargo publish
```

- [ ] **Step 4: Verify the published crate**

```bash
curl -s -A "ferrisbar-publish-check (kerry@kerryhatcher.com)" https://crates.io/api/v1/crates/ferrisbar
```

Expected: a JSON response describing the newly published crate (not a 404). Report the crates.io URL (`https://crates.io/crates/ferrisbar`) to the user.
