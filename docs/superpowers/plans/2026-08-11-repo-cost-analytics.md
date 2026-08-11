# Per-Repo Cost Analytics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Track Claude Code cost, by day and by model, per git repository, in an opt-in embedded datastore, and add a `ferrisbar report` CLI subcommand that exports it as JSON or CSV.

**Architecture:** A new optional `analytics` Cargo feature adds `redb` (pure-Rust embedded storage) and hooks into the *existing* background cost-chip refresh (`cost_cache::spawn_refresh` → `cost::refresh_daily_cache`) to resolve each transcript record's repo identity from its own `cwd` field and upsert per-`(date, repo, model)` rows, rather than adding a second background job. `ferrisbar report` reads the same store. Everything is gated so a default (`analytics` feature off) build is byte-for-byte unaffected.

**Tech Stack:** Rust 1.85.1, `redb` (new, feature-gated), `serde_json` (existing, reused for row serialization), no new CLI-arg-parsing or CSV crate — both hand-rolled to keep the dependency count minimal.

## Global Constraints

- MSRV is 1.85.1 — do not use stdlib APIs stabilized after it, and do not raise `rust-version` in `Cargo.toml`.
- Never panic on malformed/partial input, at any layer this plan touches (transcript JSON, `.git/config`, the analytics store itself, CLI flags). Every failure degrades to a smaller result or empty output, never a crash or nonzero exit for "no data yet."
- Clippy `pedantic` and `nursery` are on with `-D warnings`. An `#[allow(...)]` needs a comment explaining why, matching the house style at `src/context_bar.rs:12`.
- `redb` is the fifth runtime dependency (after `serde`, `serde_json`, `toml`, `flate2`) — it must be `optional = true` behind the `analytics` feature, so a default build's dependency count is unchanged.
- Never hand-edit `version` in `Cargo.toml` or `CHANGELOG.md` — release automation owns both.
- Unit tests live in `mod tests` beside the code they test; CLI end-to-end tests live in `tests/cli.rs`.
- Run `just fmt lint test` (and, for feature-gated code, the `*-analytics` variants added in Task 11) before every commit in this plan.

---

### Task 1: Add the `analytics` Cargo feature and the `redb` dependency

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: an optional dependency `redb`, reachable only when the crate is built with `--features analytics`. No Rust code changes yet — this task is pure build-graph plumbing.

- [ ] **Step 1: Add the dependency and feature**

In `Cargo.toml`, add to `[dependencies]` (after `flate2`):

```toml
# Pure-Rust embedded storage for the optional per-repo cost analytics
# store — no C toolchain, unlike rusqlite/Diesel's SQLite backends. See
# docs/superpowers/specs/2026-08-11-repo-cost-analytics-design.md.
redb = { version = "2", optional = true }
```

And add a new table:

```toml
[features]
analytics = ["dep:redb"]
```

- [ ] **Step 2: Verify the default build is unaffected**

Run: `cargo build`
Expected: succeeds, and `redb` does not appear in `cargo tree` output (run `cargo tree | grep -i redb` to confirm — expect no match).

- [ ] **Step 3: Verify the feature builds**

Run: `cargo build --features analytics`
Expected: succeeds; `cargo tree --features analytics | grep -i redb` now shows `redb`.

- [ ] **Step 4: Verify MSRV compatibility of the new dependency**

Run: `cargo msrv verify --features analytics`
Expected: passes. If it fails because `redb`'s own MSRV exceeds 1.85.1, pin an older `redb` version (e.g. `redb = { version = "=2.<lower>", optional = true }`) and re-run until it passes.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat: add optional analytics feature and redb dependency"
```

---

### Task 2: Add the `[analytics]` config section

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Produces: `config::AnalyticsConfig { enabled: bool }`, `Config::analytics: AnalyticsConfig`, parsed by `from_toml_str`, defaulting to `enabled: false`, unconditionally compiled (no `cfg(feature = ...)` — the config struct exists in every build; only its *effect* is feature-gated elsewhere).

- [ ] **Step 1: Write the failing tests**

Add to `src/config.rs`'s `mod tests`:

```rust
#[test]
fn analytics_defaults_match_the_documented_values() {
    let c = AnalyticsConfig::default();
    assert!(!c.enabled);
}

#[test]
fn template_includes_analytics_block() {
    assert!(TEMPLATE.contains("[analytics]"));
    assert!(TEMPLATE.contains("enabled"));
}

#[test]
fn analytics_values_are_read_from_toml() {
    let (c, _) = from_toml_str("[analytics]\nenabled = true\n");
    assert!(c.analytics.enabled);
}

#[test]
fn analytics_partial_block_fills_in_defaults() {
    let (c, _) = from_toml_str("[analytics]\n");
    assert!(!c.analytics.enabled);
}

#[test]
fn template_round_trips_including_analytics() {
    let (c, warnings) = from_toml_str(TEMPLATE);
    assert!(warnings.is_empty());
    assert_eq!(c, Config::default());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test analytics_ config::tests -- --exact`
Expected: FAIL — `AnalyticsConfig` and `Config::analytics` do not exist yet (compile error).

- [ ] **Step 3: Add `AnalyticsConfig` and wire it into `Config`**

Add near `CostConfig`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyticsConfig {
    pub enabled: bool,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}
```

Add a field to `Config`:

```rust
pub struct Config {
    pub log: LogConfig,
    pub claude: ClaudeConfig,
    pub display: DisplayConfig,
    pub cost: CostConfig,
    pub budget: BudgetConfig,
    pub analytics: AnalyticsConfig,
}
```

And to its manual `Default` impl:

```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            log: LogConfig::default(),
            claude: ClaudeConfig::default(),
            display: DisplayConfig::default(),
            cost: CostConfig::default(),
            budget: BudgetConfig::default(),
            analytics: AnalyticsConfig::default(),
        }
    }
}
```

In `from_toml_str`'s `Config { ... }` literal, add:

```rust
        analytics: parse_analytics(&table),
```

And define, next to `parse_budget`:

```rust
fn parse_analytics(table: &toml::Table) -> AnalyticsConfig {
    let analytics = section(table, "analytics");
    let defaults = AnalyticsConfig::default();
    AnalyticsConfig {
        enabled: get_bool(analytics, "enabled").unwrap_or(defaults.enabled),
    }
}
```

- [ ] **Step 4: Add the `[analytics]` block to `TEMPLATE`**

Append to the end of the `TEMPLATE` string constant (before the closing `"#;`):

```toml

[analytics]
enabled = false  # off by default. Requires ferrisbar to be built with the
                  # `analytics` Cargo feature to have any effect — see
                  # README.md's Analytics section.
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test config::tests`
Expected: PASS, including every pre-existing `config::tests` test (the new field must not break `defaults_match_the_documented_values` or any other test comparing a full `Config`).

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat: add [analytics] config section"
```

---

### Task 3: Repo identity resolution

**Files:**
- Modify: `src/git.rs` (visibility only)
- Create: `src/repo_identity.rs`
- Modify: `src/main.rs` (add `mod repo_identity;`)

**Interfaces:**
- Consumes: `git::find_git_dir(start: &Path) -> Option<PathBuf>` (existing, made `pub(crate)`).
- Produces: `repo_identity::RepoIdentity { key: String, display: String }`, `repo_identity::resolve(cwd: &str) -> RepoIdentity` — always returns *something* (never `None`), used by Task 5's `Sink` and Task 8's CLI default-repo resolution.

- [ ] **Step 1: Make `find_git_dir` crate-visible**

In `src/git.rs`, change:

```rust
fn find_git_dir(start: &Path) -> Option<PathBuf> {
```

to:

```rust
pub(crate) fn find_git_dir(start: &Path) -> Option<PathBuf> {
```

- [ ] **Step 2: Write the failing tests**

Create `src/repo_identity.rs`:

```rust
//! Resolves which git repository a working directory belongs to, for
//! analytics indexing: a normalized `origin` remote when one is
//! configured, otherwise the repository's own root folder name. See
//! docs/superpowers/specs/2026-08-11-repo-cost-analytics-design.md.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoIdentity {
    pub(crate) key: String,
    pub(crate) display: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_git_dir(repo_root: &Path, origin_url: Option<&str>) {
        let git_dir = repo_root.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let config = origin_url.map_or_else(String::new, |url| {
            format!("[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n")
        });
        std::fs::write(git_dir.join("config"), config).unwrap();
    }

    #[test]
    fn ssh_https_and_scp_forms_of_the_same_remote_resolve_to_the_same_key() {
        let cases = [
            "git@github.com:kerryhatcher/ferrisbar.git",
            "ssh://git@github.com/kerryhatcher/ferrisbar.git",
            "https://github.com/kerryhatcher/ferrisbar.git",
            "https://github.com/kerryhatcher/ferrisbar",
        ];
        let mut keys = Vec::new();
        for url in cases {
            let dir = tempfile::tempdir().unwrap();
            write_git_dir(dir.path(), Some(url));
            keys.push(resolve(dir.path().to_str().unwrap()).key);
        }
        for key in &keys[1..] {
            assert_eq!(key, &keys[0], "all four URL forms must normalize identically");
        }
        assert_eq!(keys[0], "remote:github.com/kerryhatcher/ferrisbar");
    }

    #[test]
    fn embedded_credentials_are_stripped() {
        let dir = tempfile::tempdir().unwrap();
        write_git_dir(dir.path(), Some("https://user:token@github.com/org/repo.git"));
        let identity = resolve(dir.path().to_str().unwrap());
        assert_eq!(identity.key, "remote:github.com/org/repo");
    }

    #[test]
    fn no_origin_remote_falls_back_to_the_repo_root_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path().join("my-local-project");
        std::fs::create_dir_all(&repo_root).unwrap();
        write_git_dir(&repo_root, None);
        let identity = resolve(repo_root.to_str().unwrap());
        assert_eq!(identity.key, "local:my-local-project");
        assert_eq!(identity.display, "my-local-project");
    }

    #[test]
    fn no_git_repo_at_all_falls_back_to_the_cwd_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let leaf = dir.path().join("scratch-dir");
        std::fs::create_dir_all(&leaf).unwrap();
        let identity = resolve(leaf.to_str().unwrap());
        assert_eq!(identity.key, "local:scratch-dir");
    }

    #[test]
    fn resolution_walks_up_from_a_nested_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        write_git_dir(dir.path(), Some("https://github.com/a/b.git"));
        let nested = dir.path().join("src").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        let identity = resolve(nested.to_str().unwrap());
        assert_eq!(identity.key, "remote:github.com/a/b");
    }

    #[test]
    fn nonexistent_cwd_still_resolves_without_panicking() {
        let identity = resolve("/does/not/exist/at/all");
        assert_eq!(identity.key, "local:all");
        let _: PathBuf = PathBuf::new(); // silence unused-import warning if any
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test repo_identity::`
Expected: FAIL — `resolve` and `normalize_remote_url` do not exist yet (compile error).

- [ ] **Step 3: Implement resolution**

Add above the `#[cfg(test)]` block in `src/repo_identity.rs`:

```rust
/// Strips scheme, embedded `user[:token]@`, and a trailing `.git`, unifying
/// `git@host:org/repo.git`, `ssh://git@host/org/repo.git`, and
/// `https://host/org/repo(.git)?` down to the same `host/org/repo` string.
/// `None` for an empty or unparseable URL.
fn normalize_remote_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let has_scheme = trimmed.contains("://");
    let without_scheme = trimmed.split("://").last().unwrap_or(trimmed);
    // The last '@' strips both a URL's `user[:token]@host` and the
    // scp-like shorthand's `user@host:path` down to whatever follows it.
    let host_and_path = without_scheme
        .rfind('@')
        .map_or(without_scheme, |i| &without_scheme[i + 1..]);
    let normalized = if has_scheme {
        host_and_path.to_string()
    } else {
        // scp-like shorthand uses `:` where a URL uses `/` between host
        // and path — swap only the first one so both forms end up alike.
        host_and_path.replacen(':', "/", 1)
    };
    let normalized = normalized.strip_suffix(".git").unwrap_or(&normalized);
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_lowercase())
    }
}

