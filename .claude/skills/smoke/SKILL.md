---
name: smoke
description: Render ferrisbar's statusline from sample JSON payloads and show the output, including the degradation cases. Use when verifying a change to rendering, layout, the context bar, or payload parsing actually looks right end to end.
---

# Statusline smoke test

ferrisbar reads a Claude Code statusline JSON payload on stdin and prints one
line to stdout. `cargo test` proves the bytes are right; this shows what a human
would actually see in their prompt.

## Run it

Build once, then pipe each payload at the binary:

```bash
cargo build 2>&1 | tail -5
BIN=./target/debug/ferrisbar
TODOS=$(mktemp -d)   # empty todos dir, so the todo segment stays out of the way
```

Run every case below and show the output. Keep the raw ANSI escapes visible in
at least one case — colour is part of the render — but describe what each line
means, since escape codes are hard to read in a terminal transcript.

```bash
# 1. Minimal: model + directory name only
echo '{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"session_id":"sess1"}' \
  | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo

# 2. Full context bar, fresh session (100% remaining)
echo '{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"context_window":{"remaining_percentage":100.0,"total_tokens":1000000}}' \
  | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo

# 3. Context bar under pressure — check the colour threshold
echo '{"model":{"display_name":"Opus"},"workspace":{"current_dir":"/tmp/myproject"},"context_window":{"remaining_percentage":18.0,"total_tokens":1000000}}' \
  | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo

# 4. Missing model — must fall back to "Claude", not panic
echo '{"workspace":{"current_dir":"/tmp/myproject"}}' \
  | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo

# 5. Wrong-typed field — must degrade, not panic
echo '{"model":{"display_name":42},"workspace":{"current_dir":"/tmp/myproject"}}' \
  | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo

# 6. Not JSON at all — must print nothing and exit 0
echo 'not json' | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo "exit=$?"

# 7. Empty stdin — same
printf '' | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo "exit=$?"
```

Clean up with `rm -rf "$TODOS"` when done.

## What to check

Cases 4–7 are the crate's core invariant: never panic on input, degrade to a
shorter line, and print nothing on unparseable stdin while still exiting `0`.
If any of them panics, prints a stack trace, or exits nonzero, that is a
release blocker — a crash here corrupts someone's prompt on every render.

## Exercising the todo segment

Don't point `CLAUDE_CONFIG_DIR` at a real `~/.claude` to test this. Fake it —
the filename must start with the payload's `session_id`, contain `-agent-`, and
end in `.json` (`src/todo.rs:24`); the newest matching file wins:

```bash
mkdir -p "$TODOS/todos"
echo '[{"status":"in_progress","activeForm":"Rendering the context bar","content":"Render the context bar"}]' \
  > "$TODOS/todos/sess1-agent-abc.json"
echo '{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/myproject"},"session_id":"sess1"}' \
  | CLAUDE_CONFIG_DIR=$TODOS $BIN; echo
```

The `activeForm` of the first `in_progress` item renders bold between the model
and the directory. No `in_progress` item, an empty `session_id`, or an
unreadable file all mean the segment is simply omitted.
