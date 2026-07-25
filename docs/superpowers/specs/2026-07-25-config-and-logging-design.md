# ferrisbar — configuration file and JSONL logging

## Purpose

Give ferrisbar a user-editable configuration file and a diagnostic log,
both stored in platform-appropriate user directories and both created
automatically on first run. The config file replaces today's
environment-variables-only story; the log makes ferrisbar's currently
silent failure modes debuggable.

## Background

ferrisbar reads a JSON payload on stdin and prints one statusline to
stdout. It has no persistent state. Two environment variables
(`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_AUTO_COMPACT_WINDOW`) are its entire
configuration surface, and `README.md:222` states flatly "There is no
config file."

Two problems follow from that. First, settings that should persist have
to be re-exported in a shell profile. Second, and more seriously, every
failure degrades silently: `main.rs:48` returns on unreadable stdin and
`main.rs:52` returns on unparseable stdin, both printing nothing. A user
whose statusline goes blank has no way to find out why.

This spec adds a TOML config file, a JSONL log with size-based rotation
and gzip archives, and a documented precedence order between environment
variables, the config file, and built-in defaults.

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| Compressed archives (costs a runtime dep) | Yes — add `flate2`, archives are `.gz` |
| Config format | TOML (adds `toml` dep; comments are worth it) |
| Log content | Degradation events at `warn`; per-render lines at `debug` |
| Config scope | Logging keys + adopt existing env vars + display options |
| Structure | One spec, two implementation PRs |
| macOS paths | Apple conventions (`~/Library/Application Support`) |

## Scope

### Phase 1 (first PR)

- Platform directory resolution (`src/paths.rs`)
- TOML config load/create/clamp (`src/config.rs`), including the
  `[display]` schema being *specified* but not emitted or parsed
- JSONL logging with rotation and gzip archives (`src/log.rs`)
- Env-var precedence layering over the config file
- `main.rs` wiring; `setup` reports resolved paths
- README rewrite of the Configuration section
- `flate2` and `toml` added, with `supply-chain/` exemptions

### Phase 2 (second PR)

- `[display]` keys parsed and threaded through `context_bar.rs` and
  `layout.rs`
- `[display]` block added to the generated config template

### Out of scope

- Log shipping, remote sinks, or any network behavior
- Time-based (as opposed to size-based) rotation
- A `--config <path>` CLI flag
- Any change to the statusline's rendered output in Phase 1

## Paths

| | Linux | macOS |
|---|---|---|
| Config dir | `$XDG_CONFIG_HOME/ferrisbar/` else `~/.config/ferrisbar/` | `~/Library/Application Support/ferrisbar/` |
| Data dir | `$XDG_DATA_HOME/ferrisbar/` else `~/.local/share/ferrisbar/` | `~/Library/Application Support/ferrisbar/` |
| Config file | `<config dir>/config.toml` | `<config dir>/config.toml` |
| Log file | `<data dir>/logs/ferrisbar.jsonl` | `<data dir>/logs/ferrisbar.jsonl` |

Platform selection uses `cfg(target_os = "macos")`. On macOS the config
and data directories are the same path; the code must not assume they
differ.

`$XDG_CONFIG_HOME` and `$XDG_DATA_HOME` are honored only when set **and**
non-empty, and only when they are absolute paths — a relative XDG value
is ignored in favor of the fallback, per the XDG base directory spec.

### The empty-HOME guard

`config_dir.rs:10` currently does `env::var("HOME").unwrap_or_default()`,
which produces `PathBuf::from("")`. Joining onto that yields a **relative**
path: `"ferrisbar/config.toml"` rather than an absolute one. Today nothing
writes to that path, so the bug is latent. This feature would activate it,
causing `create_dir_all` to create a stray `./ferrisbar/` directory inside
whatever repository Claude Code happens to be running in.

Therefore: every resolver in `paths.rs` returns `Option<PathBuf>` and
returns `None` when `HOME` is unset or empty. `None` means skip config
file creation, disable logging, and render the statusline normally.

`config_dir.rs` keeps its `unwrap_or_default()` fallback, which is only
ever used for reads, where a relative path fails harmlessly rather than
creating anything. Its signature does change: `claude_config_dir()` gains
a `file_override: Option<&str>` parameter so the precedence chain
(`$CLAUDE_CONFIG_DIR` → `claude.config_dir` → `~/.claude`) lives in one
place rather than being reassembled in `main.rs`. Existing callers at
`main.rs:14` and `setup.rs:22` pass the config value through.

## Config schema

The full schema, as designed. Phase 1 emits and parses everything except
`[display]`.