/// Reads `.git/config`'s `[remote "origin"]` section for its `url`. `None`
/// for a missing/unreadable config, or one with no origin remote — both
/// are normal, not faults.
fn read_origin_url(git_dir: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(git_dir.join("config")).ok()?;
    let mut in_origin_section = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_origin_section = trimmed.eq_ignore_ascii_case("[remote \"origin\"]");
            continue;
        }
        if !in_origin_section {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("url") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// The last path component of `.git`'s parent (the repository root),
/// falling back to `"unknown"` for the practically-impossible case of a
/// root path with no final component at all.
fn root_folder_name(git_dir: &Path) -> String {
    git_dir
        .parent()
        .and_then(Path::file_name)
        .map_or_else(|| "unknown".to_string(), |n| n.to_string_lossy().into_owned())
}

/// `cwd`'s own last path component, falling back to `"unknown"` the same
/// way `root_folder_name` does. Used when `cwd` is not inside any git
/// repository at all — there is no repo root to name, so `cwd` itself
/// stands in for it.
fn cwd_folder_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map_or_else(|| "unknown".to_string(), |n| n.to_string_lossy().into_owned())
}

/// Resolves `cwd`'s repository identity. Always returns a usable
/// `RepoIdentity` — a missing `.git`, an unreadable config, or an
/// unparseable remote URL all degrade to a `local:` identity rather than
/// failing.
pub(crate) fn resolve(cwd: &str) -> RepoIdentity {
    let Some(git_dir) = crate::git::find_git_dir(Path::new(cwd)) else {
        let name = cwd_folder_name(cwd);
        return RepoIdentity {
            key: format!("local:{name}"),
            display: name,
        };
    };
    if let Some(origin) = read_origin_url(&git_dir).and_then(|raw| normalize_remote_url(&raw)) {
        return RepoIdentity {
            key: format!("remote:{origin}"),
            display: origin,
        };
    }
    let name = root_folder_name(&git_dir);
    RepoIdentity {
        key: format!("local:{name}"),
        display: name,
    }
}
```

- [ ] **Step 4: Declare the module**

In `src/main.rs`, add to the `mod` list (alphabetically, after `payload`):

```rust
mod repo_identity;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test repo_identity::`
Expected: PASS. Also run `cargo build` (no features) to confirm this module, having zero `redb`/analytics dependency, compiles unconditionally.

- [ ] **Step 6: Commit**

```bash
git add src/git.rs src/repo_identity.rs src/main.rs
git commit -m "feat: resolve repo identity from a working directory's git remote"
```

---

### Task 4: Capture `cwd` on parsed transcript records

**Files:**
- Modify: `src/cost.rs`

**Interfaces:**
- Produces: `ParsedRecord.cwd: Option<String>` (new field), plus `pub(crate)` visibility on `ParsedRecord`, `Usage`, and every field Task 5's `Sink` will read (`usage`, `model`, `date`, `cwd`, and `Usage`'s four token fields) — the type Task 5 consumes.

- [ ] **Step 1: Write the failing test**

Add to `src/cost.rs`'s `mod tests`, near `parse_line`'s other coverage (search the file for existing `parse_line` tests to place this alongside them):

```rust
#[test]
fn parse_line_captures_the_top_level_cwd_field() {
    let line = r#"{"cwd":"/Users/dev/myrepo","timestamp":"2026-08-10T10:00:00Z","requestId":"req_1","message":{"model":"claude-sonnet-5","id":"msg_1","usage":{"input_tokens":100}}}"#;
    let rec = parse_line(line).unwrap();
    assert_eq!(rec.cwd, Some("/Users/dev/myrepo".to_string()));
}

