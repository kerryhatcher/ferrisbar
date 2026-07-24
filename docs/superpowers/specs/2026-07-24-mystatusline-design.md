# mystatusline — Rust-based Claude Code status line (v1: Hello World)

## Purpose

Provide a Rust binary that Claude Code invokes as its `statusLine` command, replacing
the currently active Cognee statusline script. This first version establishes the
project scaffolding and protocol shape; it renders a static `Hello World` rather than
real session data.

## Background

Claude Code's `statusLine` feature works by executing a configured command at session
start and on each status refresh, piping a JSON payload (session id, model, cwd, etc.)
to the command's stdin, and rendering whatever the command writes to stdout as the
status line text.

`~/projects/mystatusline` already exists as an initialized git repo (GitHub remote:
`kerryhatcher/mystatusline`) with a Rust-oriented `.gitignore`, README, and LICENSE, but
no Cargo project yet.

The user's currently active `statusLine` entry in `~/.claude/settings.json` points at
`/home/kwhatcher/.claude/plugins/cache/cognee/cognee-memory/0.2.0/scripts/cognee-statusline.sh`.
This project replaces that entry. A separate, unrelated Python-based status-line plugin
exists at `~/projects/status-line` — it is not affected by this work.

## Scope (v1)

- New Cargo binary crate named `mystatusline` in the existing repo root.
- `main.rs` reads and discards stdin (to behave correctly under the real
  statusLine protocol, which always pipes JSON), then writes the literal string
  `Hello World` to stdout.
- No JSON parsing, no formatting/coloring, no config file support — deferred to a
  future iteration.
- Build via `cargo install --path .`, installing the binary to `~/.cargo/bin/mystatusline`.
- Update `~/.claude/settings.json`: replace the `statusLine.command` value with the
  absolute path `~/.cargo/bin/mystatusline` (not a bare command name, since `PATH`
  inside the statusLine execution environment is not guaranteed to include
  `~/.cargo/bin`). Every other key in `settings.json` is preserved unchanged.

## Out of scope (future work)

- Parsing the stdin JSON payload and rendering real session/model/context info.
- Config file support (e.g. a `.claude/statusbar.yaml` equivalent).
- Color/formatting, multiple layout positions.
- Uninstalling/disabling the Cognee plugin's own statusline registration (if any exists
  beyond the settings.json entry).

## Testing

- Manual: `echo '{}' | ~/.cargo/bin/mystatusline` must print `Hello World`.
- After updating settings.json, a new Claude Code session is required for the change
  to take effect (statusLine config loads once at session start); the user will verify
  visually in the next session.

## Risks / notes

- If `settings.json` is a symlink, the target file must be edited directly rather than
  overwriting the symlink.
- Rust toolchain (`cargo`/`rustc` 1.97.0) is already installed and confirmed working.