```toml
# ferrisbar configuration.  https://github.com/kerryhatcher/ferrisbar
# Environment variables override anything set here.

[log]
enabled        = true
level          = "warn"    # "off" | "warn" | "debug"
path           = ""        # "" = <data dir>/logs/ferrisbar.jsonl
max_size_bytes = 1048576   # rotate at 1 MiB
max_archives   = 7         # keep .1.gz … .7.gz

[claude]
config_dir          = ""   # "" = $CLAUDE_CONFIG_DIR, else ~/.claude
auto_compact_window = 0    # 0 = use the built-in 16.5% buffer
```

Phase 2 adds:

```toml
[display]
bar_width          = 10
threshold_yellow   = 50
threshold_orange   = 65
threshold_critical = 80
show_task          = true
```

### Why `[display]` is absent from Phase 1's template

Phase 1's generated `config.toml` omits the `[display]` block entirely.
Emitting keys the binary ignores would lead users to set `bar_width = 20`,
see no change, and report a bug. Phase 2 adds the block to the template
and to the parser in the same change.

### Creation

When `config.toml` does not exist, ferrisbar writes it from a **static
commented template string** — not `toml::to_string(&Config::default())`.
Serializing would discard the comments, which are the entire reason TOML
was chosen over the already-vendored `serde_json`.

Creation failure (read-only directory, full disk) is non-fatal: defaults
apply and rendering proceeds.

### Parsing and validation

- Malformed TOML → all defaults, **exactly one** `config_parse_failed`
  warning, render proceeds. One warning, not one per bad key, so a garbage
  file cannot produce a burst of log lines per render.
- Unknown keys → ignored, via `#[serde(default)]` on every field. A config
  written by a newer ferrisbar must never break an older binary.
- Wrong-typed values → that field falls back to its default; the rest of
  the file still applies.

### Clamping

Both numeric log fields are reachable from a hand-edited file and are
clamped on load:

| Field | Clamp | Rationale |
|---|---|---|
| `max_size_bytes` | floor 4096 | `0` would mean rotate on every write |
| `max_archives` | `1..=64` | `0` leaves the shift loop no destination |

Phase 2 clamps `bar_width` to `1..=100` and requires
`threshold_yellow < threshold_orange < threshold_critical`, falling back
to 50/65/80 when the ordering does not hold.

### The bootstrap ordering problem

A config parse failure is precisely the event worth logging, but the
config is what determines where the log lives. `Config::load()` therefore
returns `(Config, Vec<Event>)` — warnings buffered as data and flushed
once the logger exists. This also keeps `load()` infallible, which the
never-panic invariant wants regardless.

The buffer is bounded: config loading emits at most a small fixed number
of events (parse failure, creation failure, clamp applied), never one per
key.

**Accepted consequence:** a persistently malformed config re-warns on
every render, so a broken file accumulates duplicate lines. This is
correct — it is a live fault rather than a past event — but it is the one
case where `warn` level can still generate volume.

## Precedence

Environment beats file beats default, at every layer. Inverting this would
silently break users who export `CLAUDE_CODE_AUTO_COMPACT_WINDOW` in a
shell profile today.

| Setting | 1. Environment | 2. `config.toml` | 3. Default |
|---|---|---|---|
| Claude config dir | `CLAUDE_CONFIG_DIR` | `claude.config_dir` | `~/.claude` |
| Auto-compact window | `CLAUDE_CODE_AUTO_COMPACT_WINDOW` | `claude.auto_compact_window` | 16.5% buffer |
| Log path | `FERRISBAR_LOG_PATH` | `log.path` | `<data dir>/logs/ferrisbar.jsonl` |
| Log level | `FERRISBAR_LOG_LEVEL` | `log.level` | `warn` |

`FERRISBAR_LOG_PATH` is named in full rather than `FERRISBAR_LOG`, which
would read as a level by analogy with `RUST_LOG`.

A `log.path` (or `FERRISBAR_LOG_PATH`) that is **relative** is resolved
against the data directory, not the process working directory — the
working directory is whatever project Claude Code is running in, and
resolving there would scatter log files across repositories. A path whose
parent directory cannot be created disables logging.

An empty string is treated as unset at every layer, matching how
`config_dir.rs:8` already handles `CLAUDE_CONFIG_DIR`. For
`auto_compact_window`, `0` is treated as unset, matching `main.rs:65`.

The two `FERRISBAR_*` variables exist so logging can be turned up for a
single session without editing a file — the common debugging move.

Phase 2 adds no environment variables for `[display]`; those are
file-only.

## Logging

### Format

One JSON object per line at the resolved log path:

```json
{"ts":1753467296123,"level":"warn","event":"stdin_parse_failed","session_id":"abc123","msg":"expected value at line 1 column 1"}
```

- `ts` — epoch milliseconds, from
  `SystemTime::now().duration_since(UNIX_EPOCH)`. Deliberately **not**
  RFC3339: date formatting would require `chrono` or `time` as a fifth
  runtime dependency. Epoch millis are machine-readable and trivially
  converted (`date -d @1753467296`).
- `level` — `"warn"` or `"debug"`.
- `event` — a stable snake_case identifier, so the log is greppable
  without parsing prose.
- `session_id` — omitted when not yet known (e.g. stdin parse failures).
- `msg` — free-form human detail.

Lines are serialized with `serde_json` and written with a trailing `\n`.
A serialization failure drops the line rather than propagating.

### Events

At `level = "warn"`:

| Event | Trigger |
|---|---|
| `stdin_read_failed` | `main.rs:48` — stdin unreadable |
| `stdin_parse_failed` | `main.rs:52` — payload not valid JSON |
| `todo_file_unreadable` | `todo.rs` — session todo file missing or malformed |
| `config_parse_failed` | `config.toml` present but invalid |
| `config_create_failed` | Could not write the initial template |
| `log_rotate_failed` | Rotation raised an error |

At `level = "debug"`, additionally one `render` event per invocation
carrying `session_id`, `model`, `cwd`, `used_pct`, and `elapsed_micros`.

A quiet log is the healthy state: at default level, any content at all is
a signal that something degraded.

`level = "off"` disables all events; `enabled = false` additionally
prevents the log file and `logs/` directory from being created at all.

### Rotation

Checked on each write, when the log file is at or over `max_size_bytes`:

1. Acquire `<logdir>/.rotate.lock` via `OpenOptions::new().create_new(true)`
   — an atomic `O_EXCL` create. A lock file whose mtime is more than 60
   seconds old is treated as stale, removed, and the acquisition retried
   once.
2. The winner shifts archives downward (`.6.gz → .7.gz`, …, `.1.gz →
   .2.gz`), dropping the old `.max_archives.gz`; renames
   `ferrisbar.jsonl` to `ferrisbar.jsonl.tmp`; gzips that to
   `ferrisbar.jsonl.1.gz`; unlinks the temp file; releases the lock.
3. A process that **fails** to acquire the lock does not rotate. It closes
   its handle, reopens the log by path, and appends.

### Why step 3 is not optional

Several Claude Code sessions run ferrisbar concurrently. A process that
opened the log *before* the winner renamed it still holds a descriptor
pointing at that same inode. Appending through that stale descriptor would
write into the file the winner is concurrently gzipping — producing a
corrupt archive, not merely a lost line.

The ordering is therefore fixed as **lock → stat → rotate → open →
append**, never open-then-lock. Any implementation that opens the log
before acquiring the lock is wrong regardless of how it behaves in tests.

### Performance

Gzipping roughly 1 MiB inside the render path costs about 10–30 ms. This
is accepted: it occurs once per megabyte written, and the alternative — a
background thread in a process that lives for a few milliseconds — is
worse. Steady-state cost per render at `warn` level with a healthy setup
is one `stat` and no writes.

## Dependencies

Two new runtime dependencies. `CLAUDE.md` requires justification and a
`supply-chain/` entry for a third; both are recorded here.

| Crate | Justification |
|---|---|
| `flate2` | gzip for rotated archives. Pinned `default-features = false, features = ["rust_backend"]` so it routes through pure-Rust `miniz_oxide` rather than `libz-sys` — keeping a C toolchain out of the build and keeping `cargo geiger`'s unsafe count down. |
| `toml` | Config parsing. Chosen over `serde_json` because comments in the generated file are the point. |

Transitively this adds roughly `miniz_oxide`, `crc32fast`, `toml_edit`,
`winnow`, `serde_spanned`, and `toml_datetime`.

### CI gates

- **`cargo deny`** — all new crates are MIT/Apache-2.0. `miniz_oxide`'s
  `MIT OR Zlib OR Apache-2.0` expression is satisfied through its MIT arm,
  so `deny.toml`'s allow-list needs **no change**.
- **`cargo vet`** — `supply-chain/config.toml` needs `[[exemptions.*]]`
  entries for every new crate, matching the existing `safe-to-deploy` /
  `safe-to-run` pattern. `just ci` fails at the `vet` step until they
  exist. Generate with `cargo vet suggest`.
- **MSRV 1.85.1** — both crates and the chosen std APIs must build at
  1.85.1. `just msrv` is the gate.

## Module layout