#[test]
fn parse_line_missing_cwd_is_none() {
    let line = &usage_line("2026-08-10T10:00:00Z", "claude-sonnet-5", "req_1", "msg_1", 100);
    let rec = parse_line(line).unwrap();
    assert_eq!(rec.cwd, None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test parse_line_captures_the_top_level_cwd_field parse_line_missing_cwd_is_none`
Expected: FAIL — `ParsedRecord` has no `cwd` field (compile error).

- [ ] **Step 3: Add the field and thread it through**

In `RecordRaw`, add a field:

```rust
#[derive(Deserialize)]
struct RecordRaw {
    message: Option<MessageRaw>,
    timestamp: Option<String>,
    #[serde(rename = "requestId", alias = "request_id")]
    request_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}
```

Change `ParsedRecord` to be `pub(crate)` with `pub(crate)` fields, plus the new one:

```rust
pub(crate) struct ParsedRecord {
    pub(crate) usage: Usage,
    pub(crate) model: String,
    pub(crate) date: String,
    timestamp_unix: Option<i64>,
    dedup_key: Option<String>,
    pub(crate) cwd: Option<String>,
}
```

Make `Usage` and its fields `pub(crate)` too:

```rust
#[allow(clippy::struct_field_names)]
#[derive(Default, Clone, Copy)]
pub(crate) struct Usage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
}
```

In `parse_line`'s final `Some(ParsedRecord { ... })`, add the new field:

```rust
    Some(ParsedRecord {
        usage,
        model: message.model.unwrap_or_default(),
        timestamp_unix: parse_iso8601_utc(&timestamp),
        date,
        dedup_key,
        cwd: record.cwd,
    })
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS — including every pre-existing `cost::tests` test, since this is a purely additive change (no existing field removed or renamed).

- [ ] **Step 5: Commit**

```bash
git add src/cost.rs
git commit -m "feat: capture cwd on parsed transcript records"
```

---

### Task 5: Analytics store (`Sink`, `Row`, key encoding)

**Files:**
- Create: `src/analytics.rs`
- Create: `src/analytics/store.rs`
- Modify: `src/main.rs` (add `mod analytics;`)

**Interfaces:**
- Consumes: `crate::cost::ParsedRecord` (Task 4), `crate::repo_identity::resolve` (Task 3).
- Produces: `analytics::Sink` with `Sink::new(enabled: bool, today: String, yesterday: String) -> Sink`, `Sink::record(&mut self, rec: &ParsedRecord, cost: f64)`, `Sink::flush(self, data_dir: &Path)`. This exact signature exists in **both** the `analytics` feature build and the default build (as a no-op stub in the latter) — Task 6 calls it unconditionally.
- Also produces (feature-gated only, consumed by Task 7): `store::Row`, `store::db_path(data_dir: &Path) -> PathBuf`, `store::decode_key(raw: &str) -> Option<(String, String, String)>`, `store::TABLE: TableDefinition<&str, &[u8]>`.

- [ ] **Step 1: Write the module shell with the non-feature stub**

Create `src/analytics.rs`:

```rust
//! Persistent per-repo, per-day, per-model cost history, feeding
//! `ferrisbar report`. Piggybacks on `cost::refresh_daily_cache`'s existing
//! transcript walk rather than adding a second one. See
//! docs/superpowers/specs/2026-08-11-repo-cost-analytics-design.md.
//!
//! `Sink` exists in every build, feature or not: `cost.rs` calls it
//! unconditionally so its hot loop never needs an `#[cfg]` of its own. When
//! the `analytics` feature is off, `Sink` below is a zero-cost no-op; the
//! real implementation lives in `store.rs`, compiled only with the feature.

#[cfg(feature = "analytics")]
mod report;
#[cfg(feature = "analytics")]
mod store;

#[cfg(feature = "analytics")]
pub(crate) use report::{parse_args as parse_report_args, render, Options as ReportOptions};
#[cfg(feature = "analytics")]
pub(crate) use store::Sink;

#[cfg(not(feature = "analytics"))]
pub(crate) struct Sink;

#[cfg(not(feature = "analytics"))]
impl Sink {
    pub(crate) fn new(_enabled: bool, _today: String, _yesterday: String) -> Self {
        Self
    }

    pub(crate) fn record(&mut self, _rec: &crate::cost::ParsedRecord, _cost: f64) {}

    pub(crate) fn flush(self, _data_dir: &std::path::Path) {}
}
```

(`report` will not exist until Task 7 — leave the `#[cfg(feature = "analytics")] mod report;` and its `use` line in place; they simply won't compile under `--features analytics` until Task 7 lands. This task only builds and tests under the default, non-analytics configuration until Step 6 below.)

- [ ] **Step 2: Declare the module**

In `src/main.rs`, add to the `mod` list (alphabetically, before `config`):

```rust
mod analytics;
```

- [ ] **Step 3: Verify the default (non-analytics) build compiles**

Run: `cargo build`
Expected: succeeds.

- [ ] **Step 4: Write the failing tests for the real store**

Create `src/analytics/store.rs`:

```rust
//! On-disk analytics store: a single redb table keyed by
//! `date\0repo_key\0model`, holding cost + token totals. Each key's value
//! is fully recomputed and overwritten on every refresh that touches it —
//! never accumulated incrementally across refreshes — matching how
//! `cost_cache` already treats the global daily total.

use crate::cost::ParsedRecord;
use crate::repo_identity::{self, RepoIdentity};
use redb::TableDefinition;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(crate) const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("cost_rows");

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct Row {
    pub(crate) repo_display: String,
    pub(crate) cost_usd: f64,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
}

impl Row {
    fn accumulate(&mut self, rec: &ParsedRecord, cost: f64) {
        self.cost_usd += cost;
        self.input_tokens += rec.usage.input_tokens;
        self.output_tokens += rec.usage.output_tokens;
        self.cache_creation_tokens += rec.usage.cache_creation_tokens;
        self.cache_read_tokens += rec.usage.cache_read_tokens;
    }
}

pub(crate) fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("analytics.redb")
}

fn encode_key(date: &str, repo_key: &str, model: &str) -> String {
    format!("{date}\0{repo_key}\0{model}")
}

/// Inverse of `encode_key`. `None` for a key that doesn't split into
/// exactly three `\0`-separated parts — defensive against a store written
/// by some future, differently-shaped version of this code.
pub(crate) fn decode_key(raw: &str) -> Option<(String, String, String)> {
    let mut parts = raw.split('\0');
    let date = parts.next()?.to_string();
    let repo_key = parts.next()?.to_string();
    let model = parts.next()?.to_string();
    if parts.next().is_some() {
        return None;
    }
    Some((date, repo_key, model))
}

pub(crate) struct Sink {
    enabled: bool,
    today: String,
    yesterday: String,
    rows: HashMap<String, Row>,
    repo_cache: HashMap<String, RepoIdentity>,
}

impl Sink {
    pub(crate) fn new(enabled: bool, today: String, yesterday: String) -> Self {
        Self {
            enabled,
            today,
            yesterday,
            rows: HashMap::new(),
            repo_cache: HashMap::new(),
        }
    }

    /// No-op unless analytics is enabled, `rec.date` is today or
    /// yesterday, and `rec.cwd` is present — usage with no `cwd` cannot be
    /// attributed to a repo, so it is skipped rather than guessed.
    pub(crate) fn record(&mut self, rec: &ParsedRecord, cost: f64) {
        if !self.enabled || (rec.date != self.today && rec.date != self.yesterday) {
            return;
        }
        let Some(cwd) = rec.cwd.as_deref() else {
            return;
        };
        let identity = self
            .repo_cache
            .entry(cwd.to_string())
            .or_insert_with(|| repo_identity::resolve(cwd))
            .clone();
        let key = encode_key(&rec.date, &identity.key, &rec.model);
        self.rows
            .entry(key)
            .or_insert_with(|| Row {
                repo_display: identity.display,
                cost_usd: 0.0,
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            })
            .accumulate(rec, cost);
    }

    /// Overwrites whatever was stored for each touched key — `self.rows`
    /// already holds each key's full recomputed total for this pass. A
    /// redb open/write/commit failure is swallowed: a failed analytics
    /// write must never break the cost-chip refresh it piggybacks on.
    pub(crate) fn flush(self, data_dir: &Path) {
        if !self.enabled || self.rows.is_empty() {
            return;
        }
        if std::fs::create_dir_all(data_dir).is_err() {
            return;
        }
        let Ok(db) = redb::Database::create(db_path(data_dir)) else {
            return;
        };
        let Ok(txn) = db.begin_write() else {
            return;
        };
        {
            let Ok(mut table) = txn.open_table(TABLE) else {
                return;
            };
            for (key, row) in &self.rows {
                let Ok(bytes) = serde_json::to_vec(row) else {
                    continue;
                };
                let _ = table.insert(key.as_str(), bytes.as_slice());
            }
        }
        let _ = txn.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_record(date: &str, model: &str, cwd: &str) -> ParsedRecord {
        // `cost.rs`'s `ParsedRecord` fields are `pub(crate)`, but its
        // `timestamp_unix`/`dedup_key` are private to that module — this
        // helper needs a constructor. Add one in `src/cost.rs` alongside
        // `ParsedRecord`'s definition (Task 4's struct):
        //
        //     #[cfg(any(test, feature = "analytics"))]
        //     impl ParsedRecord {
        //         pub(crate) fn for_test(date: &str, model: &str, cwd: &str, usage: Usage) -> Self {
        //             Self {
        //                 usage,
        //                 model: model.to_string(),
        //                 date: date.to_string(),
        //                 timestamp_unix: None,
        //                 dedup_key: None,
        //                 cwd: Some(cwd.to_string()),
        //             }
        //         }
        //     }
        crate::cost::ParsedRecord::for_test(
            date,
            model,
            cwd,
            crate::cost::Usage {
                input_tokens: 1000,
                output_tokens: 500,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
        )
    }

    #[test]
    fn encode_decode_key_round_trips() {
        let key = encode_key("2026-08-10", "remote:github.com/a/b", "claude-sonnet-5");
        assert_eq!(
            decode_key(&key),
            Some((
                "2026-08-10".to_string(),
                "remote:github.com/a/b".to_string(),
                "claude-sonnet-5".to_string()
            ))
        );
    }

    #[test]
    fn disabled_sink_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = Sink::new(false, "2026-08-10".to_string(), "2026-08-09".to_string());
        sink.record(&usage_record("2026-08-10", "claude-sonnet-5", "/tmp/repo"), 1.0);
        sink.flush(dir.path());
        assert!(!store::db_path(dir.path()).exists());
    }

    #[test]
    fn record_outside_today_or_yesterday_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = Sink::new(true, "2026-08-10".to_string(), "2026-08-09".to_string());
        sink.record(&usage_record("2026-08-01", "claude-sonnet-5", "/tmp/repo"), 1.0);
        sink.flush(dir.path());
        assert!(!db_path(dir.path()).exists(), "nothing to flush means no file at all");
    }

    #[test]
    fn record_with_no_cwd_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = Sink::new(true, "2026-08-10".to_string(), "2026-08-09".to_string());
        let mut rec = usage_record("2026-08-10", "claude-sonnet-5", "/tmp/repo");
        rec.cwd = None;
        sink.record(&rec, 1.0);
        sink.flush(dir.path());
        assert!(!db_path(dir.path()).exists());
    }

    #[test]
    fn enabled_sink_writes_a_readable_row() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap(); // no .git — resolves to a `local:` identity
        let cwd = repo.path().to_str().unwrap();
        let mut sink = Sink::new(true, "2026-08-10".to_string(), "2026-08-09".to_string());
        sink.record(&usage_record("2026-08-10", "claude-sonnet-5", cwd), 2.0);
        sink.record(&usage_record("2026-08-10", "claude-sonnet-5", cwd), 3.0);
        sink.flush(dir.path());

        let db = redb::Database::open(db_path(dir.path())).unwrap();
        let txn = db.begin_read().unwrap();
        let table = txn.open_table(TABLE).unwrap();
        let expected_key = encode_key(
            "2026-08-10",
            &repo_identity::resolve(cwd).key,
            "claude-sonnet-5",
        );
        let value = table.get(expected_key.as_str()).unwrap().unwrap();
        let row: Row = serde_json::from_slice(value.value()).unwrap();
        assert!((row.cost_usd - 5.0).abs() < 1e-9, "two records in one pass accumulate");
        assert_eq!(row.input_tokens, 2000);
    }
}
```

- [ ] **Step 5: Add the `ParsedRecord::for_test` constructor**

In `src/cost.rs`, add just below `ParsedRecord`'s struct definition (from Task 4):

```rust
#[cfg(any(test, feature = "analytics"))]
impl ParsedRecord {
    pub(crate) fn for_test(date: &str, model: &str, cwd: &str, usage: Usage) -> Self {
        Self {
            usage,
            model: model.to_string(),
            date: date.to_string(),
            timestamp_unix: None,
            dedup_key: None,
            cwd: Some(cwd.to_string()),
        }
    }
}
```

- [ ] **Step 6: Enable the `store` module for this task (temporarily bypassing `report`)**

`src/analytics.rs`'s `#[cfg(feature = "analytics")] mod report;` line references a file that doesn't exist until Task 7, so `cargo test --features analytics` will fail to compile right now with "file not found." Comment that one line and its `use` out for this task only:

```rust
// #[cfg(feature = "analytics")]
// mod report;

#[cfg(feature = "analytics")]
mod store;

// #[cfg(feature = "analytics")]
// pub(crate) use report::{parse_args as parse_report_args, render, Options as ReportOptions};
#[cfg(feature = "analytics")]
pub(crate) use store::Sink;
```

(Task 7's Step 1 uncomments these.)

- [ ] **Step 7: Run tests to verify they fail, then pass**

Run: `cargo test --features analytics analytics::store::`
Expected first: FAIL (`Sink`, `Row`, `decode_key`, `ParsedRecord::for_test` don't exist yet — compile errors) before Steps 4–5's code is in place. After adding it:
Run: `cargo test --features analytics analytics::store::`
Expected: PASS, all five tests in `store::tests`.

- [ ] **Step 8: Run the full default-build test suite too**

Run: `cargo test`
Expected: PASS — `ParsedRecord::for_test`'s `#[cfg(any(test, feature = "analytics"))]` means it's still compiled (and unused-but-harmless) in a plain `cargo test` run; nothing else in this task touches the non-analytics build.

- [ ] **Step 9: Commit**

```bash
git add src/analytics.rs src/analytics/store.rs src/cost.rs src/main.rs
git commit -m "feat: add the analytics store (Sink, Row, key encoding)"
```

---

### Task 6: Wire the `Sink` into the existing background refresh

**Files:**
- Modify: `src/cost.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `analytics::Sink` (Task 5, both builds).
- Produces: `cost::aggregate_windows(transcripts_root, now, today, analytics: &mut analytics::Sink) -> WindowTotals` (signature change — one new parameter), `cost::refresh_daily_cache(transcripts_root, data_dir, analytics_enabled: bool)` (signature change — one new parameter).

- [ ] **Step 1: Update `aggregate_windows`'s signature and call site**

In `src/cost.rs`, change the function signature:

```rust
fn aggregate_windows(
    transcripts_root: &Path,
    now: i64,
    today: &str,
    analytics: &mut crate::analytics::Sink,
) -> WindowTotals {
```

Inside the loop, immediately after the existing `if cost <= 0.0 { continue; }` line and before `if rec.date == today {`, add:

```rust
            analytics.record(&rec, cost);
```

- [ ] **Step 2: Update `refresh_daily_cache`'s signature and body**

```rust
pub fn refresh_daily_cache(transcripts_root: &Path, data_dir: &Path, analytics_enabled: bool) {
    let now = now_unix_secs();
    let today = today_utc_date(now);
    let yesterday = today_utc_date(now - 86_400);
    let mut analytics =
        crate::analytics::Sink::new(analytics_enabled, today.clone(), yesterday);
    let windows = aggregate_windows(transcripts_root, now, &today, &mut analytics);
    analytics.flush(data_dir);
    let payload = cost_cache::CachePayload {
        date: today,
        total_usd: windows.daily.total_usd,
        by_model: windows.daily.by_model,
        weekly_usd: windows.weekly_usd,
        monthly_usd: windows.monthly_usd,
        block5h_usd: windows.block5h_usd,
    };
    let _ = cost_cache::write_cache(data_dir, &payload);
    cost_cache::release_lock(data_dir);
}
```

- [ ] **Step 3: Fix the two existing test call sites**

`src/cost.rs`'s `mod tests` calls `aggregate_windows(dir.path(), now, "2026-08-10")` in two tests (`aggregate_windows_sums_matching_dates_across_files_and_dedups` and `aggregate_windows_keeps_reading_after_a_malformed_line`). Update both call sites to pass a disabled sink:

```rust
        let mut analytics = crate::analytics::Sink::new(false, String::new(), String::new());
        let windows = aggregate_windows(dir.path(), now, "2026-08-10", &mut analytics);
```

(A disabled `Sink` ignores every `record()` call regardless of date, so the empty `today`/`yesterday` strings passed here are inert.)

- [ ] **Step 4: Update the call site in `main.rs`**

```rust
        [cmd] if cmd == "--internal-refresh-daily-cost" => {
            if let Some(data_dir) = data_dir {
                cost::refresh_daily_cache(
                    &resolve_transcripts_dir(cfg),
                    data_dir,
                    cfg.analytics.enabled,
                );
            }
            true
        }
```

- [ ] **Step 5: Write the failing integration test**

Add to `src/cost.rs`'s `mod tests` (reusing the file's existing `write_transcript`/`usage_line` helpers):

```rust
#[test]
fn refresh_daily_cache_populates_the_analytics_store_when_enabled() {
    let transcripts = tempfile::tempdir().unwrap();
    let sub = transcripts.path().join("proj");
    std::fs::create_dir_all(&sub).unwrap();
    let repo = tempfile::tempdir().unwrap(); // no .git — resolves to a `local:` identity
    let today = today_utc_date(now_unix_secs());
    write_transcript(
        &sub,
        "a.jsonl",
        &[&format!(
            r#"{{"cwd":"{}","timestamp":"{today}T10:00:00Z","requestId":"req_1","message":{{"model":"claude-sonnet-5","id":"msg_1","usage":{{"input_tokens":1000000}}}}}}"#,
            repo.path().display()
        )],
    );
    let data_dir = tempfile::tempdir().unwrap();

    refresh_daily_cache(transcripts.path(), data_dir.path(), true);

    assert!(
        crate::analytics::store::db_path(data_dir.path()).exists(),
        "an enabled refresh with cost-bearing, cwd-tagged usage must write the analytics store"
    );
}

#[test]
fn refresh_daily_cache_skips_the_analytics_store_when_disabled() {
    let transcripts = tempfile::tempdir().unwrap();
    let sub = transcripts.path().join("proj");
    std::fs::create_dir_all(&sub).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let today = today_utc_date(now_unix_secs());
    write_transcript(
        &sub,
        "a.jsonl",
        &[&format!(
            r#"{{"cwd":"{}","timestamp":"{today}T10:00:00Z","requestId":"req_1","message":{{"model":"claude-sonnet-5","id":"msg_1","usage":{{"input_tokens":1000000}}}}}}"#,
            repo.path().display()
        )],
    );
    let data_dir = tempfile::tempdir().unwrap();

    refresh_daily_cache(transcripts.path(), data_dir.path(), false);

    assert!(!crate::analytics::store::db_path(data_dir.path()).exists());
}
```

`crate::analytics::store` is private (`mod store;` with no `pub`), so these two tests need `db_path` reachable from `cost.rs`. Change the module declaration in `src/analytics.rs` to re-export the module itself under the feature:

```rust
#[cfg(feature = "analytics")]
pub(crate) mod store;
```

(`report` (Task 7) stays a private `mod report;` — nothing outside `analytics.rs` needs it directly, only the re-exported functions.)

- [ ] **Step 6: Run tests to verify they fail, then pass**

Run: `cargo test --features analytics cost::`
Expected first: FAIL (`crate::analytics::store` not visible, or the two updated call sites not yet updated) before Steps 1–5 land; PASS once they do, including every pre-existing `cost::tests` test.

Run: `cargo test cost::` (no features)
Expected: PASS — Step 3's disabled-`Sink` call sites work identically in both builds.

- [ ] **Step 7: Commit**

```bash
git add src/cost.rs src/main.rs src/analytics.rs
git commit -m "feat: feed the analytics store from the existing daily-cost refresh"
```

---

### Task 7: Analytics report engine — filtering and JSON/CSV rendering

**Files:**
- Create: `src/analytics/report.rs`
- Modify: `src/analytics.rs` (uncomment the Task 5 placeholders)

**Interfaces:**
- Consumes: `store::{TABLE, Row, db_path, decode_key}` (Task 5).
- Produces: `report::Options { repo_key: Option<String>, from: Option<String>, to: Option<String>, all: bool, format: Format }`, `report::parse_args(args: &[String]) -> Result<Options, String>`, `report::render(data_dir: &Path, default_repo_key: &str, opts: &Options) -> String` — consumed by Task 8's CLI wiring as `analytics::{parse_report_args, render, ReportOptions}`.

- [ ] **Step 1: Uncomment the Task 5 placeholders**

In `src/analytics.rs`, restore:

```rust
#[cfg(feature = "analytics")]
mod report;

#[cfg(feature = "analytics")]
pub(crate) use report::{parse_args as parse_report_args, render, Options as ReportOptions};
```

- [ ] **Step 2: Write the failing tests**

Create `src/analytics/report.rs`:

```rust
//! `ferrisbar report` — reads what `store.rs` writes and renders it as
//! JSON or CSV, either for one repo (default: the one `cwd` resolves to)
//! or, with `--all`, one summary row per tracked repo.

use super::store::{db_path, decode_key, Row, TABLE};
use redb::ReadableTable;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

pub(crate) enum Format {
    Json,
    Csv,
}

pub(crate) struct Options {
    pub(crate) repo_key: Option<String>,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
    pub(crate) all: bool,
    pub(crate) format: Format,
}

struct ReportRow {
    date: String,
    repo_key: String,
    repo_display: String,
    model: String,
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_row(
        data_dir: &Path,
        date: &str,
        repo_key: &str,
        repo_display: &str,
        model: &str,
        cost_usd: f64,
    ) {
        std::fs::create_dir_all(data_dir).unwrap();
        let db = redb::Database::create(db_path(data_dir)).unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut table = txn.open_table(TABLE).unwrap();
            let key = format!("{date}\0{repo_key}\0{model}");
            let row = Row {
                repo_display: repo_display.to_string(),
                cost_usd,
                input_tokens: 1000,
                output_tokens: 500,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            };
            let bytes = serde_json::to_vec(&row).unwrap();
            table.insert(key.as_str(), bytes.as_slice()).unwrap();
        }
        txn.commit().unwrap();
    }

    fn default_options() -> Options {
        Options {
            repo_key: None,
            from: None,
            to: None,
            all: false,
            format: Format::Json,
        }
    }

    #[test]
    fn parse_args_reads_every_flag() {
        let args: Vec<String> = vec![
            "--repo", "remote:github.com/a/b", "--from", "2026-08-01", "--to", "2026-08-10",
            "--format", "csv",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let opts = parse_args(&args).unwrap();
        assert_eq!(opts.repo_key, Some("remote:github.com/a/b".to_string()));
        assert_eq!(opts.from, Some("2026-08-01".to_string()));
        assert_eq!(opts.to, Some("2026-08-10".to_string()));
        assert!(matches!(opts.format, Format::Csv));
        assert!(!opts.all);
    }

    #[test]
    fn parse_args_all_flag_needs_no_value() {
        let args: Vec<String> = vec!["--all".to_string()];
        let opts = parse_args(&args).unwrap();
        assert!(opts.all);
    }

    #[test]
    fn parse_args_rejects_an_unknown_flag() {
        let args: Vec<String> = vec!["--bogus".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn parse_args_rejects_an_unknown_format() {
        let args: Vec<String> = vec!["--format".to_string(), "pdf".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn missing_store_renders_an_empty_json_array() {
        let dir = tempfile::tempdir().unwrap();
        let out = render(dir.path(), "local:whatever", &default_options());
        assert_eq!(out.trim(), "[]");
    }

    #[test]
    fn default_scope_reports_only_the_matching_repo_sorted_by_date() {
        let dir = tempfile::tempdir().unwrap();
        write_row(dir.path(), "2026-08-11", "local:a", "a", "claude-sonnet-5", 2.0);
        write_row(dir.path(), "2026-08-10", "local:a", "a", "claude-sonnet-5", 1.0);
        write_row(dir.path(), "2026-08-10", "local:b", "b", "claude-sonnet-5", 9.0);

        let out = render(dir.path(), "local:a", &default_options());
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let rows = parsed.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["date"], "2026-08-10");
        assert_eq!(rows[1]["date"], "2026-08-11");
        assert!(rows.iter().all(|r| r["repo"] == "local:a"));
    }

    #[test]
    fn explicit_repo_flag_overrides_the_default() {
        let dir = tempfile::tempdir().unwrap();
        write_row(dir.path(), "2026-08-10", "local:b", "b", "claude-sonnet-5", 9.0);
        let opts = Options {
            repo_key: Some("local:b".to_string()),
            ..default_options()
        };
        let out = render(dir.path(), "local:a", &opts);
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 1);
    }

    #[test]
    fn from_and_to_bound_the_date_range() {
        let dir = tempfile::tempdir().unwrap();
        write_row(dir.path(), "2026-08-09", "local:a", "a", "claude-sonnet-5", 1.0);
        write_row(dir.path(), "2026-08-10", "local:a", "a", "claude-sonnet-5", 2.0);
        write_row(dir.path(), "2026-08-11", "local:a", "a", "claude-sonnet-5", 3.0);
        let opts = Options {
            from: Some("2026-08-10".to_string()),
            to: Some("2026-08-10".to_string()),
            ..default_options()
        };
        let out = render(dir.path(), "local:a", &opts);
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let rows = parsed.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["date"], "2026-08-10");
    }

    #[test]
    fn all_flag_summarizes_one_row_per_repo() {
        let dir = tempfile::tempdir().unwrap();
        write_row(dir.path(), "2026-08-10", "local:a", "a", "claude-sonnet-5", 1.0);
        write_row(dir.path(), "2026-08-11", "local:a", "a", "claude-opus-4-8", 2.0);
        write_row(dir.path(), "2026-08-10", "local:b", "b", "claude-sonnet-5", 5.0);
        let opts = Options {
            all: true,
            ..default_options()
        };
        let out = render(dir.path(), "local:a", &opts);
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let rows = parsed.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let a = rows.iter().find(|r| r["repo"] == "local:a").unwrap();
        assert!((a["cost_usd"].as_f64().unwrap() - 3.0).abs() < 1e-9, "a's two rows sum to 3.0");
    }

    #[test]
    fn csv_format_has_a_header_and_one_line_per_row() {
        let dir = tempfile::tempdir().unwrap();
        write_row(dir.path(), "2026-08-10", "local:a", "a", "claude-sonnet-5", 1.5);
        let opts = Options {
            format: Format::Csv,
            ..default_options()
        };
        let out = render(dir.path(), "local:a", &opts);
        let mut lines = out.lines();
        assert_eq!(
            lines.next().unwrap(),
            "date,repo,repo_display,model,cost_usd,input_tokens,output_tokens,cache_creation_tokens,cache_read_tokens"
        );
        assert_eq!(lines.next().unwrap(), "2026-08-10,local:a,a,claude-sonnet-5,1.500000,1000,500,0,0");
        assert!(lines.next().is_none());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --features analytics analytics::report::`
Expected: FAIL — `parse_args`, `render`, `Format`, `Options` compile but are unimplemented; the test module itself will fail to compile until Step 4 lands (no function bodies yet).

- [ ] **Step 4: Implement `parse_args`, `read_all`, `render`, and both renderers**

Add above the `#[cfg(test)]` block in `src/analytics/report.rs`:

```rust
/// Parses `ferrisbar report`'s own flags. `args` excludes the `report`
/// token itself. `Err` holds a human-readable message for an unrecognized
/// flag or one missing its value; the caller prints it to stderr.
pub(crate) fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options {
        repo_key: None,
        from: None,
        to: None,
        all: false,
        format: Format::Json,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                opts.repo_key = Some(next_value(args, &mut i, "--repo")?);
            }
            "--from" => {
                opts.from = Some(next_value(args, &mut i, "--from")?);
            }
            "--to" => {
                opts.to = Some(next_value(args, &mut i, "--to")?);
            }
            "--all" => {
                opts.all = true;
                i += 1;
            }
            "--format" => {
                let value = next_value(args, &mut i, "--format")?;
                opts.format = match value.as_str() {
                    "json" => Format::Json,
                    "csv" => Format::Csv,
                    other => return Err(format!("unknown --format {other} (expected json or csv)")),
                };
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(opts)
}

fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    let value = args
        .get(*i + 1)
        .ok_or_else(|| format!("{flag} needs a value"))?
        .clone();
    *i += 2;
    Ok(value)
}

fn in_range(date: &str, from: Option<&str>, to: Option<&str>) -> bool {
    // YYYY-MM-DD sorts lexically the same as chronologically.
    from.is_none_or(|f| date >= f) && to.is_none_or(|t| date <= t)
}

/// Every stored row, decoded. A missing/unreadable/corrupt database, or
/// any individually undecodable row, is skipped rather than failing —
/// "no data yet" is normal for a freshly enabled feature.
fn read_all(data_dir: &Path) -> Vec<ReportRow> {
    let Ok(db) = redb::Database::open(db_path(data_dir)) else {
        return Vec::new();
    };
    let Ok(txn) = db.begin_read() else {
        return Vec::new();
    };
    let Ok(table) = txn.open_table(TABLE) else {
        return Vec::new();
    };
    let Ok(iter) = table.iter() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in iter {
        let Ok((key, value)) = entry else { continue };
        let Some((date, repo_key, model)) = decode_key(key.value()) else {
            continue;
        };
        let Ok(row) = serde_json::from_slice::<Row>(value.value()) else {
            continue;
        };
        out.push(ReportRow {
            date,
            repo_key,
            repo_display: row.repo_display,
            model,
            cost_usd: row.cost_usd,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_creation_tokens: row.cache_creation_tokens,
            cache_read_tokens: row.cache_read_tokens,
        });
    }
    out
}

pub(crate) fn render(data_dir: &Path, default_repo_key: &str, opts: &Options) -> String {
    let rows = read_all(data_dir);
    if opts.all {
        return render_summary(&rows, opts);
    }
    let target = opts.repo_key.as_deref().unwrap_or(default_repo_key);
    let mut filtered: Vec<&ReportRow> = rows
        .iter()
        .filter(|r| r.repo_key == target && in_range(&r.date, opts.from.as_deref(), opts.to.as_deref()))
        .collect();
    filtered.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.model.cmp(&b.model)));
    render_rows(&filtered, &opts.format)
}

#[derive(Serialize)]
struct JsonRow<'a> {
    date: &'a str,
    repo: &'a str,
    repo_display: &'a str,
    model: &'a str,
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
}

fn render_rows(rows: &[&ReportRow], format: &Format) -> String {
    match format {
        Format::Json => {
            let json_rows: Vec<JsonRow> = rows
                .iter()
                .map(|r| JsonRow {
                    date: &r.date,
                    repo: &r.repo_key,
                    repo_display: &r.repo_display,
                    model: &r.model,
                    cost_usd: r.cost_usd,
                    input_tokens: r.input_tokens,
                    output_tokens: r.output_tokens,
                    cache_creation_tokens: r.cache_creation_tokens,
                    cache_read_tokens: r.cache_read_tokens,
                })
                .collect();
            serde_json::to_string(&json_rows).unwrap_or_else(|_| "[]".to_string())
        }
        Format::Csv => {
            let mut out = String::from(
                "date,repo,repo_display,model,cost_usd,input_tokens,output_tokens,cache_creation_tokens,cache_read_tokens\n",
            );
            for r in rows {
                let _ = writeln!(
                    out,
                    "{},{},{},{},{:.6},{},{},{},{}",
                    r.date,
                    csv_escape(&r.repo_key),
                    csv_escape(&r.repo_display),
                    csv_escape(&r.model),
                    r.cost_usd,
                    r.input_tokens,
                    r.output_tokens,
                    r.cache_creation_tokens,
                    r.cache_read_tokens
                );
            }
            out
        }
    }
}

/// RFC 4180's minimal escaping: wrap in double quotes (doubling any
/// embedded quote) only when the field contains a comma, quote, or
/// newline.
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn render_summary(rows: &[ReportRow], opts: &Options) -> String {
    let mut totals: HashMap<String, (String, f64, u64, u64, u64, u64)> = HashMap::new();
    for r in rows {
        if !in_range(&r.date, opts.from.as_deref(), opts.to.as_deref()) {
            continue;
        }
        let entry = totals
            .entry(r.repo_key.clone())
            .or_insert_with(|| (r.repo_display.clone(), 0.0, 0, 0, 0, 0));
        entry.1 += r.cost_usd;
        entry.2 += r.input_tokens;
        entry.3 += r.output_tokens;
        entry.4 += r.cache_creation_tokens;
        entry.5 += r.cache_read_tokens;
    }
    let mut summary: Vec<_> = totals.into_iter().collect();
    summary.sort_by(|a, b| b.1.1.partial_cmp(&a.1.1).unwrap_or(std::cmp::Ordering::Equal));

    match opts.format {
        Format::Json => {
            #[derive(Serialize)]
            struct SummaryRow<'a> {
                repo: &'a str,
                repo_display: &'a str,
                cost_usd: f64,
                input_tokens: u64,
                output_tokens: u64,
                cache_creation_tokens: u64,
                cache_read_tokens: u64,
            }
            let json_rows: Vec<SummaryRow> = summary
                .iter()
                .map(|(repo, (display, cost, i, o, cc, cr))| SummaryRow {
                    repo,
                    repo_display: display,
                    cost_usd: *cost,
                    input_tokens: *i,
                    output_tokens: *o,
                    cache_creation_tokens: *cc,
                    cache_read_tokens: *cr,
                })
                .collect();
            serde_json::to_string(&json_rows).unwrap_or_else(|_| "[]".to_string())
        }
        Format::Csv => {
            let mut out = String::from(
                "repo,repo_display,cost_usd,input_tokens,output_tokens,cache_creation_tokens,cache_read_tokens\n",
            );
            for (repo, (display, cost, i, o, cc, cr)) in &summary {
                let _ = writeln!(
                    out,
                    "{},{},{cost:.6},{i},{o},{cc},{cr}",
                    csv_escape(repo),
                    csv_escape(display)
                );
            }
            out
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --features analytics analytics::report::`
Expected: PASS, all eleven tests.

- [ ] **Step 6: Run the full feature-enabled and default suites**

Run: `cargo test --features analytics`
Expected: PASS, every test in the crate.
Run: `cargo test`
Expected: PASS, unaffected.

- [ ] **Step 7: Commit**

```bash
git add src/analytics.rs src/analytics/report.rs
git commit -m "feat: add the analytics report engine (filtering, JSON/CSV rendering)"
```

---

### Task 8: `ferrisbar report` CLI wiring

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `analytics::{parse_report_args, render, ReportOptions}` (Task 7), `repo_identity::resolve` (Task 3).
- Produces: a working `ferrisbar report [--repo KEY] [--from DATE] [--to DATE] [--all] [--format json|csv]` subcommand when built with the `analytics` feature; a clean nonzero-exit error when built without it.

- [ ] **Step 1: Add the dual `run_report` implementations**

In `src/main.rs`, add (after `dispatch_subcommand`, or any convenient top-level location):

```rust
#[cfg(feature = "analytics")]
fn run_report(data_dir: Option<&Path>, args: &[String]) {
    let Some(data_dir) = data_dir else {
        eprintln!("ferrisbar report: no data directory available on this platform");
        std::process::exit(1);
    };
    let opts = match analytics::parse_report_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("ferrisbar report: {msg}");
            std::process::exit(1);
        }
    };
    let cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let default_key = repo_identity::resolve(&cwd).key;
    print!("{}", analytics::render(data_dir, &default_key, &opts));
}

#[cfg(not(feature = "analytics"))]
fn run_report(_data_dir: Option<&Path>, _args: &[String]) {
    eprintln!("ferrisbar report: built without the `analytics` feature");
    std::process::exit(1);
}
```

- [ ] **Step 2: Add the dispatch arm**

In `dispatch_subcommand`'s `match args.as_slice() { ... }`, add a new arm immediately before the trailing `_ => { ... }` catch-all:

```rust
        [cmd, rest @ ..] if cmd == "report" => {
            run_report(data_dir, rest);
            true
        }
```

- [ ] **Step 3: Make the usage message reflect the build**

Replace the existing:

```rust
            eprintln!("Usage: {program_name} [setup [--project]]");
```

with a call to a new helper:

```rust
            eprintln!("{}", usage_line(&program_name));
```

and define, near `dispatch_subcommand`:

```rust
fn usage_line(program_name: &str) -> String {
    #[cfg(feature = "analytics")]
    {
        format!(
            "Usage: {program_name} [setup [--project]] [report [--repo KEY] [--from DATE] [--to DATE] [--all] [--format json|csv]]"
        )
    }
    #[cfg(not(feature = "analytics"))]
    {
        format!("Usage: {program_name} [setup [--project]]")
    }
}
```

- [ ] **Step 4: Verify both builds**

Run: `cargo build && cargo build --features analytics`
Expected: both succeed.

Run: `cargo run --features analytics -- report --bogus-flag` (from the repo root, with no stdin needed since `report` returns before reading it)
Expected: prints `ferrisbar report: unknown flag --bogus-flag` to stderr and exits nonzero (verify with `echo $?` on the prior command on Unix, or `$LASTEXITCODE` on Windows PowerShell).

Run: `cargo run --features analytics -- report --all`
Expected: prints `[]` (empty JSON array) — no data has ever been recorded in this dev environment's real data dir yet, and that is the documented empty-state behavior, not an error.

Run: `cargo run -- report` (default build, no `analytics` feature)
Expected: prints `ferrisbar report: built without the \`analytics\` feature` to stderr and exits nonzero.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire up the ferrisbar report subcommand"
```

---

### Task 9: CLI end-to-end tests

**Files:**
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: the compiled `ferrisbar` binary (built with `--features analytics` for these tests — see Step 0), `run_command`, `isolated`, `config_dir`, `data_dir` (all pre-existing helpers in this file).

- [ ] **Step 0: Confirm how this file's tests get an analytics-enabled binary**

`tests/cli.rs` spawns `env!("CARGO_BIN_EXE_ferrisbar")`, which Cargo builds according to whatever flags the `cargo test` invocation itself used. These new tests therefore only run meaningfully under `cargo test --features analytics` (Task 11 wires this into CI as a separate `just test-analytics` recipe). Add a doc comment above the new tests noting this, so a future reader isn't confused why `cargo test` (no features) skips them silently rather than failing — they still compile and run under the default build, but the `report` subcommand itself will exit nonzero with "built without the `analytics` feature," so they must be written to only assert on the feature-enabled behavior. Guard the whole block with `#[cfg(feature = "analytics")]` so they don't run (and fail) in a default-build `cargo test`.

- [ ] **Step 1: Write the failing tests**

Add to `tests/cli.rs`, after `unknown_subcommand_exits_nonzero_without_hanging`:

```rust
#[cfg(feature = "analytics")]
fn write_git_remote(repo_root: &Path, url: &str) {
    let git_dir = repo_root.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(
        git_dir.join("config"),
        format!("[remote \"origin\"]\n\turl = {url}\n"),
    )
    .unwrap();
}

#[cfg(feature = "analytics")]
fn write_analytics_config(home: &Path) {
    let dir = config_dir(home);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), "[analytics]\nenabled = true\n").unwrap();
}

#[cfg(feature = "analytics")]
#[test]
fn report_reflects_usage_ingested_by_the_background_refresh() {
    let (mut cmd, home) = isolated();
    write_analytics_config(home.path());

    let repo = tempfile::tempdir().unwrap();
    write_git_remote(repo.path(), "https://github.com/kerryhatcher/ferrisbar.git");

    let claude_config = tempfile::tempdir().unwrap();
    let transcripts = claude_config.path().join("projects").join("proj");
    fs::create_dir_all(&transcripts).unwrap();
    let today = today_utc_date();
    fs::write(
        transcripts.join("a.jsonl"),
        format!(
            r#"{{"cwd":"{}","timestamp":"{today}T10:00:00Z","requestId":"req_1","message":{{"model":"claude-sonnet-5","id":"msg_1","usage":{{"input_tokens":1000000}}}}}}"#,
            escape_for_string_literal(repo.path())
        ),
    )
    .unwrap();

    cmd.env(
        "CLAUDE_CONFIG_DIR",
        claude_config.path().to_str().unwrap(),
    )
    .env_remove("FERRISBAR_COST_TTL_SECONDS") // this test needs the real ingestion path, not the disabled default
    .args(["--internal-refresh-daily-cost"]);
    let refresh_output = cmd.output().expect("failed to run --internal-refresh-daily-cost");
    assert!(refresh_output.status.success());

    let report_output = run_command(
        &["report", "--all", "--format", "json"],
        &[
            (
                "HOME",
                home.path().to_str().unwrap(),
            ),
            (
                "CLAUDE_CONFIG_DIR",
                claude_config.path().to_str().unwrap(),
            ),
        ],
        None,
    );
    assert!(report_output.status.success());
    let stdout = String::from_utf8_lossy(&report_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let rows = parsed.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["repo"], "remote:github.com/kerryhatcher/ferrisbar");
    // sonnet-5 input rate is $2/Mtok; 1M input tokens = $2.
    assert!((rows[0]["cost_usd"].as_f64().unwrap() - 2.0).abs() < 1e-9);
}

#[cfg(feature = "analytics")]
#[test]
fn report_defaults_to_the_repo_resolved_from_cwd() {
    let (mut cmd, home) = isolated();
    write_analytics_config(home.path());

    let repo = tempfile::tempdir().unwrap();
    write_git_remote(repo.path(), "https://github.com/someone/example.git");

    let claude_config = tempfile::tempdir().unwrap();
    let transcripts = claude_config.path().join("projects").join("proj");
    fs::create_dir_all(&transcripts).unwrap();
    let today = today_utc_date();
    fs::write(
        transcripts.join("a.jsonl"),
        format!(
            r#"{{"cwd":"{}","timestamp":"{today}T10:00:00Z","requestId":"req_1","message":{{"model":"claude-sonnet-5","id":"msg_1","usage":{{"input_tokens":1000000}}}}}}"#,
            escape_for_string_literal(repo.path())
        ),
    )
    .unwrap();

    cmd.env("CLAUDE_CONFIG_DIR", claude_config.path().to_str().unwrap())
        .env_remove("FERRISBAR_COST_TTL_SECONDS")
        .args(["--internal-refresh-daily-cost"]);
    assert!(cmd.output().unwrap().status.success());

    // Run `report` with cwd set to the repo itself — no `--repo` flag.
    let report_output = run_command(
        &["report"],
        &[
            ("HOME", home.path().to_str().unwrap()),
            ("CLAUDE_CONFIG_DIR", claude_config.path().to_str().unwrap()),
        ],
        Some(repo.path()),
    );
    assert!(report_output.status.success());
    let stdout = String::from_utf8_lossy(&report_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let rows = parsed.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["repo"], "remote:github.com/someone/example");
}

#[cfg(feature = "analytics")]
#[test]
fn report_with_no_data_yet_prints_an_empty_array_and_exits_zero() {
    let output = run_command(&["report", "--all"], &[], None);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[]");
}

#[test]
fn report_without_the_analytics_feature_exits_nonzero() {
    // Meaningful only in a build without `analytics`; under
    // `--features analytics` this assertion would (correctly) fail, so
    // this test is deliberately not `#[cfg(feature = "analytics")]` —
    // it documents and checks the *other* build's behavior. Run this
    // specific test only via `just test` (default build), not
    // `just test-analytics`.
    let output = run_command(&["report"], &[], None);
    assert!(!output.status.success());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --features analytics report_ `
Expected: FAIL until Tasks 1–8 are all in place (if this task is executed after them in order, they should already pass on first try — this step exists to catch any integration gap between tasks, e.g. a helper name mismatch).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --features analytics`
Expected: PASS, every test including the new ones.

Run: `cargo test` (default build)
Expected: PASS — `report_without_the_analytics_feature_exits_nonzero` runs and passes; the four `#[cfg(feature = "analytics")]` tests are skipped, not failed.

- [ ] **Step 4: Commit**

```bash
git add tests/cli.rs
git commit -m "test: add end-to-end coverage for ferrisbar report"
```

---

### Task 10: Update the README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add the `[analytics]` block to the config file example**

In README.md's "### The config file" section (the fenced `toml` block, currently ending after `[budget]`'s keys), append:

```toml

[analytics]
enabled      = false
```

- [ ] **Step 2: Add explanatory prose**

Immediately after the existing `[budget]` paragraph (which ends "...independent of `display.bar_width`."), add:

```markdown
`[analytics]` is **off by default** and, unlike every other config block,
has no effect at all unless ferrisbar was built with the optional
`analytics` Cargo feature (`cargo build --features analytics`) — a
pure-Rust embedded datastore is not part of the default build. When both
the feature and `enabled = true` are set, the same background refresh
that computes the daily-cost chip also resolves each transcript record's
git repository (its `origin` remote, normalized, or the repo's own
folder name when there is no remote) and records that day's cost and
token totals per repo and model to `<data dir>/analytics.redb`. The
`ferrisbar report` subcommand reads that store:

```bash
ferrisbar report                              # this repo, full history, JSON
ferrisbar report --repo remote:github.com/org/name
ferrisbar report --from 2026-08-01 --to 2026-08-31 --format csv
ferrisbar report --all                        # one summary row per tracked repo
```

Recording only ever looks at the current and previous UTC day — there is
no backfill of history from before the feature was enabled, and no
cross-machine merging of separate `analytics.redb` files.
```

- [ ] **Step 3: Verify the doc example still parses**

Run: `cargo test config::tests::template_round_trips_including_analytics` (from Task 2)
Expected: PASS — confirms the `[analytics]` keys shown in the README example match what `config.rs` actually parses (this test doesn't read the README directly, but is the closest existing guard against the two drifting; re-run it here as a sanity check after editing docs).

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document the [analytics] config block and ferrisbar report"
```

---

### Task 11: Wire the `analytics` feature into CI, and the supply-chain vet entry for `redb`

**Files:**
- Modify: `justfile`
- Modify: `.github/workflows/ci.yml`
- Modify: `supply-chain/config.toml`

**Interfaces:**
- Produces: `just test-analytics`, `just lint-analytics`, `just deny-analytics` recipes; corresponding CI steps; a `cargo vet check` pass that covers `redb` and any of its transitive dependencies.

- [ ] **Step 1: Add the new `just` recipes**

In `justfile`, add near `test`/`lint`/`deny`:

```make
# Run the test suite with the analytics feature enabled — the default
# `test` recipe never compiles src/analytics/store.rs or report.rs at
# all, since the feature is optional and off by default.
test-analytics:
    cargo test --features analytics

# Lint the analytics feature's code the same way `lint` covers the
# default build.
lint-analytics:
    cargo clippy --all-targets --features analytics -- -D warnings

# Check licenses/bans for redb (and any of its transitive deps) too —
# deny.toml's [graph] all-features = false means the default `deny`
# recipe skips anything gated behind an inactive feature.
deny-analytics:
    cargo deny check --all-features
```

And add both new recipes to the `ci` chain:

```make
ci: fmt lint lint-analytics test test-analytics audit msrv deny deny-analytics trivy vet geiger
```

- [ ] **Step 2: Verify the new recipes work**

Run: `just test-analytics`
Expected: PASS (same tests as `cargo test --features analytics`, already verified in earlier tasks).

Run: `just lint-analytics`
Expected: PASS with no warnings. If clippy flags anything in `src/analytics/`, `src/repo_identity.rs`, or the touched parts of `src/cost.rs`/`src/main.rs`, fix it now — per `CLAUDE.md`, a genuinely-wrong lint gets an `#[allow(...)]` with a comment explaining why; everything else gets fixed.

Run: `just deny-analytics`
Expected: PASS if `redb`'s license is `MIT` or `Apache-2.0` (both already in `deny.toml`'s `[licenses] allow` list). If it or a transitive dependency uses a license not on that list, add it to `[licenses] allow` in `deny.toml` and re-run.

- [ ] **Step 3: Add the CI jobs**

In `.github/workflows/ci.yml`, add a step to the existing `test` job (after `- run: just test`):

```yaml
      - run: just test-analytics
```

Add a step to the existing `lint` job (after `- run: just lint`):

```yaml
      - run: just lint-analytics
```

Add a step to the existing `security` job (after `- run: just deny`):

```yaml
      - run: just deny-analytics
```

- [ ] **Step 4: Run `cargo vet check` and record what it flags**

Run: `cargo vet check`
Expected: FAILS, listing `redb` (and possibly its own transitive dependencies, if any — `redb`'s dependency tree is typically small/pure-Rust, but check the actual output) as unaudited, now that Task 1 added it to `Cargo.toml`/`Cargo.lock`. `cargo vet` audits everything in `Cargo.lock` regardless of whether a feature gating it is active by default, so this step is required for `just vet`/`just ci` to pass at all from this point on — it is not conditional on the `analytics` feature the way Steps 1–3 are.

- [ ] **Step 5: Add exemption entries**

For every crate `cargo vet check` listed in Step 4, add an entry to `supply-chain/config.toml`, in alphabetical order among the existing `[[exemptions.*]]` blocks, using the exact crate name and resolved version from the `cargo vet check` output:

```toml
[[exemptions.redb]]
version = "<exact version from Cargo.lock>"
criteria = "safe-to-deploy"
```

(Repeat for any additional crates the output lists — matching the existing house style of a plain `safe-to-deploy`/`safe-to-run` self-exemption rather than a full audit, consistent with every other entry already in this file.)

- [ ] **Step 6: Verify `cargo vet check` passes**

Run: `cargo vet check`
Expected: PASS.

- [ ] **Step 7: Run the full CI chain locally**

Run: `just ci`
Expected: PASS end to end.

- [ ] **Step 8: Commit**

```bash
git add justfile .github/workflows/ci.yml supply-chain/config.toml
git commit -m "ci: wire the analytics feature into test/lint/deny, vet the redb dependency"
```

---

## Self-Review Notes

- **Spec coverage:** storage engine (Task 1), config opt-in (Task 2), repo identity + URL normalization (Task 3), `cwd` capture (Task 4), the store and its overwrite-not-accumulate semantics (Task 5), piggybacking on the existing refresh with today+yesterday bucketing (Task 6), report filtering/formats (Task 7), CLI wiring and default-scope resolution (Task 8), end-to-end coverage across both builds (Task 9), docs (Task 10), dependency/CI/supply-chain impact (Task 11). Every "Decisions made during brainstorming" row in the spec maps to at least one task above.
- **Out-of-scope items respected:** no backfill logic anywhere (Task 6 only ever computes today/yesterday), no HTML/PDF renderer, no `--output` flag (stdout only, per Task 8), no per-branch breakdown (repo+date+model only, per Task 5's key encoding).
- **Type consistency check:** `Sink::new(enabled: bool, today: String, yesterday: String)` and `Sink::record(&mut self, rec: &ParsedRecord, cost: f64)` are identical across the Task 5 stub, Task 5's real implementation, and every call site in Task 6 and Task 5's own tests. `RepoIdentity { key, display }` field names match between Task 3's definition and every consumer (Task 5's `Sink::record`, Task 8's `run_report`). `Row`'s five fields match between Task 5's definition, Task 5's tests, and Task 7's `read_all`/`ReportRow` construction.
