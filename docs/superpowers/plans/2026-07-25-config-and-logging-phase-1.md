# ferrisbar Config File and JSONL Logging — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give ferrisbar an auto-created TOML config file and a rotating gzip-archived JSONL log, both in platform-appropriate user directories, without changing a single byte of what it prints to stdout.

**Architecture:** Three new modules with one responsibility each and a strict dependency order — `paths.rs` (pure platform directory resolution, env read only by thin wrappers) → `config.rs` (lenient TOML parsing, clamping, template creation) → `log.rs` (JSONL append, size-triggered rotation, gzip archiving under an `O_EXCL` lock). `main.rs` wires them in before reading stdin and converts today's silent `return`s into logged events.

**Tech Stack:** Rust 2021, `serde_json` (existing), `toml` (new, parse-only), `flate2` (new, pure-Rust backend), `tempfile` + `filetime` (existing dev-deps).

**Spec:** `docs/superpowers/specs/2026-07-25-config-and-logging-design.md`

**Scope:** Phase 1 only. The spec's `[display]` block is deliberately *not* implemented and *not* emitted into the generated config template — that is Phase 2 and gets its own plan.

## Global Constraints

Every task's requirements implicitly include this section.

- **MSRV is 1.85.1.** No stdlib API stabilized after it. `just msrv` is the gate.
- **`toml` is pinned to `>=1.1, <1.2`.** Version 1.1.3 declares `rust-version = "1.85"` — a one-patch margin against our 1.85.1. An unbounded range lets `cargo update` break the MSRV gate with no code change to blame.
- **Never panic.** No new `unwrap`, `expect`, `panic!`, slicing by range, or integer subtraction that can underflow on the render path. Partial or wrong-typed input degrades to less output, never to a crash.
- **Nothing new on stdout, ever.** A stray `println!` corrupts the user's prompt on every render. Diagnostics go to the log file or stderr. `setup`'s existing stdout report is the only exception, and it only runs under the `setup` subcommand.
- **Every new failure mode degrades.** Unset base env var, unwritable directory, full disk, malformed config, failed rotation — each disables the offending piece and still prints the statusline.
- **Clippy `pedantic` + `nursery`, CI runs `-D warnings`.** Run `just lint` at the end of every task, not just at the end of the plan. When a lint is genuinely wrong, `#[allow(...)]` it *with a comment explaining why* — see `src/context_bar.rs:12-14` for the house style.
- **Unit tests never mutate process env.** `cargo test` runs a binary's unit tests multi-threaded in one process, so `std::env::set_var` races unrelated tests. Path logic takes its inputs as parameters. Only `tests/cli.rs` sets env, and only per-`Command` on a child process.
- **Conventional Commits.** `release-please` derives the version bump from the prefix. Never hand-edit `version` in `Cargo.toml` or `CHANGELOG.md`.
- **Branch is `feat/config-and-logging`**, already created and holding the two spec commits. Direct pushes to `main` are blocked.

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `src/paths.rs` | Create | Platform config/data dirs. Pure resolvers + thin env wrappers. No I/O. |
| `src/config.rs` | Create | Lenient TOML parse, clamping, template creation. Depends on `paths`. |
| `src/log.rs` | Create | JSONL append, rotation, gzip. Depends on `config`. |
| `src/config_dir.rs` | Modify | `claude_config_dir` gains an override parameter for the precedence chain. |
| `src/setup.rs` | Modify | Relax the both-vars-unset guard; report resolved paths. |
| `src/main.rs` | Modify | Wire modules in; log the degradation events. |
| `tests/cli.rs` | Modify | Retrofit env isolation onto existing tests; add new e2e cases. |
| `Cargo.toml` | Modify | Two new dependencies. |
| `supply-chain/config.toml` | Modify | Vet exemptions for the new tree. |
| `README.md` | Modify | Rewrite the Configuration section. |
| `CLAUDE.md` | Modify | Update the dependency-count invariant. |

**Test invocations** (verified against this repo):
- Unit tests for one module: `cargo test --bin ferrisbar paths::`
- One unit test: `cargo test --bin ferrisbar paths::tests::name -- --exact`
- End-to-end: `cargo test --test cli`
- Everything: `just test`

---

### Task 1: Add dependencies and clear the supply-chain gates

Deliberately first and standalone. The spec requires proving the resolved dependency tree builds at MSRV 1.85.1 *before* any feature code exists, because `toml` sits one patch below our floor — discovering that after writing three modules would be expensive.

**Files:**
- Modify: `Cargo.toml:19-21`
- Modify: `supply-chain/config.toml`

**Interfaces:**
- Consumes: nothing
- Produces: `toml::Table` and `flate2::write::GzEncoder` available to later tasks

- [ ] **Step 1: Add both dependencies**

In `Cargo.toml`, replace the `[dependencies]` block:

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }
# Pinned below 1.2: toml 1.1.3 declares rust-version 1.85, one patch under
# our 1.85.1 MSRV. An unbounded range lets `cargo update` break `just msrv`
# with no code change to attribute it to.
toml = { version = ">=1.1, <1.2", default-features = false, features = ["std", "parse"] }
# rust_backend routes through pure-Rust miniz_oxide instead of libz-sys,
# keeping a C toolchain out of the build and cargo-geiger's unsafe count down.
flate2 = { version = "1", default-features = false, features = ["rust_backend"] }
```

`toml`'s `serde` and `display` features are deliberately off. Config is parsed by hand into `toml::Table` for per-field leniency, and the generated file is a static string — so neither serde integration nor a TOML writer is needed.

- [ ] **Step 2: Resolve the tree and verify MSRV**

```bash
cargo build
just msrv
```

Expected: both succeed. If `just msrv` fails, do not continue — narrow the `toml` bound until the resolved tree builds at 1.85.1, and record the working bound in `Cargo.toml`'s comment.

- [ ] **Step 3: Check the license gate**

```bash
just deny
```

Expected: PASS with no `deny.toml` change. `miniz_oxide` carries `MIT OR Zlib OR Apache-2.0`; the allow-list at `deny.toml:20` satisfies it through the MIT arm. If this fails on a license, add only the specific license needed and note why in a comment.

- [ ] **Step 4: Generate vet exemptions**

```bash
cargo vet suggest
```

Add the suggested `[[exemptions.*]]` entries to `supply-chain/config.toml`, matching the existing style (entries are alphabetical; `criteria` is `safe-to-deploy` for crates in the shipped binary, `safe-to-run` for dev-only). Expect entries for roughly `flate2`, `miniz_oxide`, `crc32fast`, `toml`, `toml_parser`, `toml_datetime`, `serde_spanned`, and `winnow`.

- [ ] **Step 5: Verify the vet gate**

```bash
just vet
```

Expected: PASS.

- [ ] **Step 6: Confirm nothing else regressed**

```bash
just ci
```

Expected: EXIT 0. This is the last point where a green `just ci` is free — take it.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock supply-chain/config.toml
git commit -m "build: add toml and flate2 for config file and log rotation"
```

---

### Task 2: `src/paths.rs` — platform directory resolution

**Files:**
- Create: `src/paths.rs`
- Modify: `src/main.rs:1-6` (add `mod paths;`)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub fn resolve_config_dir(base: Option<&str>, xdg: Option<&str>) -> Option<PathBuf>`
  - `pub fn resolve_data_dir(base: Option<&str>, xdg: Option<&str>) -> Option<PathBuf>`
  - `pub fn config_file() -> Option<PathBuf>` — env-reading wrapper, `<config dir>/config.toml`
  - `pub fn data_dir() -> Option<PathBuf>` — env-reading wrapper
  - `pub fn default_log_path(data_dir: &Path) -> PathBuf` — `<data dir>/logs/ferrisbar.jsonl`

The `base`/`xdg` parameter names are platform-neutral on purpose: `base` is `HOME` on Unix, `%APPDATA%` (config) or `%LOCALAPPDATA%` (data) on Windows. `xdg` is only consulted on the non-macOS, non-Windows branch.

- [ ] **Step 1: Write the failing tests**

Create `src/paths.rs`:

```rust
use std::path::{Path, PathBuf};

const APP_DIR: &str = "ferrisbar";

