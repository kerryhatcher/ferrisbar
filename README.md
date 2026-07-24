# mystatusline
A tool for custom claude status line

## Build & install

```bash
cargo install --path .
```

This installs the binary to `~/.cargo/bin/mystatusline`.

## Wiring into Claude Code

Prerequisite: a working Rust toolchain (`cargo`/`rustc`) is required to build
and install the binary.

After running `cargo install --path .`, verify the binary works before wiring
it up:

```bash
echo '{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp"}}' | mystatusline
```

This should print a statusline like `Claude │ tmp` (dimmed), reflecting the
model name and directory from the JSON payload. Claude Code sends a much
richer payload at runtime (context window usage, session id, etc.) — see
this repo's `docs/superpowers/specs/` for the full input/output contract.

Set the `statusLine` command in `~/.claude/settings.json` (or a project-level
`.claude/settings.json`) to the absolute path of the installed binary:

```json
"statusLine": {
  "type": "command",
  "command": "/home/kwhatcher/.cargo/bin/mystatusline"
}
```

The path above is machine-specific — this repo is public, and `cargo install`
may place the binary somewhere other than `~/.cargo/bin` depending on your
Cargo configuration. Confirm the actual location on your machine with
`which mystatusline` and use that path in your own `statusLine` config.

Claude Code reads the statusLine config once at session start, so start a new
session after changing it.
