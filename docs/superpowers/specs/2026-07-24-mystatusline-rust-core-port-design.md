# mystatusline — Rust port of core statusline.py features (v2)

## Purpose

Replace the current `Hello World` placeholder in `mystatusline` with a real
statusline renderer, ported from the Python plugin at
`~/projects/status-line/bin/statusline.py`. This is a **core-only** port: it
picks up model name, context-window usage bar, active todo task, and
directory name, but deliberately drops speckit feature-state tracking,
rate-limit meters, the `.claude/statusbar.yaml` config file, last-slash-command
display, and the `/tmp/claude-ctx-<session>.json` bridge file.

## Background

`~/projects/status-line` is a separate, Python-based Claude Code plugin
(`uv run --script` shebang, PyYAML dependency) with the full feature set
described in its README. It is not modified by this work — it remains the
reference implementation being ported from, not a dependency of
`mystatusline`.

`mystatusline` v1 (see `2026-07-24-mystatusline-design.md`) established the
Cargo binary scaffolding: it reads and discards stdin, then prints
`Hello World`. This spec supersedes that placeholder behavior while keeping
the same installation model — a plain Cargo binary wired directly into
`~/.claude/settings.json`'s `statusLine.command` (not a Claude Code plugin).

## Scope (v2)

### In scope

- Parse the stdin JSON payload (Claude Code's statusLine protocol):
  - `model.display_name` (default: `"Claude"`)
  - `workspace.current_dir` (default: current process cwd)
  - `session_id`
  - `context_window.remaining_percentage`
  - `context_window.total_tokens` (default: `1_000_000`)
- Context-usage bar: 10-segment colored bar (`█`/`░`), computed the same way
  as Python —
  - Buffer percentage defaults to `16.5`, but is overridden via
    `CLAUDE_CODE_AUTO_COMPACT_WINDOW` (an env var giving a token count) when
    set to a positive integer: `buffer_pct = min(100, max(0, (1 - acw/total) * 100))`.
  - `usable_remaining = max(0, (remaining - buffer_pct) / (100 - buffer_pct) * 100)`
  - `used = clamp(round(100 - usable_remaining), 0, 100)`
  - Color thresholds: green `<50`, yellow `<65`, orange `<80`, red+`💀` `>=80`.
  - Omitted entirely when `context_window.remaining_percentage` is absent.
- Active todo task: look in `$CLAUDE_CONFIG_DIR/todos` (fallback
  `$HOME/.claude/todos`) for files whose name starts with `session_id`,
  contains `-agent-`, and ends with `.json` (not a glob — three independent
  string checks, matching Python), pick the most recently modified, parse it
  as a JSON array of todos, and use the
  first entry with `status == "in_progress"` — displaying its `activeForm`
  (falling back to `content`) in bold.
- Directory name: `basename(cwd)`, dimmed.
- Compose a single-line output (Python's "end" position, now the only
  position since config-driven position toggling is out of scope):
  `<dim>model</dim> │ [<bold>task</bold> │ ]<dim>dirname</dim>[ <dim>│</dim> <color>bar pct%</color>]`
- Write to stdout with **no trailing newline** (matches Python's
  `sys.stdout.write`; the current v1 `println!` behavior does not carry
  forward).
- Silent degradation on any malformed/missing input, matching Python:
  invalid JSON → empty stdout, exit 0; unreadable todos dir → no task shown;
  unreadable/malformed todo file → skip it, don't error.

### Out of scope (unchanged from Python, deferred indefinitely or to a future spec)

- speckit feature-state chain tracking (`find_speckit_feature_dir`,
  `read_speckit_state`, `next_speckit_command`, progress bar).
- Rate-limit meters (5h/weekly usage, reset-time formatting) and the second
  output line they occupy.
- `.claude/statusbar.yaml` config file (`context_position`,
  `show_last_command`) and the config-walk-up logic.
- Last-slash-command transcript lookup (`read_last_slash_command`) — used
  both by `show_last_command` and by speckit's `analyze` detection, both out
  of scope here.
- The `/tmp/claude-ctx-<session>.json` bridge file written for an external
  "PostToolUse context monitor" — no such consumer exists in this repo.
- "front" layout position (only "end" is implemented).

## Architecture

New modules under `src/`, replacing the single-file `main.rs`:

- **`payload.rs`** — `serde::Deserialize` structs mirroring the subset of the
  stdin JSON shape needed above, with manual default handling matching
  Python's `.get(...) or default` (e.g. empty/missing `display_name` falls
  back to `"Claude"`, not just missing).