/// Accepts a directory only when it is present, non-empty, and absolute.
///
/// The absolute check is what keeps a stray `./ferrisbar/` from being
/// created inside whatever repository Claude Code happens to be running in
/// — see the empty-HOME guard in the design spec.
fn usable_dir(raw: Option<&str>) -> Option<PathBuf> {
    let raw = raw?;
    if raw.is_empty() {
        return None;
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_dir_rejects_none_empty_and_relative() {
        assert_eq!(usable_dir(None), None);
        assert_eq!(usable_dir(Some("")), None);
        assert_eq!(usable_dir(Some("relative/path")), None);
    }

    #[test]
    fn usable_dir_accepts_absolute() {
        assert!(usable_dir(Some("/home/someone")).is_some());
    }

    #[test]
    fn config_dir_none_when_base_missing_or_empty() {
        assert_eq!(resolve_config_dir(None, None), None);
        assert_eq!(resolve_config_dir(Some(""), None), None);
        assert_eq!(resolve_data_dir(None, None), None);
        assert_eq!(resolve_data_dir(Some(""), None), None);
    }

    #[test]
    fn dirs_end_in_app_name() {
        let cfg = resolve_config_dir(Some("/base"), None).unwrap();
        let data = resolve_data_dir(Some("/base"), None).unwrap();
        assert_eq!(cfg.file_name().unwrap(), APP_DIR);
        assert_eq!(data.file_name().unwrap(), APP_DIR);
    }

    #[test]
    fn dirs_are_absolute() {
        assert!(resolve_config_dir(Some("/base"), None).unwrap().is_absolute());
        assert!(resolve_data_dir(Some("/base"), None).unwrap().is_absolute());
    }

    #[test]
    fn default_log_path_is_under_logs() {
        let p = default_log_path(Path::new("/data/ferrisbar"));
        assert_eq!(p, PathBuf::from("/data/ferrisbar/logs/ferrisbar.jsonl"));
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    #[test]
    fn xdg_honored_when_absolute_and_ignored_when_relative() {
        assert_eq!(
            resolve_config_dir(Some("/home/u"), Some("/xdg/cfg")),
            Some(PathBuf::from("/xdg/cfg/ferrisbar"))
        );
        assert_eq!(
            resolve_config_dir(Some("/home/u"), Some("rel")),
            Some(PathBuf::from("/home/u/.config/ferrisbar"))
        );
        assert_eq!(
            resolve_config_dir(Some("/home/u"), Some("")),
            Some(PathBuf::from("/home/u/.config/ferrisbar"))
        );
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    #[test]
    fn linux_fallbacks() {
        assert_eq!(
            resolve_config_dir(Some("/home/u"), None),
            Some(PathBuf::from("/home/u/.config/ferrisbar"))
        );
        assert_eq!(
            resolve_data_dir(Some("/home/u"), None),
            Some(PathBuf::from("/home/u/.local/share/ferrisbar"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_application_support_for_both() {
        let expected = PathBuf::from("/Users/u/Library/Application Support/ferrisbar");
        assert_eq!(resolve_config_dir(Some("/Users/u"), None), Some(expected.clone()));
        assert_eq!(resolve_data_dir(Some("/Users/u"), Some("/xdg")), Some(expected));
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_base_directly_and_ignores_xdg() {
        assert_eq!(
            resolve_config_dir(Some(r"C:\Users\u\AppData\Roaming"), Some(r"C:\xdg")),
            Some(PathBuf::from(r"C:\Users\u\AppData\Roaming\ferrisbar"))
        );
    }
}
```

Add `mod paths;` to `src/main.rs` after `mod payload;` (keep the list alphabetical).

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --bin ferrisbar paths::
```

Expected: FAIL — `cannot find function resolve_config_dir in this scope`.

- [ ] **Step 3: Implement the resolvers**

Insert into `src/paths.rs`, after `usable_dir` and before `mod tests`:

```rust
#[cfg(target_os = "macos")]
fn platform_dir(base: Option<&str>, _xdg: Option<&str>, _unix_fallback: &str) -> Option<PathBuf> {
    Some(
        usable_dir(base)?
            .join("Library")
            .join("Application Support")
            .join(APP_DIR),
    )
}

#[cfg(windows)]
fn platform_dir(base: Option<&str>, _xdg: Option<&str>, _unix_fallback: &str) -> Option<PathBuf> {
    Some(usable_dir(base)?.join(APP_DIR))
}

#[cfg(all(not(target_os = "macos"), not(windows)))]
fn platform_dir(base: Option<&str>, xdg: Option<&str>, unix_fallback: &str) -> Option<PathBuf> {
    if let Some(dir) = usable_dir(xdg) {
        return Some(dir.join(APP_DIR));
    }
    let mut path = usable_dir(base)?;
    for part in unix_fallback.split('/') {
        path.push(part);
    }
    Some(path.join(APP_DIR))
}

/// `base` is `$HOME` on Unix and `%APPDATA%` on Windows. `xdg` is
/// `$XDG_CONFIG_HOME` and is consulted only on the XDG branch.
pub fn resolve_config_dir(base: Option<&str>, xdg: Option<&str>) -> Option<PathBuf> {
    platform_dir(base, xdg, ".config")
}

/// `base` is `$HOME` on Unix and `%LOCALAPPDATA%` on Windows. `xdg` is
/// `$XDG_DATA_HOME` and is consulted only on the XDG branch.
pub fn resolve_data_dir(base: Option<&str>, xdg: Option<&str>) -> Option<PathBuf> {
    platform_dir(base, xdg, ".local/share")
}

pub fn default_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("ferrisbar.jsonl")
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --bin ferrisbar paths::
```

Expected: PASS.

- [ ] **Step 5: Add the env-reading wrappers**

These are the only functions in the module that touch process env, and they are never called from a unit test. Append before `mod tests`:

```rust
fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

#[cfg(windows)]
fn base_vars() -> (Option<String>, Option<String>) {
    (env_opt("APPDATA"), env_opt("LOCALAPPDATA"))
}

#[cfg(not(windows))]
fn base_vars() -> (Option<String>, Option<String>) {
    let home = env_opt("HOME");
    (home.clone(), home)
}

/// `<config dir>/config.toml`, or `None` when the platform base directory
/// is unavailable.
pub fn config_file() -> Option<PathBuf> {
    let (config_base, _) = base_vars();
    resolve_config_dir(config_base.as_deref(), env_opt("XDG_CONFIG_HOME").as_deref())
        .map(|d| d.join("config.toml"))
}

/// The data directory, or `None` when the platform base is unavailable.
pub fn data_dir() -> Option<PathBuf> {
    let (_, data_base) = base_vars();
    resolve_data_dir(data_base.as_deref(), env_opt("XDG_DATA_HOME").as_deref())
}
```

- [ ] **Step 6: Lint and test**

```bash
just lint && cargo test --bin ferrisbar paths::
```

Expected: both PASS. If clippy flags `platform_dir`'s unused parameters on the macOS or Windows branch, the leading underscores already handle it; do not delete the parameters, since a uniform signature is what keeps the three branches interchangeable.

- [ ] **Step 7: Commit**

```bash
git add src/paths.rs src/main.rs
git commit -m "feat: resolve platform config and data directories"
```

---

### Task 3: `src/config.rs` — lenient parsing and clamping

Parsing goes through `toml::Table` by hand rather than `#[derive(Deserialize)]`. With derive, one wrong-typed field fails the entire parse; the spec requires that field to fall back while the rest of the file still applies. This mirrors the `lenient_option` deserializer already in `src/payload.rs`.

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs` (add `mod config;`)

**Interfaces:**
- Consumes: `toml` from Task 1
- Produces:
  - `pub struct Config { pub log: LogConfig, pub claude: ClaudeConfig }`
  - `pub struct LogConfig { pub enabled: bool, pub level: String, pub path: String, pub max_size_bytes: u64, pub max_archives: u8 }`
  - `pub struct ClaudeConfig { pub config_dir: String, pub auto_compact_window: f64 }`
  - `impl Default for Config`
  - `pub fn from_toml_str(input: &str) -> (Config, Vec<ParseWarning>)`
  - `pub enum ParseWarning { Syntax(String) }`
  - `pub const TEMPLATE: &str`

- [ ] **Step 1: Write the failing tests**

Create `src/config.rs`:

```rust
/// Written verbatim when no config file exists. A static string rather than
/// a serialized `Config`, because serializing would strip the comments —
/// which are the entire reason TOML was chosen over the already-vendored
/// serde_json.
///
/// The spec's `[display]` block is deliberately absent: Phase 1 does not
/// parse those keys, and emitting keys the binary ignores invites bug
/// reports from users who set `bar_width` and see nothing change.
pub const TEMPLATE: &str = r#"# ferrisbar configuration.  https://github.com/kerryhatcher/ferrisbar
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
"#;

pub const MIN_MAX_SIZE_BYTES: u64 = 4096;
pub const MIN_MAX_ARCHIVES: u8 = 1;
pub const MAX_MAX_ARCHIVES: u8 = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum ParseWarning {
    Syntax(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_values() {
        let c = Config::default();
        assert!(c.log.enabled);
        assert_eq!(c.log.level, "warn");
        assert_eq!(c.log.path, "");
        assert_eq!(c.log.max_size_bytes, 1_048_576);
        assert_eq!(c.log.max_archives, 7);
        assert_eq!(c.claude.config_dir, "");
        assert!((c.claude.auto_compact_window - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn template_round_trips_to_the_defaults() {
        let (c, warnings) = from_toml_str(TEMPLATE);
        assert!(warnings.is_empty(), "template must parse cleanly");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn template_does_not_mention_display() {
        assert!(!TEMPLATE.contains("[display]"));
        assert!(!TEMPLATE.contains("bar_width"));
    }

    #[test]
    fn empty_input_yields_defaults_without_warning() {
        let (c, warnings) = from_toml_str("");
        assert_eq!(c, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn malformed_toml_yields_defaults_and_exactly_one_warning() {
        let (c, warnings) = from_toml_str("this is not = = toml");
        assert_eq!(c, Config::default());
        assert_eq!(warnings.len(), 1, "one warning per render, never one per key");
        assert!(matches!(warnings[0], ParseWarning::Syntax(_)));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let (c, warnings) = from_toml_str("[log]\nfrom_the_future = 42\n");
        assert_eq!(c, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn wrong_typed_field_falls_back_without_discarding_the_rest() {
        let (c, _) = from_toml_str("[log]\nenabled = \"yes\"\nmax_archives = 3\n");
        assert!(c.log.enabled, "wrong-typed field falls back to its default");
        assert_eq!(c.log.max_archives, 3, "sibling keys still apply");
    }

    #[test]
    fn values_are_read_from_every_section() {
        let (c, _) = from_toml_str(
            "[log]\nenabled = false\nlevel = \"debug\"\npath = \"/tmp/x.jsonl\"\n\
             [claude]\nconfig_dir = \"/c\"\nauto_compact_window = 150000\n",
        );
        assert!(!c.log.enabled);
        assert_eq!(c.log.level, "debug");
        assert_eq!(c.log.path, "/tmp/x.jsonl");
        assert_eq!(c.claude.config_dir, "/c");
        assert!((c.claude.auto_compact_window - 150_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn max_size_bytes_clamps_up_from_zero() {
        let (c, _) = from_toml_str("[log]\nmax_size_bytes = 0\n");
        assert_eq!(c.log.max_size_bytes, MIN_MAX_SIZE_BYTES);
    }

    #[test]
    fn max_archives_clamps_at_both_ends() {
        let (zero, _) = from_toml_str("[log]\nmax_archives = 0\n");
        assert_eq!(zero.log.max_archives, MIN_MAX_ARCHIVES);
        let (huge, _) = from_toml_str("[log]\nmax_archives = 999\n");
        assert_eq!(huge.log.max_archives, MAX_MAX_ARCHIVES);
    }

    #[test]
    fn negative_numbers_fall_back_to_defaults() {
        let (c, _) = from_toml_str("[log]\nmax_size_bytes = -1\nmax_archives = -5\n");
        assert_eq!(c.log.max_size_bytes, Config::default().log.max_size_bytes);
        assert_eq!(c.log.max_archives, Config::default().log.max_archives);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --bin ferrisbar config::
```

Expected: FAIL — `cannot find type Config in this scope`.

- [ ] **Step 3: Implement the types and parser**

Insert after the `ParseWarning` enum, before `mod tests`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct LogConfig {
    pub enabled: bool,
    pub level: String,
    pub path: String,
    pub max_size_bytes: u64,
    pub max_archives: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeConfig {
    pub config_dir: String,
    pub auto_compact_window: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub log: LogConfig,
    pub claude: ClaudeConfig,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "warn".to_string(),
            path: String::new(),
            max_size_bytes: 1_048_576,
            max_archives: 7,
        }
    }
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            config_dir: String::new(),
            auto_compact_window: 0.0,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log: LogConfig::default(),
            claude: ClaudeConfig::default(),
        }
    }
}

fn section<'a>(table: &'a toml::Table, name: &str) -> Option<&'a toml::Table> {
    table.get(name).and_then(toml::Value::as_table)
}

fn get_bool(section: Option<&toml::Table>, key: &str) -> Option<bool> {
    section?.get(key)?.as_bool()
}

fn get_string(section: Option<&toml::Table>, key: &str) -> Option<String> {
    section?.get(key)?.as_str().map(str::to_string)
}

fn get_integer(section: Option<&toml::Table>, key: &str) -> Option<i64> {
    section?.get(key)?.as_integer()
}

/// Accepts both `150000` and `150000.0`, since a user hand-editing a token
/// count has no reason to know which the parser wants.
//
// Safe: an i64 token count converts to f64 with precision loss only beyond
// 2^53, far above any real context window. The allow sits on the function
// because attributes on bare expressions are not stable.
#[allow(clippy::cast_precision_loss)]
fn get_number(section: Option<&toml::Table>, key: &str) -> Option<f64> {
    let value = section?.get(key)?;
    value
        .as_float()
        .or_else(|| value.as_integer().map(|i| i as f64))
}

/// Parses leniently: an unreadable field falls back to its default and its
/// siblings still apply. Returns at most one warning — a garbage file must
/// not produce a burst of log lines on every render.
pub fn from_toml_str(input: &str) -> (Config, Vec<ParseWarning>) {
    let table = match input.parse::<toml::Table>() {
        Ok(table) => table,
        Err(e) => return (Config::default(), vec![ParseWarning::Syntax(e.to_string())]),
    };

    let defaults = Config::default();
    let log = section(&table, "log");
    let claude = section(&table, "claude");

    let max_size_bytes = get_integer(log, "max_size_bytes")
        .and_then(|v| u64::try_from(v).ok())
        .map_or(defaults.log.max_size_bytes, |v| {
            v.max(MIN_MAX_SIZE_BYTES)
        });

    let max_archives = get_integer(log, "max_archives")
        .and_then(|v| u8::try_from(v).ok())
        .map_or(defaults.log.max_archives, |v| {
            v.clamp(MIN_MAX_ARCHIVES, MAX_MAX_ARCHIVES)
        });

    let config = Config {
        log: LogConfig {
            enabled: get_bool(log, "enabled").unwrap_or(defaults.log.enabled),
            level: get_string(log, "level").unwrap_or(defaults.log.level),
            path: get_string(log, "path").unwrap_or(defaults.log.path),
            max_size_bytes,
            max_archives,
        },
        claude: ClaudeConfig {
            config_dir: get_string(claude, "config_dir").unwrap_or(defaults.claude.config_dir),
            auto_compact_window: get_number(claude, "auto_compact_window")
                .unwrap_or(defaults.claude.auto_compact_window),
        },
    };

    (config, Vec::new())
}
```

Note the clamp asymmetry, which the tests pin: `max_archives = 999` clamps *down* to 64 because it still fits a `u8`, while `max_archives = -5` fails `u8::try_from` and falls back to the default 7. Both are correct — a negative value is nonsense rather than an out-of-range intent.

Add `mod config;` to `src/main.rs`.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --bin ferrisbar config::
```

Expected: PASS, all 11 tests.

- [ ] **Step 5: Lint**

```bash
just lint
```

Expected: PASS. Clippy `nursery` may suggest `derivable_impls` on `Default for Config`; it is not derivable because `LogConfig` and `ClaudeConfig` have non-`Default` field values, so if it fires on the `Config` impl specifically, `#[allow]` it with a comment noting the nested impls carry the real defaults.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: parse ferrisbar config from TOML with per-field fallback"
```

---

### Task 4: `src/config.rs` — load-or-create from disk

Splitting file I/O from parsing keeps Task 3's tests pure and makes this task's tests exclusively about filesystem behavior.

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Consumes: `from_toml_str`, `TEMPLATE`, `ParseWarning` from Task 3
- Produces: `pub fn load(path: Option<&Path>) -> (Config, Vec<ParseWarning>)`, and `ParseWarning` gains a `Create(String)` variant

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src/config.rs`:

```rust
#[test]
fn load_with_no_path_yields_defaults_silently() {
    let (c, warnings) = load(None);
    assert_eq!(c, Config::default());
    assert!(warnings.is_empty(), "an unresolvable home is not a fault to report");
}

#[test]
fn load_creates_the_file_and_its_parent_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("config.toml");

    let (c, warnings) = load(Some(&path));

    assert!(path.exists(), "config file must be created on first run");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
    assert_eq!(c, Config::default());
    assert!(warnings.is_empty());
}

#[test]
fn load_reads_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[log]\nmax_archives = 3\n").unwrap();

    let (c, warnings) = load(Some(&path));

    assert_eq!(c.log.max_archives, 3);
    assert!(warnings.is_empty());
}

#[test]
fn load_does_not_overwrite_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let original = "[log]\nmax_archives = 3\n";
    std::fs::write(&path, original).unwrap();

    let _ = load(Some(&path));

    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn load_of_malformed_file_warns_once_and_leaves_it_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let original = "not = = toml";
    std::fs::write(&path, original).unwrap();

    let (c, warnings) = load(Some(&path));

    assert_eq!(c, Config::default());
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        original,
        "a malformed config is never overwritten — the user's edits are theirs"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --bin ferrisbar config::tests::load
```

Expected: FAIL — `cannot find function load in this scope`.

- [ ] **Step 3: Implement `load`**

Extend the `ParseWarning` enum:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ParseWarning {
    Syntax(String),
    Create(String),
}
```

Add `use std::path::Path;` at the top of the file, and insert `load` before `mod tests`:

```rust
/// Reads the config file, creating it from `TEMPLATE` when absent.
///
/// Infallible by construction: a missing home directory, an unreadable
/// file, a read-only directory, or malformed TOML all yield defaults and
/// let the statusline render. Warnings are returned as data rather than
/// logged directly, because the config is what determines where the log
/// lives — the caller flushes them once the logger exists.
pub fn load(path: Option<&Path>) -> (Config, Vec<ParseWarning>) {
    let Some(path) = path else {
        return (Config::default(), Vec::new());
    };

    match std::fs::read_to_string(path) {
        Ok(contents) => from_toml_str(&contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut warnings = Vec::new();
            if let Err(e) = create_template(path) {
                warnings.push(ParseWarning::Create(e.to_string()));
            }
            (Config::default(), warnings)
        }
        Err(e) => (Config::default(), vec![ParseWarning::Create(e.to_string())]),
    }
}

fn create_template(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, TEMPLATE)
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --bin ferrisbar config::
```

Expected: PASS, all 16 tests.

- [ ] **Step 5: Lint and run the full suite**

```bash
just lint && just test
```

Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat: create the config file from a template on first run"
```

---

### Task 5: `src/log.rs` — JSONL events and appending

Rotation is deliberately held back to Task 6 so this task's tests cover formatting and level filtering in isolation.

**Files:**
- Create: `src/log.rs`
- Modify: `src/main.rs` (add `mod log;`)

**Interfaces:**
- Consumes: `config::Config`, `paths::default_log_path`
- Produces:
  - `pub enum Level { Off, Warn, Debug }` with `pub fn from_str_lenient(&str) -> Level`
  - `pub struct Event { pub level: Level, pub name: &'static str, pub session_id: Option<String>, pub msg: String }` plus `pub fn warn(name, msg) -> Event` and `pub fn debug(name, msg) -> Event`
  - `pub struct Logger` with `pub fn new(cfg: &config::Config, data_dir: Option<&Path>) -> Logger` and `pub fn log(&self, event: &Event)`
  - `pub fn line_for(event: &Event, ts_millis: u128) -> String`

`line_for` takes the timestamp as a parameter so the serialization is testable without a clock.

- [ ] **Step 1: Write the failing tests**

Create `src/log.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn read_lines(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn level_parses_leniently_and_defaults_to_warn() {
        assert_eq!(Level::from_str_lenient("off"), Level::Off);
        assert_eq!(Level::from_str_lenient("warn"), Level::Warn);
        assert_eq!(Level::from_str_lenient("debug"), Level::Debug);
        assert_eq!(Level::from_str_lenient("DEBUG"), Level::Debug);
        assert_eq!(Level::from_str_lenient("nonsense"), Level::Warn);
        assert_eq!(Level::from_str_lenient(""), Level::Warn);
    }

    #[test]
    fn line_is_one_json_object_with_the_expected_fields() {
        let event = warn("stdin_parse_failed", "expected value");
        let line = line_for(&event, 1_753_467_296_123);

        assert!(!line.contains('\n'), "a JSONL record is exactly one line");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["ts"], 1_753_467_296_123_u64);
        assert_eq!(v["level"], "warn");
        assert_eq!(v["event"], "stdin_parse_failed");
        assert_eq!(v["msg"], "expected value");
        assert!(v.get("session_id").is_none(), "omitted when unknown");
    }

    #[test]
    fn line_includes_session_id_when_present() {
        let mut event = warn("todo_file_unreadable", "no such file");
        event.session_id = Some("abc123".to_string());
        let v: serde_json::Value = serde_json::from_str(&line_for(&event, 1)).unwrap();
        assert_eq!(v["session_id"], "abc123");
    }

    #[test]
    fn disabled_logging_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.enabled = false;

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(!dir.path().join("logs").exists(), "no directory is created either");
    }

    #[test]
    fn level_off_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.level = "off".to_string();

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(read_lines(&crate::paths::default_log_path(dir.path())).is_empty());
    }

    #[test]
    fn warn_level_suppresses_debug_events_but_keeps_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let logger = Logger::new(&Config::default(), Some(dir.path()));

        logger.log(&debug("render", "should not appear"));
        logger.log(&warn("stdin_parse_failed", "should appear"));

        let lines = read_lines(&crate::paths::default_log_path(dir.path()));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("stdin_parse_failed"));
    }

    #[test]
    fn debug_level_keeps_both() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.level = "debug".to_string();
        let logger = Logger::new(&cfg, Some(dir.path()));

        logger.log(&debug("render", "a"));
        logger.log(&warn("stdin_parse_failed", "b"));

        assert_eq!(read_lines(&crate::paths::default_log_path(dir.path())).len(), 2);
    }

    #[test]
    fn appends_rather_than_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let logger = Logger::new(&Config::default(), Some(dir.path()));

        logger.log(&warn("stdin_parse_failed", "first"));
        logger.log(&warn("stdin_read_failed", "second"));

        let lines = read_lines(&crate::paths::default_log_path(dir.path()));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("first"));
        assert!(lines[1].contains("second"));
    }

    #[test]
    fn explicit_relative_path_resolves_against_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.path = "custom.jsonl".to_string();

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(
            dir.path().join("custom.jsonl").exists(),
            "relative paths must not resolve against the process cwd, \
             which is whatever project Claude Code is running in"
        );
    }

    #[test]
    fn explicit_absolute_path_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.jsonl");
        let mut cfg = Config::default();
        cfg.log.path = target.to_string_lossy().into_owned();

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(target.exists());
    }

    #[test]
    fn no_data_dir_disables_logging_without_erroring() {
        let logger = Logger::new(&Config::default(), None);
        logger.log(&warn("stdin_parse_failed", "x"));
        // Reaching this line without a panic is the assertion.
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --bin ferrisbar log::
```

Expected: FAIL — `cannot find type Level in this scope`.

- [ ] **Step 3: Implement**

Insert at the top of `src/log.rs`, before `mod tests`:

```rust
use crate::config::Config;
use crate::paths;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off,
    Warn,
    Debug,
}

impl Level {
    /// Unknown values fall back to `Warn` rather than erroring — a typo in
    /// the level should leave the user with the safe default, not silence.
    pub fn from_str_lenient(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Self::Off,
            "debug" => Self::Debug,
            _ => Self::Warn,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Debug => "debug",
        }
    }
}

pub struct Event {
    pub level: Level,
    pub name: &'static str,
    pub session_id: Option<String>,
    pub msg: String,
}

pub fn warn(name: &'static str, msg: impl Into<String>) -> Event {
    Event { level: Level::Warn, name, session_id: None, msg: msg.into() }
}

pub fn debug(name: &'static str, msg: impl Into<String>) -> Event {
    Event { level: Level::Debug, name, session_id: None, msg: msg.into() }
}

/// Timestamp is a parameter so serialization is testable without a clock.
pub fn line_for(event: &Event, ts_millis: u128) -> String {
    let mut map = serde_json::Map::new();
    map.insert("ts".to_string(), serde_json::json!(ts_millis));
    map.insert("level".to_string(), serde_json::json!(event.level.as_str()));
    map.insert("event".to_string(), serde_json::json!(event.name));
    if let Some(id) = &event.session_id {
        map.insert("session_id".to_string(), serde_json::json!(id));
    }
    map.insert("msg".to_string(), serde_json::json!(event.msg));
    serde_json::Value::Object(map).to_string()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

pub struct Logger {
    path: Option<PathBuf>,
    level: Level,
    max_size_bytes: u64,
    max_archives: u8,
}

impl Logger {
    pub fn new(cfg: &Config, data_dir: Option<&Path>) -> Self {
        let level = Level::from_str_lenient(&cfg.log.level);
        let path = if cfg.log.enabled && level != Level::Off {
            resolve_log_path(&cfg.log.path, data_dir)
        } else {
            None
        };
        Self {
            path,
            level,
            max_size_bytes: cfg.log.max_size_bytes,
            max_archives: cfg.log.max_archives,
        }
    }

    /// Never returns an error and never panics. Every failure — no data
    /// directory, unwritable path, full disk — silently disables this write
    /// so the statusline still renders.
    pub fn log(&self, event: &Event) {
        let Some(path) = &self.path else { return };
        if self.level == Level::Off || event.level > self.level {
            return;
        }
        let _ = self.append(path, event);
    }

    fn append(&self, path: &Path, event: &Event) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{}", line_for(event, now_millis()))
    }
}

/// An empty configured path means "use the default under the data dir". A
/// relative path resolves against the data dir, never the process working
/// directory — the cwd is whatever project Claude Code is running in, and
/// resolving there would scatter log files across repositories.
fn resolve_log_path(configured: &str, data_dir: Option<&Path>) -> Option<PathBuf> {
    if configured.is_empty() {
        return data_dir.map(paths::default_log_path);
    }
    let candidate = Path::new(configured);
    if candidate.is_absolute() {
        return Some(candidate.to_path_buf());
    }
    data_dir.map(|d| d.join(candidate))
}
```

Add `mod log;` to `src/main.rs`.

Note `Level`'s derived `Ord`: variants are declared `Off < Warn < Debug`, so `event.level > self.level` filters a `Debug` event out at `Warn` level. Do not reorder the variants.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --bin ferrisbar log::
```

Expected: PASS, all 11 tests.

- [ ] **Step 5: Lint**

```bash
just lint
```

Expected: PASS. `max_size_bytes` and `max_archives` are stored but not yet read, so clippy's `dead_code` may fire. Leave them — Task 6 reads both, and Task 6's tests set them through `Config` rather than touching the fields directly. If `-D warnings` blocks the commit, add `#[allow(dead_code)] // read by rotate_if_needed in the next commit` and delete the allow in Task 6.

- [ ] **Step 6: Commit**

```bash
git add src/log.rs src/main.rs
git commit -m "feat: append JSONL diagnostic events to a log file"
```

---

### Task 6: `src/log.rs` — rotation, locking, and gzip archives

**Files:**
- Modify: `src/log.rs`

**Interfaces:**
- Consumes: `Logger`, `Event` from Task 5
- Produces: `fn rotate_if_needed(&self, path: &Path)` called from `Logger::append`; `fn archive_path(base: &Path, n: u8) -> PathBuf`

**Critical ordering — read before implementing.** The sequence is **lock → stat → rotate → open → append**, never open-then-lock. Several Claude Code sessions run ferrisbar concurrently; a process that opened the log before another renamed it holds a descriptor to the old inode, and appending through it writes into the file being archived. Because `Logger::append` opens by path on every write and this task inserts the rotation check *before* that open, the requirement is satisfied structurally — do not restructure `append` to hold a long-lived handle.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src/log.rs`:

```rust
use std::io::Read as _;

fn tiny_logger(dir: &std::path::Path, max_archives: u8) -> Logger {
    let mut cfg = Config::default();
    cfg.log.max_size_bytes = 4096;
    cfg.log.max_archives = max_archives;
    Logger::new(&cfg, Some(dir))
}

fn gunzip(path: &std::path::Path) -> String {
    let file = std::fs::File::open(path).unwrap();
    let mut out = String::new();
    flate2::read::GzDecoder::new(file).read_to_string(&mut out).unwrap();
    out
}

#[test]
fn no_rotation_below_the_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 7);
    let base = crate::paths::default_log_path(dir.path());

    logger.log(&warn("stdin_parse_failed", "small"));

    assert!(base.exists());
    assert!(!archive_path(&base, 1).exists(), "must not rotate under the limit");
}

#[test]
fn rotation_archives_the_full_file_and_starts_a_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 7);
    let base = crate::paths::default_log_path(dir.path());

    let filler = "x".repeat(5000);
    logger.log(&warn("stdin_parse_failed", filler.clone()));
    logger.log(&warn("stdin_read_failed", "after rotation"));

    assert!(archive_path(&base, 1).exists(), "archive .1.gz must exist");
    let current = std::fs::read_to_string(&base).unwrap();
    assert!(current.contains("after rotation"));
    assert!(!current.contains(&filler), "the big line moved into the archive");
}

#[test]
fn archive_decompresses_to_exactly_the_bytes_written() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 7);
    let base = crate::paths::default_log_path(dir.path());

    logger.log(&warn("stdin_parse_failed", "y".repeat(5000)));
    let before_rotation = std::fs::read_to_string(&base).unwrap();
    logger.log(&warn("stdin_read_failed", "trigger"));

    assert_eq!(gunzip(&archive_path(&base, 1)), before_rotation);
}

#[test]
fn archives_shift_and_cap_at_max_archives() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 2);
    let base = crate::paths::default_log_path(dir.path());

    for i in 0..4 {
        logger.log(&warn("stdin_parse_failed", format!("gen{i}{}", "z".repeat(5000))));
    }

    assert!(archive_path(&base, 1).exists());
    assert!(archive_path(&base, 2).exists());
    assert!(!archive_path(&base, 3).exists(), "oldest generation is dropped");
    assert!(
        gunzip(&archive_path(&base, 1)).contains("gen2"),
        ".1.gz always holds the most recent archive"
    );
}

#[test]
fn a_held_lock_defers_rotation_without_losing_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 7);
    let base = crate::paths::default_log_path(dir.path());

    logger.log(&warn("stdin_parse_failed", "w".repeat(5000)));
    // Simulate another live process mid-rotation.
    let lock = base.parent().unwrap().join(".rotate.lock");
    std::fs::write(&lock, "").unwrap();

    logger.log(&warn("stdin_read_failed", "deferred"));

    assert!(!archive_path(&base, 1).exists(), "loser must not rotate");
    assert!(
        std::fs::read_to_string(&base).unwrap().contains("deferred"),
        "loser still appends — the line is never dropped"
    );
}

#[test]
fn a_stale_lock_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 7);
    let base = crate::paths::default_log_path(dir.path());

    logger.log(&warn("stdin_parse_failed", "v".repeat(5000)));
    let lock = base.parent().unwrap().join(".rotate.lock");
    std::fs::write(&lock, "").unwrap();
    let stale = filetime::FileTime::from_unix_time(
        filetime::FileTime::now().unix_seconds() - 120,
        0,
    );
    filetime::set_file_mtime(&lock, stale).unwrap();

    logger.log(&warn("stdin_read_failed", "after reclaim"));

    assert!(archive_path(&base, 1).exists(), "a lock older than 60s is reclaimed");
}

/// Unix-only: the test wedges rotation by making the archive paths
/// directories, and `rename`-onto-non-empty-directory and
/// `File::create`-on-a-directory have different error semantics on Windows.
/// The production code path is platform-independent; only this way of
/// provoking a failure is not.
#[cfg(unix)]
#[test]
fn a_failed_rotation_is_reported_and_the_records_survive() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 3);
    let base = crate::paths::default_log_path(dir.path());

    logger.log(&warn("stdin_parse_failed", "t".repeat(5000)));

    // .1.gz is a directory, so gzip cannot create it. .2.gz and .3.gz are
    // non-empty directories, so the shift loop cannot move .1.gz out of
    // the way first.
    std::fs::create_dir_all(archive_path(&base, 1)).unwrap();
    std::fs::create_dir_all(archive_path(&base, 2).join("occupied")).unwrap();
    std::fs::create_dir_all(archive_path(&base, 3).join("occupied")).unwrap();

    logger.log(&warn("stdin_read_failed", "trigger"));

    let current = std::fs::read_to_string(&base).unwrap();
    assert!(current.contains("log_rotate_failed"), "the failure is reported");
    assert!(current.contains("trigger"), "the triggering line is still written");
    assert!(current.contains("ttttt"), "the staged file was restored, not lost");
    assert!(
        !base.parent().unwrap().join(".rotate.lock").exists(),
        "the guard releases the lock even on the failure path"
    );
}

#[test]
fn the_lock_is_released_after_a_successful_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let logger = tiny_logger(dir.path(), 7);
    let base = crate::paths::default_log_path(dir.path());

    logger.log(&warn("stdin_parse_failed", "u".repeat(5000)));
    logger.log(&warn("stdin_read_failed", "trigger"));

    assert!(
        !base.parent().unwrap().join(".rotate.lock").exists(),
        "a leaked lock would block rotation for the next 60 seconds"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --bin ferrisbar log::tests
```

Expected: FAIL — `cannot find function archive_path in this scope`.

- [ ] **Step 3: Implement rotation**

Add to the imports at the top of `src/log.rs`:

```rust
use flate2::write::GzEncoder;
use flate2::Compression;
use std::time::Duration;
```

Insert before `mod tests`:

```rust
const LOCK_STALE_AFTER: Duration = Duration::from_secs(60);

pub(crate) fn archive_path(base: &Path, n: u8) -> PathBuf {
    let mut name = base.as_os_str().to_os_string();
    name.push(format!(".{n}.gz"));
    PathBuf::from(name)
}

/// Removes the lock file on drop so a rotation that fails partway through
/// does not block every later rotation for 60 seconds.
struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `create_new` is an atomic `O_EXCL` create: exactly one concurrent
/// process gets the lock.
///
/// A lock older than `LOCK_STALE_AFTER` is assumed to be from a process
/// that died mid-rotation and is reclaimed. Two processes can both judge
/// the same lock stale and both proceed; the consequence is a dropped
/// archive generation rather than corruption, and it is accepted rather
/// than engineered around.
fn acquire_lock(dir: &Path) -> Option<LockGuard> {
    let path = dir.join(".rotate.lock");
    match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(_) => return Some(LockGuard(path)),
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return None,
        Err(_) => {}
    }

    let stale = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .and_then(|t| SystemTime::now().duration_since(t).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "clock went backwards")
        }))
        .is_ok_and(|age| age > LOCK_STALE_AFTER);

    if !stale {
        return None;
    }
    std::fs::remove_file(&path).ok()?;
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .ok()
        .map(|_| LockGuard(path))
}

fn gzip_into(source: &Path, dest: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(source)?;
    let mut encoder = GzEncoder::new(std::fs::File::create(dest)?, Compression::default());
    std::io::copy(&mut input, &mut encoder)?;
    encoder.finish()?;
    Ok(())
}

impl Logger {
    /// Ordering is load-bearing: lock, then stat, then rotate. The caller
    /// opens the log only after this returns, so no descriptor can outlive
    /// the rename.
    ///
    /// Returns a message when rotation was attempted and failed. Deferring
    /// the report to the caller keeps this function from calling `log`,
    /// which would re-enter rotation.
    fn rotate_if_needed(&self, path: &Path) -> Option<String> {
        let dir = path.parent()?;
        let over_limit = std::fs::metadata(path).is_ok_and(|m| m.len() >= self.max_size_bytes);
        if !over_limit {
            return None;
        }
        // Lock held for the whole rotation; released on drop.
        let _guard = acquire_lock(dir)?;

        // Re-check under the lock: another process may have just rotated.
        if !std::fs::metadata(path).is_ok_and(|m| m.len() >= self.max_size_bytes) {
            return None;
        }

        // Shift downward from the oldest, so .N.gz is overwritten rather
        // than accumulating past max_archives.
        for n in (1..self.max_archives).rev() {
            let _ = std::fs::rename(archive_path(path, n), archive_path(path, n + 1));
        }

        let staged = path.with_extension("rotating");
        if let Err(e) = std::fs::rename(path, &staged) {
            return Some(format!("staging rename failed: {e}"));
        }
        if let Err(e) = gzip_into(&staged, &archive_path(path, 1)) {
            // Put it back rather than losing the records outright.
            let _ = std::fs::rename(&staged, path);
            return Some(format!("gzip failed: {e}"));
        }
        let _ = std::fs::remove_file(&staged);
        None
    }
}
```

Then call it from `append`, immediately before the file is opened:

```rust
    fn append(&self, path: &Path, event: &Event) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let rotate_error = self.rotate_if_needed(path);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        // Written inline rather than via `log`, which would recurse back
        // into rotation. The file is known to be over its limit here, so
        // this line is the one permitted overshoot.
        if let Some(msg) = rotate_error {
            let event = warn("log_rotate_failed", msg);
            writeln!(file, "{}", line_for(&event, now_millis()))?;
        }
        writeln!(file, "{}", line_for(event, now_millis()))
    }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --bin ferrisbar log::
```

Expected: PASS — 19 tests on Unix, 18 on Windows (one is `cfg(unix)`).

- [ ] **Step 5: Add the concurrency test**

Rotation races cannot be reproduced in-process. Add to `tests/cli.rs` — it will be wired to the tempdir helper in Task 9, so for now construct the environment inline:

```rust
#[test]
fn concurrent_renders_never_produce_a_corrupt_archive() {
    use std::io::Read as _;

    let home = tempfile::tempdir().unwrap();
    let config_dir = home.path().join(".config").join("ferrisbar");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[log]\nlevel = \"debug\"\nmax_size_bytes = 4096\nmax_archives = 5\n",
    )
    .unwrap();

    let mut children = Vec::new();
    for _ in 0..12 {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ferrisbar"))
            .env("HOME", home.path())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env("APPDATA", home.path())
            .env("LOCALAPPDATA", home.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write as _;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"model":{"display_name":"Claude"},"session_id":"s"}"#)
            .unwrap();
        children.push(child);
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }

    // Every archive that exists must decompress cleanly.
    let logs = home.path().join(".local").join("share").join("ferrisbar").join("logs");
    for entry in std::fs::read_dir(&logs).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "gz") {
            let mut out = String::new();
            flate2::read::GzDecoder::new(std::fs::File::open(&path).unwrap())
                .read_to_string(&mut out)
                .unwrap_or_else(|e| panic!("{} is corrupt: {e}", path.display()));
        }
    }
    assert!(!logs.join(".rotate.lock").exists(), "no lock may be left behind");
}
```

`flate2` is a runtime dependency, so it is available to integration tests without a dev-dependency entry.

- [ ] **Step 6: Run the concurrency test**

```bash
cargo test --test cli concurrent_renders
```

Expected: PASS. If it fails intermittently, the ordering in `rotate_if_needed` is wrong — re-read the ordering note at the top of this task rather than adding retries.

- [ ] **Step 7: Lint and run the full suite**

```bash
just lint && just test
```

Expected: both PASS.

- [ ] **Step 8: Commit**

```bash
git add src/log.rs tests/cli.rs
git commit -m "feat: rotate the log at a size limit into gzip archives"
```

---

### Task 7: Precedence wiring in `config_dir.rs` and `setup.rs`

**Files:**
- Modify: `src/config_dir.rs:6-14`
- Modify: `src/setup.rs:7-23`, `src/setup.rs:69-87`

**Interfaces:**
- Consumes: `config::Config` from Task 4
- Produces: `pub fn claude_config_dir(file_override: Option<&str>) -> PathBuf`; `setup::run(project_scope: bool, cfg: &Config) -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

Replace the contents of `src/config_dir.rs`'s (currently absent) test module by appending:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_override_used_when_env_is_absent() {
        assert_eq!(
            resolve(None, Some("/from/config"), Some("/home/u")),
            PathBuf::from("/from/config")
        );
    }

    #[test]
    fn env_beats_the_file() {
        assert_eq!(
            resolve(Some("/from/env"), Some("/from/config"), Some("/home/u")),
            PathBuf::from("/from/env"),
            "an exported CLAUDE_CONFIG_DIR must keep winning for existing users"
        );
    }

    #[test]
    fn empty_values_are_treated_as_unset_at_every_layer() {
        assert_eq!(
            resolve(Some(""), Some("/from/config"), Some("/home/u")),
            PathBuf::from("/from/config")
        );
        assert_eq!(
            resolve(Some(""), Some(""), Some("/home/u")),
            PathBuf::from("/home/u/.claude")
        );
    }

    #[test]
    fn falls_back_to_home_dot_claude() {
        assert_eq!(
            resolve(None, None, Some("/home/u")),
            PathBuf::from("/home/u/.claude")
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --bin ferrisbar config_dir::
```

Expected: FAIL — `cannot find function resolve in this scope`.

- [ ] **Step 3: Implement the precedence chain**

Replace `src/config_dir.rs`'s function with a pure resolver plus an env wrapper:

```rust
use std::env;
use std::path::PathBuf;

fn non_empty(raw: Option<&str>) -> Option<&str> {
    raw.filter(|v| !v.is_empty())
}

/// Precedence: `$CLAUDE_CONFIG_DIR`, then the config file's
/// `claude.config_dir`, then `$HOME/.claude`.
///
/// Environment beats file deliberately — inverting it would silently stop
/// an exported `CLAUDE_CONFIG_DIR` from working for existing users.
fn resolve(env_value: Option<&str>, file_value: Option<&str>, home: Option<&str>) -> PathBuf {
    if let Some(dir) = non_empty(env_value) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = non_empty(file_value) {
        return PathBuf::from(dir);
    }
    PathBuf::from(home.unwrap_or_default()).join(".claude")
}

/// Resolves Claude Code's config directory.
pub fn claude_config_dir(file_override: Option<&str>) -> PathBuf {
    resolve(
        env::var("CLAUDE_CONFIG_DIR").ok().as_deref(),
        file_override,
        env::var("HOME").ok().as_deref(),
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --bin ferrisbar config_dir::
```

Expected: PASS, all 4 tests.

- [ ] **Step 5: Relax the `setup.rs` guard**

`src/setup.rs:14-21` currently errors when neither `$CLAUDE_CONFIG_DIR` nor `$HOME` is set. With `claude.config_dir` in play, that rejects a case that is now valid. Change `resolve_settings_path` to take the config value and accept it as a third source:

```rust
fn resolve_settings_path(project_scope: bool, file_override: Option<&str>) -> Result<PathBuf, String> {
    if project_scope {
        let cwd = env::current_dir()
            .map_err(|e| format!("failed to determine the current directory: {e}"))?;
        return Ok(cwd.join(".claude").join("settings.local.json"));
    }

    let has_config_dir = env::var("CLAUDE_CONFIG_DIR").is_ok_and(|v| !v.is_empty());
    let has_file_override = file_override.is_some_and(|v| !v.is_empty());
    let has_home = env::var("HOME").is_ok_and(|v| !v.is_empty());
    if !has_config_dir && !has_file_override && !has_home {
        return Err(
            "Cannot resolve the Claude Code config directory: none of $CLAUDE_CONFIG_DIR, \
             claude.config_dir in config.toml, or $HOME is set."
                .to_string(),
        );
    }
    Ok(config_dir::claude_config_dir(file_override).join("settings.json"))
}
```

Update `run` to accept and forward the config, and to report the resolved paths:

```rust
pub fn run(project_scope: bool, cfg: &crate::config::Config) -> Result<(), String> {
    let file_override = Some(cfg.claude.config_dir.as_str());
    let settings_path = resolve_settings_path(project_scope, file_override)?;
    let new_command = env::current_exe()
        .map_err(|e| format!("failed to resolve the current executable path: {e}"))?
        .to_string_lossy()
        .into_owned();

    let previous = apply_statusline_update(&settings_path, &new_command)?;

    println!("Updated statusLine in {}", settings_path.display());
    match previous {
        Some(before) => println!("  before: {before}"),
        None => println!("  before: (none)"),
    }
    println!("  after:  {new_command}");
    if let Some(path) = crate::paths::config_file() {
        println!("Config: {}", path.display());
    }
    if let Some(dir) = crate::paths::data_dir() {
        println!("Log:    {}", crate::paths::default_log_path(&dir).display());
    }
    println!("Start a new Claude Code session for the change to take effect.");

    Ok(())
}
```

- [ ] **Step 6: Fix the call sites and run everything**

`src/main.rs:14` becomes `config_dir::claude_config_dir(Some(&cfg.claude.config_dir)).join("todos")` — Task 8 completes this wiring. For now, pass `None` so the crate compiles, and add a guard test in `setup.rs`'s existing `mod tests`:

```rust
#[test]
fn guard_accepts_a_config_file_override() {
    // Not asserting on the path, only that a config-only source is accepted.
    let result = resolve_settings_path(false, Some("/from/config"));
    assert!(result.is_ok());
}
```

```bash
cargo test --bin ferrisbar && just lint
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/config_dir.rs src/setup.rs src/main.rs
git commit -m "feat: layer env over config file over defaults for the Claude dir"
```

---

### Task 8: Wire it all into `src/main.rs`

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: everything from Tasks 2–7
- Produces: the running behavior

- [ ] **Step 1: Rewrite `main`**

Replace `src/main.rs` lines 13–81 with:

```rust
fn resolve_todos_dir(cfg: &config::Config) -> PathBuf {
    config_dir::claude_config_dir(Some(&cfg.claude.config_dir)).join("todos")
}

fn main() {
    // Config and logging come up before anything else can fail, so the
    // failures below are reportable. `cfg` is mutable only so Task 10 can
    // layer the FERRISBAR_* overrides on before the logger reads it.
    let (mut cfg, warnings) = config::load(paths::config_file().as_deref());
    let _ = &mut cfg; // Task 10 replaces this line with the env overrides.
    let logger = log::Logger::new(&cfg, paths::data_dir().as_deref());
    for warning in &warnings {
        match warning {
            config::ParseWarning::Syntax(msg) => {
                logger.log(&log::warn("config_parse_failed", msg.clone()));
            }
            config::ParseWarning::Create(msg) => {
                logger.log(&log::warn("config_create_failed", msg.clone()));
            }
        }
    }

    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [cmd] if cmd == "setup" => {
            if let Err(e) = setup::run(false, &cfg) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        [cmd, flag] if cmd == "setup" && flag == "--project" => {
            if let Err(e) = setup::run(true, &cfg) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        _ => {
            let program = env::args().next().unwrap_or_default();
            let program_name = Path::new(&program).file_name().map_or_else(
                || "ferrisbar".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            eprintln!("Usage: {program_name} [setup [--project]]");
            std::process::exit(1);
        }
    }

    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        logger.log(&log::warn("stdin_read_failed", e.to_string()));
        return;
    }

    let payload = match serde_json::from_str::<Payload>(&input) {
        Ok(payload) => payload,
        Err(e) => {
            // Empty stdin is the documented no-op, not a fault.
            if !input.trim().is_empty() {
                logger.log(&log::warn("stdin_parse_failed", e.to_string()));
            }
            return;
        }
    };

    let process_cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cwd = payload.cwd(&process_cwd);
    let model = payload.model_name();
    let session_id = payload.session_id();

    let acw: f64 = env::var("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &f64| *v > 0.0)
        .unwrap_or(cfg.claude.auto_compact_window);
    let ctx = context_bar::render(
        payload.remaining_percentage(),
        payload.total_tokens(),
        acw,
    );

    let todos_dir = resolve_todos_dir(&cfg);
    // `active_task` returns None both for "no task in progress" and for
    // "the directory isn't there", which are very different problems. The
    // existence check separates them without changing todo.rs's signature.
    //
    // Gated on the parent existing: a Claude config dir that has no todos/
    // is a genuine anomaly worth a warning, whereas no Claude dir at all
    // means the resolved path is simply wrong, and warning on every render
    // for a user who has never run Claude Code there would be pure churn.
    let claude_dir_present = todos_dir.parent().is_some_and(Path::is_dir);
    if !session_id.is_empty() && claude_dir_present && !todos_dir.is_dir() {
        let mut event = log::warn(
            "todo_file_unreadable",
            format!("todos directory not found: {}", todos_dir.display()),
        );
        event.session_id = Some(session_id.clone());
        logger.log(&event);
    }
    let task = todo::active_task(&session_id, &todos_dir);

    let dirname = Path::new(&cwd)
        .file_name()
        .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());

    let output = layout::compose_statusline(&model, &ctx, task.as_deref(), &dirname);

    let mut render = log::debug(
        "render",
        format!("model={model} dir={dirname} acw={acw}"),
    );
    render.session_id = Some(session_id);
    logger.log(&render);

    print!("{output}");
}
```

Confirm the module list at the top of `src/main.rs` reads exactly:

```rust
mod config;
mod config_dir;
mod context_bar;
mod layout;
mod log;
mod paths;
mod payload;
mod setup;
mod todo;
```

Note that `mod log;` shadows nothing — this crate has no `log` crate dependency — but if a future dependency introduces one, the local module still wins inside `main.rs`.

Note the `.filter(|v| *v > 0.0)` on the env var: `main.rs`'s original `unwrap_or(0.0)` treated `0` as "unset", and the config layer needs the same treatment so an exported `CLAUDE_CODE_AUTO_COMPACT_WINDOW=0` falls through to the file rather than pinning the buffer to zero.

- [ ] **Step 2: Build and run the existing suite**

```bash
cargo build && just test
```

Expected: PASS. Any failure in `tests/cli.rs` at this point is Task 9's work — note which tests fail and continue.

- [ ] **Step 3: Verify by hand that stdout is unchanged**

```bash
echo '{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp/demo"}}' \
  | cargo run --quiet | cat -A | head -3
```

Expected: the same escape sequences as before this branch. To compare against `main` directly:

```bash
PAYLOAD='{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp/demo"}}'
echo "$PAYLOAD" | cargo run --quiet | cat -A > /tmp/after.txt
git stash push --include-untracked
echo "$PAYLOAD" | cargo run --quiet | cat -A > /tmp/before.txt
git stash pop
diff /tmp/before.txt /tmp/after.txt && echo "IDENTICAL"
```

Expected: `IDENTICAL`.

- [ ] **Step 4: Verify empty stdin still prints nothing and exits 0**

```bash
printf '' | cargo run --quiet; echo "EXIT=$?"
```

Expected: no output, `EXIT=0`.

- [ ] **Step 5: Lint**

```bash
just lint
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat: load config and initialize logging before rendering"
```

---

### Task 9: Retrofit `tests/cli.rs` and add end-to-end coverage

This is a retrofit, not an addition. Now that `main` creates a config file, every *existing* test that spawns the binary writes into the developer's real home directory and rotates their real log. All of them must route through the tempdir helper.

**Files:**
- Modify: `tests/cli.rs` (all existing tests, plus new ones)

**Interfaces:**
- Consumes: the wired binary from Task 8
- Produces: `fn isolated() -> (Command, TempDir)`

- [ ] **Step 1: Add the isolation helper**

Add near the top of `tests/cli.rs`:

```rust
/// Every `Command` in this file must come from here. Without the env
/// overrides, running the suite writes into the developer's real
/// ~/.config and ~/.local/share — and rotates and gzips their real log.
///
/// The returned TempDir must stay alive for the duration of the test; it
/// deletes the directory tree on drop.
fn isolated() -> (std::process::Command, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ferrisbar"));
    cmd.env("HOME", home.path())
        .env("APPDATA", home.path())
        .env("LOCALAPPDATA", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_CODE_AUTO_COMPACT_WINDOW");
    (cmd, home)
}
```

- [ ] **Step 2: Route every existing test through it**

Read `tests/cli.rs` in full and replace each direct `Command::new(env!("CARGO_BIN_EXE_ferrisbar"))` with `let (mut cmd, _home) = isolated();`. Bind the `TempDir` to a named variable, never `_` — `_` drops it immediately and deletes the directory before the child runs. Tests that intentionally set `CLAUDE_CONFIG_DIR` or `HOME` keep those calls, since they come after the helper's defaults and override them.

- [ ] **Step 3: Run the existing suite**

```bash
cargo test --test cli
```

Expected: PASS. Confirm no `ferrisbar` directory appeared in your real config location:

```bash
ls ~/.config/ferrisbar ~/.local/share/ferrisbar 2>&1 | head -2
```

Expected: "No such file or directory" for both. If either exists and you did not create it deliberately, a test is still leaking — find it before continuing.

- [ ] **Step 4: Write the new end-to-end tests**

Append to `tests/cli.rs`:

```rust
fn run(cmd: &mut std::process::Command, stdin: &str) -> (String, bool) {
    use std::io::Write as _;
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.success())
}

const PAYLOAD: &str = r#"{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp/demo"},"session_id":"s1"}"#;

#[test]
fn a_normal_render_creates_the_config_file() {
    let (mut cmd, home) = isolated();
    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Claude"));
    assert!(
        home.path().join(".config").join("ferrisbar").join("config.toml").exists(),
        "the config file is created on first run"
    );
}

#[test]
fn a_malformed_config_still_renders_and_is_left_untouched() {
    let (mut cmd, home) = isolated();
    let dir = home.path().join(".config").join("ferrisbar");
    std::fs::create_dir_all(&dir).unwrap();
    let original = "not = = toml";
    std::fs::write(dir.join("config.toml"), original).unwrap();

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Claude"), "a broken config must not blank the statusline");
    assert_eq!(
        std::fs::read_to_string(dir.join("config.toml")).unwrap(),
        original
    );

    let log = home.path().join(".local").join("share").join("ferrisbar")
        .join("logs").join("ferrisbar.jsonl");
    assert!(
        std::fs::read_to_string(&log).unwrap().contains("config_parse_failed"),
        "the failure is diagnosable"
    );
}

#[test]
fn stdout_is_identical_with_and_without_a_config_file() {
    let (mut first, home_a) = isolated();
    let (baseline, _) = run(&mut first, PAYLOAD);

    // Second run: the config file now exists from the first run.
    let mut second = std::process::Command::new(env!("CARGO_BIN_EXE_ferrisbar"));
    second
        .env("HOME", home_a.path())
        .env("APPDATA", home_a.path())
        .env("LOCALAPPDATA", home_a.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_CODE_AUTO_COMPACT_WINDOW");
    let (with_config, _) = run(&mut second, PAYLOAD);

    assert_eq!(baseline, with_config, "config presence must not alter output");
}

#[test]
fn logging_disabled_creates_no_log_directory() {
    let (mut cmd, home) = isolated();
    let dir = home.path().join(".config").join("ferrisbar");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[log]\nenabled = false\n").unwrap();

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Claude"));
    assert!(!home.path().join(".local").join("share").join("ferrisbar").join("logs").exists());
}

#[test]
fn an_unresolvable_home_still_renders() {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ferrisbar"));
    cmd.env_remove("HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CLAUDE_CONFIG_DIR");

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Claude"));
}

#[test]
fn no_stray_directory_is_created_when_home_is_unset() {
    let scratch = tempfile::tempdir().unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ferrisbar"));
    cmd.current_dir(scratch.path())
        .env_remove("HOME")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME");

    let (_, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(
        !scratch.path().join("ferrisbar").exists(),
        "an empty HOME must never produce a relative path that lands in the user's repo"
    );
}

#[test]
fn env_var_beats_the_config_file_for_the_claude_dir() {
    let (mut cmd, home) = isolated();
    let dir = home.path().join(".config").join("ferrisbar");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        format!("[claude]\nconfig_dir = \"{}\"\n", home.path().join("from-file").display()),
    )
    .unwrap();

    // The env var points at a todos dir containing an active task; the
    // file points somewhere empty. The task appearing proves env won.
    let from_env = home.path().join("from-env");
    std::fs::create_dir_all(from_env.join("todos")).unwrap();
    std::fs::write(
        from_env.join("todos").join("s1-agent-s1.json"),
        r#"[{"content":"Ship it","status":"in_progress","activeForm":"Shipping it"}]"#,
    )
    .unwrap();
    cmd.env("CLAUDE_CONFIG_DIR", &from_env);

    let (stdout, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    assert!(stdout.contains("Shipping it"), "CLAUDE_CONFIG_DIR must still win");
}
```

- [ ] **Step 5: Run the new tests**

```bash
cargo test --test cli
```

Expected: PASS. The todo fixture filename is not arbitrary: `src/todo.rs:24` requires it to start with the session id, contain `-agent-`, and end with `.json`, so `s1-agent-s1.json` against `"session_id":"s1"` satisfies all three. `src/todo.rs:45-49` prefers `activeForm` over `content`, which is why the assertion looks for "Shipping it" rather than "Ship it".

- [ ] **Step 6: Full suite and lint**

```bash
just test && just lint
```

Expected: both PASS.

- [ ] **Step 7: Commit**

```bash
git add tests/cli.rs
git commit -m "test: isolate the CLI suite from the real home directory"
```

---

### Task 10: Environment overrides and documentation

**Files:**
- Modify: `README.md:220-232`
- Modify: `CLAUDE.md` (the dependency invariant)

**Interfaces:**
- Consumes: the shipped behavior
- Produces: nothing code-facing

- [ ] **Step 1: Rewrite the README Configuration section**

`README.md:222` currently reads "There is no config file." Replace the section body (keeping the `## 🧰 Configuration` heading and the anchors that `README.md:45` and `README.md:75` link to) with:

````markdown
ferrisbar reads an optional TOML config file, creating it with the defaults
below the first time it runs. Environment variables override anything set
in the file.

### Where things live

| | Linux | macOS | Windows |
|---|---|---|---|
| Config | `$XDG_CONFIG_HOME/ferrisbar/config.toml`, else `~/.config/ferrisbar/config.toml` | `~/Library/Application Support/ferrisbar/config.toml` | `%APPDATA%\ferrisbar\config.toml` |
| Log | `$XDG_DATA_HOME/ferrisbar/logs/`, else `~/.local/share/ferrisbar/logs/` | `~/Library/Application Support/ferrisbar/logs/` | `%LOCALAPPDATA%\ferrisbar\logs\` |

### The config file

```toml
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

A relative `log.path` resolves against the data directory, not the
directory you happen to be in.

### The log

One JSON object per line, at `<data dir>/logs/ferrisbar.jsonl`:

```json
{"ts":1753467296123,"level":"warn","event":"stdin_parse_failed","session_id":"abc123","msg":"expected value at line 1 column 1"}
```

`ts` is epoch milliseconds. At the default `warn` level ferrisbar logs only
when something degrades — a statusline that renders correctly writes
nothing — so **any content in this file is a signal**. Set `level = "debug"`
to add one line per render while troubleshooting.

The file rotates at `max_size_bytes` into `ferrisbar.jsonl.1.gz` through
`.7.gz`, oldest dropped.

### Environment variables

Each overrides its config-file counterpart.

| Variable | Overrides | Notes |
|---|---|---|
| `CLAUDE_CONFIG_DIR` | `claude.config_dir` | Where Claude Code keeps its per-session task files, which is how the active task is found. |
| `CLAUDE_CODE_AUTO_COMPACT_WINDOW` | `claude.auto_compact_window` | Token count at which [auto-compaction][compact] fires. Set it to calibrate the gauge exactly instead of trusting the default buffer. |
| `FERRISBAR_LOG_PATH` | `log.path` | Log to somewhere else for one session. |
| `FERRISBAR_LOG_LEVEL` | `log.level` | Turn logging up without editing the file. |
````

Verify the `[compact]` link reference still resolves — it is defined further down the original README and must not be orphaned by the rewrite.

- [ ] **Step 2: Check the anchors still work**

```bash
grep -n "#-configuration" README.md
```

Expected: the links at `README.md:45` and `README.md:75` still point at a heading that exists.

- [ ] **Step 3: Update the CLAUDE.md invariant**

Replace the dependency bullet in `CLAUDE.md`'s Invariants section:

```markdown
- **Four runtime dependencies is deliberate.** `serde` and `serde_json` for
  the payload, `toml` for the config file, `flate2` for log rotation. A
  fifth needs a justification and a `cargo vet` entry in `supply-chain/`.
  `toml` is version-pinned below 1.2 because it sits one patch under our
  MSRV floor.
```

- [ ] **Step 4: Implement the two `FERRISBAR_*` overrides**

The README now documents them, so they must exist. In `src/main.rs`, replace the `let _ = &mut cfg;` placeholder line from Task 8 with:

```rust
    if let Some(path) = env::var("FERRISBAR_LOG_PATH").ok().filter(|v| !v.is_empty()) {
        cfg.log.path = path;
    }
    if let Some(level) = env::var("FERRISBAR_LOG_LEVEL").ok().filter(|v| !v.is_empty()) {
        cfg.log.level = level;
    }
    let cfg = cfg;
```

The trailing rebind drops mutability so nothing downstream can alter the config after the logger has read it.

Add an end-to-end test to `tests/cli.rs`:

```rust
#[test]
fn ferrisbar_log_level_env_var_beats_the_config_file() {
    let (mut cmd, home) = isolated();
    let dir = home.path().join(".config").join("ferrisbar");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[log]\nlevel = \"off\"\n").unwrap();
    cmd.env("FERRISBAR_LOG_LEVEL", "debug");

    let (_, ok) = run(&mut cmd, PAYLOAD);

    assert!(ok);
    let log = home.path().join(".local").join("share").join("ferrisbar")
        .join("logs").join("ferrisbar.jsonl");
    assert!(
        std::fs::read_to_string(&log).unwrap().contains("\"event\":\"render\""),
        "FERRISBAR_LOG_LEVEL must override level = \"off\" in the file"
    );
}
```

- [ ] **Step 5: Run the full check suite**

```bash
just ci
```

Expected: EXIT 0. `cargo geiger` may print a warning; that recipe is prefixed with `-` in the justfile and is informational only.

- [ ] **Step 6: Manual smoke test**

```bash
echo '{"model":{"display_name":"Claude"},"workspace":{"current_dir":"/tmp/demo"},"context_window":{"remaining_percentage":40,"total_tokens":200000}}' \
  | cargo run --quiet; echo
ls -la ~/.config/ferrisbar/ ~/.local/share/ferrisbar/logs/ 2>&1
cat ~/.local/share/ferrisbar/config.toml 2>/dev/null || cat ~/.config/ferrisbar/config.toml
```

Expected: a rendered statusline; `config.toml` present with the template contents; `logs/` present and the log file either absent or empty, because a healthy render at `warn` level writes nothing.

- [ ] **Step 7: Commit**

```bash
git add README.md CLAUDE.md src/main.rs tests/cli.rs
git commit -m "docs: document the config file, log format, and precedence"
```

---

## Done criteria

- [ ] `just ci` exits 0
- [ ] `config.toml` is created on first run on Linux, macOS, and Windows (CI's three-OS matrix proves it)
- [ ] A healthy render at default level writes no log content
- [ ] A malformed config renders normally and logs exactly one warning
- [ ] Unset `HOME` renders normally and creates nothing anywhere
- [ ] Statusline stdout is byte-identical to `main`'s output for every payload
- [ ] `README.md` no longer claims there is no config file
- [ ] No `[display]` keys appear in the generated template

## Not in this plan

Phase 2 — the `[display]` block (`bar_width`, `threshold_yellow`,
`threshold_orange`, `threshold_critical`, `show_task`) threaded through
`context_bar.rs` and `layout.rs` — gets its own plan. Its central risk is
recorded in the spec: making the bar width configurable turns
`context_bar.rs:22`'s `10 - filled` into `width - filled`, a `usize`
subtraction that underflows and panics unless `filled` is clamped to
`width` first.
