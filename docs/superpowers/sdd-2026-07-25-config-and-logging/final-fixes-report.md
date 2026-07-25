# Final review fix wave — 2026-07-25

Branch: `feat/config-and-logging`. All 7 items from the final-review fix wave
applied in one pass; `just ci` is green.

## Fix 1 (gating) — `todo_file_unreadable` inverted

Removed the directory-existence warn from `main.rs` entirely. Moved the
diagnostic into `todo::active_task`, which now returns
`(Option<String>, Option<String>)`: the task text, and an optional
diagnostic message for `main.rs` to log under the existing
`todo_file_unreadable` event name.

**Shape chosen and why:** a positional 2-tuple rather than a small struct or
`Result`. `todo.rs` must stay infallible (no `?` propagating an error type)
and must not depend on `log`, so a struct with named fields would just be a
tuple with more ceremony for two values used exactly once at the call site.
The tuple destructures as `let (task, todo_diagnostic) = todo::active_task(...)`
in `main.rs`, which reads unambiguously at the call site — task first
(matches the old return value's position, minimizing the diff), diagnostic
second. `(None, None)` means "nothing to report" for every non-fault case:
missing `todos/` dir, no matching file, no `in_progress` entry, empty
session id. A diagnostic is produced only when a file was *matched* for the
session and then failed to read or parse — genuinely once-per-fault.

Added 4 unit tests in `todo.rs`:
- `malformed_json_in_a_matched_file_produces_a_diagnostic`
- `an_unreadable_matched_file_produces_a_diagnostic` (`cfg(unix)`, chmod 000)
- `none_when_dir_missing` (updated to assert `(None, None)`)
- `none_when_no_matching_files` (updated to assert `(None, None)`)

All other existing `todo.rs` tests updated to the tuple shape.

**Behavioral verification** (HOME with only `.claude/`, no `todos/`):
3 consecutive renders → log file never created (previously: 3 warn lines,
forever). Verified by running the built binary directly; see transcript
below.

**Behavioral verification** (matched todo file containing `not json`):
3 consecutive renders → exactly 3 `todo_file_unreadable` warn lines, one per
render:

```
{"ts":...,"level":"warn","event":"todo_file_unreadable","session_id":"s1","msg":"could not parse .../todos/s1-agent-1.json: expected ident at line 1 column 2"}
{"ts":...,"level":"warn","event":"todo_file_unreadable","session_id":"s1","msg":"could not parse .../todos/s1-agent-1.json: expected ident at line 1 column 2"}
{"ts":...,"level":"warn","event":"todo_file_unreadable","session_id":"s1","msg":"could not parse .../todos/s1-agent-1.json: expected ident at line 1 column 2"}
```
(`grep -c todo_file_unreadable` → 3)

## Fix 2 — `setup` reported the wrong log path

`log::resolve_log_path` made `pub` (module `log` is private to the crate, so
`pub` — not `pub(crate)` — is what clippy's `redundant_pub_crate` wants for
crate-internal-only visibility here). `setup::run` now computes the level via
`log::Level::from_str_lenient(&cfg.log.level)` and only prints the `Log:`
line when `cfg.log.enabled && level != Level::Off`, using
`log::resolve_log_path(&cfg.log.path, data_dir)` for the actual path — the
same function `Logger::new` uses, no second implementation.

## Fix 3 — read-only-directory coverage

- `log.rs` unit test `read_only_log_directory_disables_logging_without_erroring`
  (`cfg(unix)`): chmods `<data_dir>/logs` to `0o555`, asserts `Logger::log`
  neither panics nor creates a file, restores the original mode before the
  tempdir drops.
- `tests/cli.rs` e2e test `a_non_writable_data_directory_still_renders`
  (`cfg(unix)`): chmods `<home>/.local/share/ferrisbar` to `0o555` before
  spawning the child, asserts the render still succeeds and contains
  `"Claude"`, restores the mode afterward.

Both skip (not assert vacuously) when `id -u` reports root, since root
bypasses directory permission checks and the write would silently succeed —
this environment runs as uid 1000, so both tests exercised the real path,
confirmed by a passing `cargo test` run.

## Fix 4 — `render` event missing `used_pct`

Added both `used_pct` and `elapsed_micros` (the "if clean" extra was clean:
`Instant::now()` captured as the first line of `main()`, `.elapsed()` read
just before logging). `used_pct` is computed via
`context_bar::compute_used(remaining, total_tokens, acw)` when
`remaining_percentage` is present, else logged as the literal `none`.

Event line shape: `model=... dir=... acw=... used_pct=<n|none> elapsed_micros=<n>`.

## Fix 5 — `FERRISBAR_LOG_LEVEL` now re-enables logging

In `apply_env_overrides` (extracted from `main`, see refactor note below): an
explicit, non-empty `FERRISBAR_LOG_LEVEL` other than `"off"` (case/whitespace
-insensitive) now also sets `cfg.log.enabled = true`. `FERRISBAR_LOG_LEVEL=off`
still disables via the existing `level != Level::Off` check in `Logger::new`.

Added e2e test `ferrisbar_log_level_env_var_re_enables_logging_disabled_in_the_file`
in `tests/cli.rs`: config file has `enabled = false`, env sets
`FERRISBAR_LOG_LEVEL=debug`, asserts the log file contains a `"render"` event.
Routed through `isolated()`, per the existing convention.

## Fix 6 — README precedence note

Amended the `CLAUDE_CODE_AUTO_COMPACT_WINDOW` row's Notes cell: "Overrides
when set to a positive number; `0`, a negative value, or a string that
doesn't parse as a number all defer to the config file." No behavior changed.

## Fix 7 — concurrency test documentation

Added the requested comment to `concurrent_renders_never_produce_a_corrupt_archive`
recording the division of labor with `log.rs`'s
`a_held_lock_defers_rotation_without_losing_the_line` for `O_EXCL`
exclusivity, and why this e2e test structurally cannot detect a missing
`O_EXCL` (misnumbered-but-valid archive on a lost race; microsecond race
window vs. millisecond process spawn).

## Incidental refactor (required to pass the gate)

Adding the Fix 1/4/5 code pushed `main()` to 107 lines, tripping clippy's
`too_many_lines` (limit 100, `-D warnings`). Extracted four named helpers —
`apply_env_overrides`, `flush_config_warnings`, `dispatch_subcommand`,
`log_render_event` — with no behavior change. Verified via a stdout diff
between the pre-fix-wave binary and the post-fix-wave binary across 5 payload
shapes (plain, context-window, matched-task, invalid-JSON, empty-stdin): all
byte-identical.

## Verification

- `just ci`: **exit 0**. `cargo geiger`'s own "Found 1 warnings" print is
  informational only (the justfile's `-cargo geiger` line ignores its exit
  code, per `CLAUDE.md`); the actual `just ci` process exit code was
  confirmed `0` via `echo $?`.
- `cargo test`: 101 unit + 22 e2e = 123 passed, 0 failed.
- Leak check after full `just ci`:
  ```
  $ ls -d ~/.config/ferrisbar ~/.local/share/ferrisbar
  ls: cannot access '/home/kwhatcher/.config/ferrisbar': No such file or directory
  ls: cannot access '/home/kwhatcher/.local/share/ferrisbar': No such file or directory
  ```
  Both absent, as required.

## Commits

Grouped into 3 logical commits (see git log for exact SHAs):
1. `fix(todo)`: Fix 1 — todo.rs diagnostic shape + main.rs wiring + refactor
   to satisfy `too_many_lines` (Fixes 1, 4, 5 landed together since they all
   touch the same `main()` body and the refactor was driven by all three).
2. `fix(setup)`: Fix 2 — setup reports the resolved log path.
3. `test`, `docs`: Fix 3, 6, 7 — added coverage and doc/comment corrections.
