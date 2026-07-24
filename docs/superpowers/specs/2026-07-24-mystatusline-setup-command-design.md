# mystatusline — `setup` subcommand (v3)

## Purpose

Add a `setup` subcommand to the `mystatusline` binary that updates a user's
Claude Code settings file so `statusLine.command` points at the currently
installed `mystatusline` binary — replacing the manual JSON-editing steps
currently documented in this repo's README.

## Background

Today, wiring `mystatusline` into Claude Code requires manually editing
`~/.claude/settings.json` (per README.md's "Wiring into Claude Code"
section) to set the absolute path of the installed binary. The Python
`status-line` plugin at `~/projects/status-line` solves the equivalent
problem via a `/status-line:setup` slash command that dispatches Claude
Code's built-in `statusline-setup` agent — but `mystatusline` is a plain
Cargo binary, not a Claude Code plugin, so it has no slash-command
infrastructure. This spec adds the same convenience as a CLI subcommand of
the binary itself, doing the settings.json edit directly rather than via an
agent.

## Scope (v3)

### In scope

- New subcommand: `mystatusline setup` and `mystatusline setup --project`.
  - No args (existing behavior, **unchanged**): read stdin, render the
    statusline. Claude Code's `statusLine` hook never passes CLI arguments,
    so this path is untouched by this spec.
  - `setup` (no further args): target `$HOME/.claude/settings.json`.
  - `setup --project`: target `.claude/settings.local.json` under the
    current working directory (created, along with the `.claude/`
    directory, if either doesn't exist). No walking up from cwd looking for
    an existing `.claude/` — run it from the project root.
  - Any other argument (unrecognized subcommand, or `setup` combined with
    anything other than exactly `--project`): print a usage message to
    stderr and exit with a nonzero status. No render, no file write.
- The target path resolves via `std::env::current_exe()` — the absolute
  path of the binary currently running `setup`. This is definitionally "the
  installed version," regardless of where `cargo install` (or a manual
  copy) placed it.
- The settings file's `statusLine` key is fully replaced with:
  ```json
  { "type": "command", "command": "<resolved absolute path>" }
  ```
  Every other top-level key in the settings file is preserved unchanged.
- If the settings file doesn't exist yet, it's created (along with its
  parent directory, e.g. `~/.claude/` or `./.claude/`) containing just
  `{"statusLine": {...}}`.
- After a successful write, print a short report to stdout:
  ```
  Updated statusLine in <resolved settings file path>
    before: <previous statusLine.command value, or "(none)">
    after:  <new path>
  Start a new Claude Code session for the change to take effect.
  ```
- No interactive confirmation prompt — invoking the subcommand is the
  user's confirmation. (Decided during brainstorming: "Report, no prompt.")
- Error handling: if the settings file exists but fails to parse as JSON,
  or parses to a JSON value that isn't an object, **abort without writing
  anything** and print a clear error to stderr (e.g. naming the file and the
  parse problem) with a nonzero exit. Never blind-overwrite a file whose
  existing structure isn't understood.
- Symlinked settings files: no special-case code. `std::fs::write` (via
  `File::create`, no `O_NOFOLLOW`) follows symlinks on Unix by default, so
  writing to a symlinked settings path already writes through to its
  target — verified during brainstorming, not left as an open risk.

### Out of scope