| File | Responsibility | Depends on |
|---|---|---|
| `src/paths.rs` | New. Platform config/data dirs. | env only |
| `src/config.rs` | New. Parse, create, clamp. | `paths` |
| `src/log.rs` | New. Append, rotate, gzip. | `config` |
| `src/config_dir.rs` | Gains an override parameter. | env |
| `src/main.rs` | Wiring. | all |
| `src/setup.rs` | Reports resolved paths. | `paths`, `config` |

`main.rs` order of operations, after argument parsing and before reading
stdin: resolve paths → ensure directories → load config → initialize
logger → flush deferred warnings → render.

`setup` shares one `ensure()` with the render path rather than
duplicating directory-creation logic, and adds the resolved config and log
paths to its existing report.

## Invariants

Both are extensions of rules the codebase already holds.

1. **Nothing new on stdout, ever.** A stray `println!` corrupts the prompt
   on every render. All diagnostics go to the log file or stderr. The
   end-to-end tests assert stdout is byte-identical to today's output.

2. **Every new failure mode degrades.** An unset `HOME`, an unwritable
   data directory, a full disk, a malformed config, or a failed rotation
   each disable the offending piece and still print the statusline. No new
   `unwrap`, `expect`, or panicking index on the render path.

## Testing

### Environment isolation

`tests/cli.rs` drives the real binary. Without overriding `HOME` and the
`XDG_*` variables to a tempdir, `just ci` would write into the developer's
actual `~/.config` and `~/.local/share` — and would rotate and gzip their
real log. Every test touching paths sets those variables explicitly.
`tempfile` is already a dev-dependency.

### Unit tests

`mod tests` beside each module, per house style.

**`paths.rs`**
- `XDG_CONFIG_HOME` / `XDG_DATA_HOME` honored when set and absolute
- relative XDG value ignored in favor of the fallback
- fallback paths when XDG unset
- **`None` when `HOME` is empty** — guards the relative-path bug directly
- macOS branch resolves both dirs to Application Support

**`config.rs`**
- missing file is created with the template
- the generated template round-trips through the parser
- malformed TOML → defaults plus exactly one warning
- unknown keys ignored
- wrong-typed field falls back without discarding the rest
- `max_size_bytes = 0` clamps to 4096
- `max_archives = 0` clamps to 1; `max_archives = 999` clamps to 64
- env var beats file value; file value beats default; empty env treated as
  unset

**`log.rs`**
- no file written when `enabled = false`
- `warn` level suppresses `debug` events
- rotation triggers at the threshold, not before
- archive count caps at `max_archives`, oldest dropped
- **a `.gz` archive decompresses to exactly the bytes written**
- read-only log directory disables logging without erroring
- a stale `.rotate.lock` (mtime > 60s) is reclaimed

### End-to-end (`tests/cli.rs`)

With `HOME` pointed at a tempdir:

- a normal payload renders correctly and `config.toml` is created
- a malformed `config.toml` still renders, and logs one warning
- a non-writable data directory still renders
- unset `HOME` still renders
- **stdout is byte-identical to today's output in all of the above** —
  this is the assertion that proves the feature cannot corrupt a prompt

### Concurrency

Rotation races are explicitly **not** unit-tested; a single-process test
cannot reproduce them. Coverage is one integration test that spawns N
concurrent binaries against a small `max_size_bytes` and asserts that
every resulting archive decompresses cleanly.

## Phase 2 detail

`[display]` keys thread through `context_bar::render` and
`layout::compose_statusline`.

The one genuine risk is at `context_bar.rs:21-22`. Today:

```rust
let filled = (used / 10) as usize;
format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
```

This is safe only because both constants are 10 and `used` is clamped to
100. Making the width configurable turns `10 - filled` into
`width - filled`, a `usize` subtraction that **underflows and panics**
whenever rounding pushes `filled` past `width`. Phase 2 computes:

```rust
let filled = ((used as usize) * width / 100).min(width);
```

with `width - filled` as the remainder, and ships tests at `width = 1`,
`width = 0` (clamped to 1), and `used = 100`.

Thresholds are validated as monotonically increasing on load, falling back
to 50/65/80 when they are not.

## Documentation

`README.md:222` reads "There is no config file," which this change makes
false. That line and the Configuration table at `README.md:228-229` are
rewritten into a config-file section carrying the schema, the file
locations table, and the precedence table.

This is a user-visible change to a documented contract and is the item
most likely to be missed in review; the PR description calls it out
explicitly.

`CLAUDE.md`'s "Two runtime dependencies is deliberate" line is updated to
four, preserving the rule that further additions need justification and a
vet entry.
