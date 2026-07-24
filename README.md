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

Set the `statusLine` command automatically:

```bash
mystatusline setup
```

This updates `~/.claude/settings.json` (preserving every other setting) to
point `statusLine.command` at this binary's installed location. Use
`mystatusline setup --project` instead to write `.claude/settings.local.json`
in the current project directory rather than your user-level settings.

Claude Code reads the statusLine config once at session start, so start a new
session after changing it.