- `clap` or any other CLI-parsing crate — manual `std::env::args()`
  matching is sufficient for two subcommand shapes and keeps this crate's
  dependency footprint minimal (runtime deps remain `serde` + `serde_json`
  only, per the original design's Global Constraints).
- An interactive y/n confirmation or `--yes` flag.
- Walking up from cwd to find an existing `.claude/` directory for
  `--project` (unlike the speckit-lookup-style walk-up in the Python
  original's out-of-scope config feature) — cwd-direct only.
- Targeting the shared, typically-committed `.claude/settings.json` for
  project scope — `--project` always uses `.claude/settings.local.json`
  (Claude Code's personal/gitignored-by-convention override file), since a
  machine-specific absolute binary path shouldn't land in checked-in config.
- Any other `statusLine`-adjacent settings (there are none today besides
  `type`/`command`).

## Architecture

- **`src/setup.rs`** (new module):
  - `pub fn run(project_scope: bool) -> Result<(), String>` — resolves the
    target settings path, reads+parses (or starts fresh), mutates the
    `statusLine` key, writes the file back, prints the before/after report.
    Returns `Err(message)` for any failure (parse error, non-object root,
    I/O error) rather than panicking; `main.rs` prints the message to
    stderr and exits nonzero.
  - Internal helper(s) for: resolving the settings path (`$HOME` for
    user-scope, cwd-joined `.claude/settings.local.json` for
    `--project`), and the read-mutate-write JSON logic — kept separable so
    the JSON logic is unit-testable without touching real `$HOME`.
- **`src/main.rs`**: grows a dispatch at the very top of `main()`, before
  today's stdin-read logic:
  - `std::env::args().skip(1).collect::<Vec<_>>()` — `[]` → existing
    behavior (fall through, unchanged); `["setup"]` → `setup::run(false)`;
    `["setup", "--project"]` → `setup::run(true)`; anything else → usage
    message to stderr, `std::process::exit(1)`.
- **`Cargo.toml`**: add the `preserve_order` feature to the existing
  `serde_json` dependency (`serde_json = { version = "1", features =
  ["preserve_order"] }`) so re-serializing the settings JSON preserves the
  user's existing key order instead of resorting it alphabetically (the
  default behavior of `serde_json::Map` without this feature). This pulls
  in `indexmap` transitively via `serde_json` — no new crate is added
  directly to `Cargo.toml`'s dependency list.

## Data flow

1. `main()` inspects `env::args()`. If it doesn't match one of the two
   recognized `setup` forms and isn't empty, print usage to stderr and exit
   1 — do not fall through to stdin-reading (which would otherwise hang
   waiting on input that will never come in a non-statusLine invocation).
2. For a recognized `setup` invocation, resolve the target path:
   - User scope: `$HOME/.claude/settings.json` (`$HOME` read directly via
     `std::env::var`, consistent with how `main()` already resolves the
     todos directory — no `dirs` crate).
   - Project scope: `<cwd>/.claude/settings.local.json`.
3. Read the file if it exists (`fs::read_to_string`); if absent, treat as
   `"{}"`. Parse as `serde_json::Value`. If parsing fails, or the parsed
   value isn't `Value::Object`, return an `Err` describing the problem —
   nothing is written.
4. Capture the previous value at `statusLine.command` (if present and a
   string) for the before/after report.
5. Resolve the running binary's absolute path via `env::current_exe()`
   (mapped to an `Err` on failure — vanishingly rare, but not `.unwrap()`'d).
6. Set the object's `"statusLine"` key to
   `json!({"type": "command", "command": <resolved path>})`, leaving every
   other key untouched.
7. Ensure the parent directory exists (`fs::create_dir_all`), then write the
   re-serialized (`to_string_pretty`) JSON back to the target path.
8. Print the before/after report to stdout; return `Ok(())`.

## Testing

- **Unit tests** (in `src/setup.rs`, exercising the JSON-mutate logic
  directly against tempdir-based fake settings paths — not real `$HOME`):
  - Settings file doesn't exist → created with just `{"statusLine": {...}}`.
  - Settings file exists with unrelated keys (`"env": {...}`, etc.) → those
    keys survive unchanged; only `statusLine` is replaced.
  - Settings file already has a `statusLine` entry → old value captured
    correctly for the "before" report, then replaced.
  - Settings file contains invalid JSON syntax → `Err` returned, file on
    disk is unmodified (read back and compare to original bytes).
  - Settings file parses to a non-object (e.g. a JSON array or a bare
    string) → `Err` returned, file unmodified.
  - Key order preservation: a settings file with keys in a specific
    non-alphabetical order round-trips with that order intact (proves the
    `preserve_order` feature is doing its job).
- **Integration tests** (`tests/cli.rs`, spawned real binary):
  - `mystatusline setup` with `HOME` pointed at a tempdir → verify
    `<tempdir>/.claude/settings.json` on disk has the expected
    `statusLine` value pointing at `CARGO_BIN_EXE_mystatusline`'s path.
  - `mystatusline setup --project` run with cwd set to a tempdir → verify
    `<tempdir>/.claude/settings.local.json` is created correctly.
  - `mystatusline badsubcommand` → nonzero exit, stderr non-empty, no file
    written, no hang (process exits promptly rather than blocking on stdin).
  - `mystatusline` with no args and a valid JSON payload on stdin → still
    produces the normal statusline output (regression check that dispatch
    didn't break the existing default path).

## Risks / notes

- `preserve_order` changes `serde_json::Map`'s internal representation
  (`IndexMap` instead of `BTreeMap`) for every consumer in this crate, not
  just `setup.rs` — but no other module in this crate currently constructs
  or inspects a bare `serde_json::Map`/`Value::Object` directly (`payload`,
  `todo` all deserialize into typed structs), so this is a no-op change
  everywhere except the new `setup.rs`.
- `to_string_pretty`'s formatting (2-space indent, specific spacing) may
  not exactly match however the user's settings.json was previously
  formatted (e.g. if hand-edited with different indentation). This is an
  accepted minor cosmetic tradeoff — Claude Code itself writes this file
  with standard JSON formatting when it manages it, so in practice this
  should rarely be visible.
- No test coverage of actual `$HOME`/real settings.json — all tests use
  tempdir-based paths or override `HOME` for the child process, never
  touching the developer's real Claude Code configuration.