- **`context_bar.rs`** — pure functions: computing `used` from
  `(remaining_percentage, total_tokens, acw_env)`, and rendering the colored
  bar string. No I/O.
- **`todo.rs`** — `active_task(session_id: &str, todos_dir: &Path) -> Option<String>`,
  doing the file-scan/mtime-pick/parse/status-filter described above. Takes
  the todos directory as a parameter (resolved by the caller from env vars)
  so it's testable with a tempdir.
- **`layout.rs`** — ANSI color constants (`DIM`, `BOLD`, `RESET`, `GREEN`,
  `YELLOW`, `ORANGE`, `RED`, `BLINK_RED`) and
  `compose_statusline(model: &str, ctx: &str, task: Option<&str>, dirname: &str) -> String`.
- **`main.rs`** — reads all of stdin, attempts JSON parse (returns early with
  no output on failure), resolves the todos directory from env, calls into
  the above modules, prints the composed line via `print!` (not `println!`).

### Dependencies

`serde` + `serde_json` only. No YAML, regex, or datetime crate — everything
in scope needs just JSON parsing and integer/string formatting. `$HOME`
resolution uses `std::env::var("HOME")` directly (no `dirs` crate), since
this tool targets Unix-like systems the same as the Python original.

## Data flow

1. `main` reads all of stdin to a `String`.
2. Parse as JSON into a `Payload` struct; on error, return immediately (no
   stdout output at all — matches Python's bare `except: return`).
3. Extract `model`, `cwd`, `session_id`, `remaining_percentage`, `total_tokens`
   with their defaults.
4. Compute `ctx` string via `context_bar::render(remaining_percentage,
   total_tokens)` — empty string if `remaining_percentage` is `None`.
5. Resolve todos dir: `$CLAUDE_CONFIG_DIR/todos` if set, else
   `$HOME/.claude/todos`. Call `todo::active_task(session_id, &todos_dir)` —
   `None` if `session_id` is empty or dir doesn't exist.
6. `dirname` = `Path::new(&cwd).file_name()`, falling back to the full `cwd`
   string if `file_name()` returns `None` (e.g. cwd is `/`).
7. `layout::compose_statusline(&model, &ctx, task.as_deref(), &dirname)` →
   final string.
8. `print!("{output}")` — no trailing newline.

## Testing

- **Unit tests** (colocated per module):
  - `context_bar`: no-remaining-percentage → empty string; each color
    threshold boundary (49/50, 64/65, 79/80); `CLAUDE_CODE_AUTO_COMPACT_WINDOW`
    override changes the buffer percentage and thus `used`.
  - `todo`: empty dir → `None`; picks the newest of multiple matching files by
    mtime; ignores files failing any of the three name checks (prefix,
    `-agent-` substring, `.json` suffix); todo list with no `in_progress`
    entry → `None`; `activeForm` preferred over `content`; malformed JSON
    file → `None`, no panic.
  - `layout`: with/without task, with/without ctx bar — exact string match
    against expected ANSI-coded output.
- **Integration test** (`tests/cli.rs`, replacing the current hello-world
  assertions): spawn the built binary with fixture JSON on stdin, assert
  exact stdout bytes (no trailing newline) for: minimal payload (model only,
  no context, no todos dir), full payload with context bar at a known
  percentage, invalid JSON (expect empty stdout, exit success), and a payload
  whose session has an `in_progress` todo written to a tempdir-based
  `CLAUDE_CONFIG_DIR`.

## Risks / notes

- Changing from `println!` to `print!` (no trailing newline) is an
  intentional behavior change from v1, needed for parity with the Python
  original and with Claude Code's actual statusLine rendering expectations.
- `CLAUDE_CONFIG_DIR`/`HOME` env vars must be read at runtime, not cached,
  since tests will override them per-test-process (`Command::env(...)`) —
  no global/lazy_static state.
- This is a strict subset of Python's behavior; a user relying on speckit
  state, rate-limit meters, `.claude/statusbar.yaml`, or `show_last_command`
  today would lose those if they switch from the Python plugin to this
  binary. That's an accepted tradeoff of this spec, not an oversight.
