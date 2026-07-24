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
