# Repo Cost Today Segment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Append a `repo $X.XX` segment to the statusline's existing daily-cost line, showing today's cost for whichever repo the current render's `cwd` resolves to, read live from the analytics store.

**Architecture:** A new dual-build query function (`analytics::today_repo_cost`, real redb-backed / no-op stub, mirroring `Sink`'s existing pattern) sums today's per-model rows for one repo key. `cost::daily_chip` gains two parameters (`cwd`, `analytics_enabled`), resolves repo identity, calls the query, and appends the segment when positive. `main.rs`'s one call site is updated to pass them through.

**Tech Stack:** Rust, `redb` (already a dependency behind the existing `analytics` feature) — no new dependencies.

## Global Constraints

- MSRV is 1.85.1.
- Never panic on input — every fallible step in the new query function degrades to `None`, never a panic or error propagated to the render.
- Clippy pedantic/nursery with `-D warnings`; plain `pub`, never `pub(crate)` (this crate is bin-only with no `pub mod` anywhere, so `pub(crate)` is always redundant — this has bitten prior work in this repo more than once).
- No new runtime dependency.
- No new config toggle — this segment follows `[analytics] enabled` with no separate switch, and shows only when the repo's today-total is positive (a `$0.00` row is treated the same as no data).
- Run `just fmt lint test` and, since this touches feature-gated code, `just lint-analytics test-analytics` too, before every commit in this plan.

---

### Task 1: `analytics::today_repo_cost` query function

**Files:**
- Modify: `src/cost.rs` (widen `today_utc_date`/`now_unix_secs` visibility)
- Modify: `src/analytics.rs` (dual re-export/stub)
- Modify: `src/analytics/store.rs` (real implementation + tests)

**Interfaces:**
- Consumes: `crate::cost::today_utc_date(now_unix_secs: i64) -> String` and `crate::cost::now_unix_secs() -> i64` (existing, widened to `pub` by this task), `redb`'s existing read-side API as already used by `analytics::report::read_all` (`Database::open`, `begin_read`, `open_table`, `iter`), `store::{db_path, decode_key, Row, TABLE}` (existing).
- Produces: `analytics::today_repo_cost(enabled: bool, data_dir: &Path, repo_key: &str) -> Option<f64>` — identical signature in both the `#[cfg(feature = "analytics")]` real build and the `#[cfg(not(feature = "analytics"))]` stub. Task 2 calls this from `cost::daily_chip`.

- [ ] **Step 1: Widen `today_utc_date` and `now_unix_secs` to `pub`**

In `src/cost.rs`, change:

```rust
fn today_utc_date(now_unix_secs: i64) -> String {
```

to:

```rust
pub fn today_utc_date(now_unix_secs: i64) -> String {
```

and change:

```rust
fn now_unix_secs() -> i64 {
```

to:

```rust
pub fn now_unix_secs() -> i64 {
```

(Both stay exactly where they are; only the `pub` keyword is added. Every existing call site inside `cost.rs` keeps working unchanged — widening visibility never breaks an existing caller.)

- [ ] **Step 2: Write the failing tests**

Add to `src/analytics/store.rs`'s `mod tests` (the file already has `use super::*;` and the `usage_record(date: &str, model: &str, cwd: &str) -> ParsedRecord` helper — reuse it; `Sink::record`'s `cost` is a separate argument from the record itself, so no new fixture helper is needed):

```rust
#[test]
fn today_repo_cost_sums_across_models_for_the_matching_repo_and_date() {
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap(); // no .git — resolves to a `local:` identity
    let cwd = repo.path().to_str().unwrap();
    let today = crate::cost::today_utc_date(crate::cost::now_unix_secs());

    let mut sink = Sink::new(true, today.clone(), "1970-01-01".to_string());
    sink.record(&usage_record(&today, "claude-sonnet-5", cwd), 2.0);
    sink.record(&usage_record(&today, "claude-opus-5", cwd), 3.0);
    sink.flush(dir.path());

    let repo_key = repo_identity::resolve(cwd).key;
    let total = today_repo_cost(true, dir.path(), &repo_key);
    assert!(
        (total.unwrap() - 5.0).abs() < 1e-9,
        "sums across both models for today"
    );
}

#[test]
fn today_repo_cost_ignores_a_different_repo_or_date() {
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let other_repo = tempfile::tempdir().unwrap();
    let cwd = repo.path().to_str().unwrap();
    let today = crate::cost::today_utc_date(crate::cost::now_unix_secs());
    let yesterday = "2020-01-01".to_string(); // definitely not today

    let mut sink = Sink::new(true, today.clone(), yesterday.clone());
    sink.record(&usage_record(&today, "claude-sonnet-5", cwd), 2.0);
    sink.record(&usage_record(&yesterday, "claude-sonnet-5", cwd), 9.0); // wrong date
    sink.flush(dir.path());

    let other_key = repo_identity::resolve(other_repo.path().to_str().unwrap()).key;
    assert_eq!(
        today_repo_cost(true, dir.path(), &other_key),
        None,
        "a repo with no rows for today must return None, not 0.0"
    );
}

#[test]
fn today_repo_cost_missing_store_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let repo_key = repo_identity::resolve("/does/not/matter").key;
    assert_eq!(today_repo_cost(true, dir.path(), &repo_key), None);
}

#[test]
fn today_repo_cost_disabled_touches_no_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("never-created");
    let repo_key = repo_identity::resolve("/does/not/matter").key;
    assert_eq!(today_repo_cost(false, &nonexistent, &repo_key), None);
    assert!(
        !nonexistent.exists(),
        "a disabled query must not create the data directory"
    );
}

#[test]
fn today_repo_cost_a_real_zero_cost_row_is_some_zero_not_none() {
    // A row genuinely exists for today (e.g. an unpriced model's usage,
    // recorded with cost 0.0) — this must be distinguished from "no rows
    // at all," which is what `None` means. Whether a zero total is worth
    // *displaying* is `daily_chip`'s call (Task 2), not this function's.
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let cwd = repo.path().to_str().unwrap();
    let today = crate::cost::today_utc_date(crate::cost::now_unix_secs());

    let mut sink = Sink::new(true, today.clone(), "1970-01-01".to_string());
    sink.record(&usage_record(&today, "claude-future-model-9", cwd), 0.0);
    sink.flush(dir.path());

    let repo_key = repo_identity::resolve(cwd).key;
    assert_eq!(today_repo_cost(true, dir.path(), &repo_key), Some(0.0));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --features analytics analytics::store::tests::today_repo_cost`
Expected: FAIL — `today_repo_cost` does not exist yet (compile error).

- [ ] **Step 4: Implement `today_repo_cost`**

Add to `src/analytics/store.rs`, below `Sink`'s `impl` block (this file already has `use redb::TableDefinition;` at the top; add `use redb::ReadableTable;` alongside it — needed for `.iter()`/`.get()` on the opened table, matching `report.rs`'s existing import):

```rust
/// Sums `cost_usd` across every model for today's date and `repo_key`.
/// `None` means no rows exist for this repo today at all — disabled,
/// no store yet, or genuinely no activity. `Some(0.0)` is a real,
/// different case (e.g. today's only activity was an unpriced model)
/// and is returned as-is; it's `daily_chip`'s job (Task 2), not this
/// function's, to decide that a zero total isn't worth displaying.
/// Read-only: never creates the file and never triggers a refresh —
/// this only ever reads whatever the last background refresh already
/// committed.
pub fn today_repo_cost(enabled: bool, data_dir: &Path, repo_key: &str) -> Option<f64> {
    if !enabled {
        return None;
    }
    let today = crate::cost::today_utc_date(crate::cost::now_unix_secs());
    let Ok(db) = redb::Database::open(db_path(data_dir)) else {
        return None;
    };
    let Ok(txn) = db.begin_read() else {
        return None;
    };
    let Ok(table) = txn.open_table(TABLE) else {
        return None;
    };
    let Ok(iter) = table.iter() else {
        return None;
    };
    let mut total = 0.0_f64;
    let mut found = false;
    for entry in iter {
        let Ok((key, value)) = entry else { continue };
        let Some((date, key_repo, _model)) = decode_key(key.value()) else {
            continue;
        };
        if date != today || key_repo != repo_key {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<Row>(value.value()) else {
            continue;
        };
        total += row.cost_usd;
        found = true;
    }
    if found {
        Some(total)
    } else {
        None
    }
}
```

- [ ] **Step 5: Re-export from `src/analytics.rs`**

Add the re-export next to `Sink`'s existing one:

```rust
// `cost.rs`'s `daily_chip` calls this unconditionally (see `Sink`'s own
// rationale above), so it's a real, always-live re-export.
#[cfg(feature = "analytics")]
pub use store::today_repo_cost;
```

And add the matching stub next to `Sink`'s stub `impl` block (same file, `#[cfg(not(feature = "analytics"))]` section):

```rust
// The zero-cost no-op used in place of `store::today_repo_cost` when the
// `analytics` feature is off. `cost.rs`'s `daily_chip` still calls it
// unconditionally, so this is not dead code in a plain build either.
#[cfg(not(feature = "analytics"))]
pub fn today_repo_cost(_enabled: bool, _data_dir: &std::path::Path, _repo_key: &str) -> Option<f64> {
    None
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --features analytics analytics::store::tests::today_repo_cost`
Expected: PASS, all five new tests.

- [ ] **Step 7: Run clippy and the full suite in both configurations**

Run: `cargo clippy --all-targets --features analytics -- -D warnings`
Expected: clean.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.
Run: `cargo test --features analytics` then plain `cargo test`
Expected: both PASS, no regressions.

- [ ] **Step 8: Commit**

```bash
git add src/cost.rs src/analytics.rs src/analytics/store.rs
git commit -m "feat: add analytics::today_repo_cost query function"
```

---

### Task 2: Wire the segment into `daily_chip`

**Files:**
- Modify: `src/cost.rs` (`daily_chip` signature + new formatting helper)
- Modify: `src/main.rs` (update the one call site)

**Interfaces:**
- Consumes: `analytics::today_repo_cost` (Task 1), `repo_identity::resolve(cwd: &str) -> RepoIdentity` (existing).
- Produces: `cost::daily_chip(cfg: &CostConfig, data_dir: Option<&Path>, cwd: &str, analytics_enabled: bool) -> Option<String>` — signature change; the one existing call site in `main.rs` is updated in this same task, so nothing outside this task depends on the old three-argument signature by the time it lands.

- [ ] **Step 1: Write the failing tests**

Add to `src/cost.rs`'s `mod tests`, near the existing `format_daily_chip_*`/`daily_chip_*` tests:

```rust
#[test]
fn format_repo_segment_keeps_cents_under_a_dollar() {
    assert_eq!(
        format_repo_segment(0.5),
        format!("{DIM}repo{RESET} {GREEN}$0.50{RESET}")
    );
}

#[test]
fn format_repo_segment_rounds_to_whole_dollars_at_and_above_one() {
    assert_eq!(
        format_repo_segment(12.34),
        format!("{DIM}repo{RESET} {GREEN}$12{RESET}")
    );
}

// This test relies on `Sink`'s and `today_repo_cost`'s *real* behavior
// (an actual redb write, then an actual read-back) to prove the segment
// appears — under a plain build both are no-op stubs with the same
// signatures, so this would compile but fail its assertions at runtime
// rather than fail to compile. Gate it so it only runs where it's
// meaningful; `daily_chip_omits_the_repo_segment_when_analytics_disabled`
// below needs no such gate, since it passes `analytics_enabled: false`
// and never touches `Sink`/`today_repo_cost` in either build.
#[cfg(feature = "analytics")]
#[test]
fn daily_chip_appends_the_repo_segment_when_analytics_has_data() {
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap(); // no .git — resolves to a `local:` identity
    let cwd = repo.path().to_str().unwrap();
    let today = today_utc_date(now_unix_secs());
    cost_cache::write_cache(
        dir.path(),
        &cost_cache::CachePayload {
            date: today.clone(),
            total_usd: 4.2,
            by_model: Vec::new(),
            ..cost_cache::CachePayload::default()
        },
    )
    .unwrap();

    let mut sink = crate::analytics::Sink::new(true, today, "1970-01-01".to_string());
    sink.record(
        &ParsedRecord::for_test(
            &today_utc_date(now_unix_secs()),
            "claude-sonnet-5",
            cwd,
            Usage {
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
        2.0,
    );
    sink.flush(dir.path());

    let chip = daily_chip(&CostConfig::default(), Some(dir.path()), cwd, true).unwrap();
    assert!(chip.contains("repo"), "got: {chip}");
    assert!(chip.contains("$2"), "got: {chip}");
}

#[test]
fn daily_chip_omits_the_repo_segment_when_analytics_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let cwd = repo.path().to_str().unwrap();
    let today = today_utc_date(now_unix_secs());
    cost_cache::write_cache(
        dir.path(),
        &cost_cache::CachePayload {
            date: today,
            total_usd: 4.2,
            by_model: Vec::new(),
            ..cost_cache::CachePayload::default()
        },
    )
    .unwrap();

    // `analytics_enabled = false` here means the repo segment must be
    // absent even if a store happens to exist on disk.
    let chip = daily_chip(&CostConfig::default(), Some(dir.path()), cwd, false).unwrap();
    assert!(!chip.contains("repo"), "got: {chip}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --features analytics cost::tests::format_repo_segment cost::tests::daily_chip_appends cost::tests::daily_chip_omits`
Expected: FAIL — `format_repo_segment` doesn't exist and `daily_chip` doesn't take four arguments yet (compile errors).

- [ ] **Step 3: Add `format_repo_segment` and update `daily_chip`**

Add just above `pub fn daily_chip`:

```rust
/// Formats the segment appended to the daily chip when today's per-repo
/// total is available and positive: `repo $X.XX` (or `repo $Y` once the
/// amount reaches a whole dollar), matching `format_daily_chip`'s own
/// cents-vs-whole-dollar rule.
fn format_repo_segment(cost: f64) -> String {
    let amount = if cost < 1.0 {
        format!("${cost:.2}")
    } else {
        format!("${cost:.0}")
    };
    format!("{DIM}repo{RESET} {GREEN}{amount}{RESET}")
}
```

Change `daily_chip` from:

```rust
pub fn daily_chip(cfg: &CostConfig, data_dir: Option<&Path>) -> Option<String> {
    if !cfg.show_daily {
        return None;
    }
    let payload = fresh_same_day_cache(cfg.ttl_seconds, data_dir?)?;
    let daily = DailyTotal {
        total_usd: payload.total_usd,
        by_model: payload.by_model,
    };
    Some(format_daily_chip(&daily, cfg.breakdown_min_usd))
}
```

to:

```rust
pub fn daily_chip(
    cfg: &CostConfig,
    data_dir: Option<&Path>,
    cwd: &str,
    analytics_enabled: bool,
) -> Option<String> {
    if !cfg.show_daily {
        return None;
    }
    let data_dir = data_dir?;
    let payload = fresh_same_day_cache(cfg.ttl_seconds, data_dir)?;
    let daily = DailyTotal {
        total_usd: payload.total_usd,
        by_model: payload.by_model,
    };
    let mut chip = format_daily_chip(&daily, cfg.breakdown_min_usd);
    if analytics_enabled {
        let repo_key = crate::repo_identity::resolve(cwd).key;
        let repo_cost = crate::analytics::today_repo_cost(true, data_dir, &repo_key)
            .filter(|cost| *cost > 0.0);
        if let Some(cost) = repo_cost {
            let _ = write!(chip, " {DIM}│{RESET} {}", format_repo_segment(cost));
        }
    }
    Some(chip)
}
```

- [ ] **Step 4: Fix the existing call sites broken by the signature change**

`src/cost.rs`'s own `mod tests` has several pre-existing `daily_chip(...)` calls with the old two-argument signature (search the file for `daily_chip(&CostConfig` / `daily_chip(&cfg` to find every one — there are more than one, in tests named along the lines of `daily_chip_disabled_returns_none`, `daily_chip_zero_ttl_returns_none`, `daily_chip_no_data_dir_returns_none`, `daily_chip_no_cache_yet_returns_none`, `daily_chip_reads_a_fresh_same_day_cache`, `daily_chip_ignores_a_stale_dated_cache`, `daily_chip_refreshes_a_wrong_dated_cache_even_when_ttl_fresh`, and any others `cargo test` surfaces as compile errors — fix every one the compiler flags, not just this list). For each, add `"/tmp/does-not-matter-for-this-test", false` as the two new trailing arguments — none of these tests care about the repo segment, so passing `analytics_enabled: false` keeps their existing assertions unchanged (a placeholder `cwd` string is fine precisely because `analytics_enabled: false` means it's never touched).

Also check `src/main.rs` for any other reference — there should be exactly one, in `main()`, at the `if let Some(daily) = cost::daily_chip(&cfg.cost, data_dir.as_deref())` line.

- [ ] **Step 5: Update `main.rs`'s call site**

Change:

```rust
    if let Some(daily) = cost::daily_chip(&cfg.cost, data_dir.as_deref()) {
        output.push('\n');
        output.push_str(&daily);
    }
```

to:

```rust
    if let Some(daily) =
        cost::daily_chip(&cfg.cost, data_dir.as_deref(), &cwd, cfg.analytics.enabled)
    {
        output.push('\n');
        output.push_str(&daily);
    }
```

(`cwd` is already in scope at this point in `main()` — it's computed a few lines earlier via `let cwd = payload.cwd(&process_cwd);`, and `cfg.analytics` already exists as a config field regardless of whether the `analytics` Cargo feature is compiled in.)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --features analytics cost::`
Expected: PASS, including every pre-existing `cost::tests` test (now compiling with the extra two arguments) and the four new tests from Step 1.
Run: `cargo test cost::` (no features)
Expected: PASS — `daily_chip_appends_the_repo_segment_when_analytics_has_data` is compiled out (not failed) in this build per its `#[cfg(feature = "analytics")]` gate from Step 1; every other test, including `daily_chip_omits_the_repo_segment_when_analytics_disabled`, runs and passes here too.

- [ ] **Step 7: Run clippy and the full suite in both configurations**

Run: `cargo clippy --all-targets --features analytics -- -D warnings`
Expected: clean.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.
Run: `cargo test --features analytics` then plain `cargo test`
Expected: both PASS.

- [ ] **Step 8: Commit**

```bash
git add src/cost.rs src/main.rs
git commit -m "feat: append a repo-cost segment to the daily chip"
```

---

### Task 3: End-to-end coverage

**Files:**
- Modify: `tests/cli.rs`

**Interfaces:**
- Consumes: `isolated()`, `command_with_home(home: &Path) -> Command`, `write_analytics_config(home: &Path)`, `write_git_remote(repo_root: &Path, url: &str)`, `today_utc_date() -> String`, `escape_for_string_literal(path: &Path) -> String` (all existing helpers in this file — reuse them, do not redefine).

- [ ] **Step 1: Write the failing test**

Add to `tests/cli.rs`, in the same `#[cfg(feature = "analytics")]` block as the existing ingestion tests (after `report_defaults_to_the_repo_resolved_from_cwd`, before `report_with_no_data_yet_prints_an_empty_array_and_exits_zero`):

```rust
#[cfg(feature = "analytics")]
#[test]
fn daily_chip_shows_a_repo_segment_for_the_current_repo() {
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

    // Real ingestion: same as the existing report_reflects_usage_ingested
    // test, populating both cost-cache.json and the analytics store.
    cmd.env("CLAUDE_CONFIG_DIR", claude_config.path().to_str().unwrap())
        .env_remove("FERRISBAR_COST_TTL_SECONDS")
        .args(["--internal-refresh-daily-cost"]);
    let refresh_output = cmd
        .output()
        .expect("failed to run --internal-refresh-daily-cost");
    assert!(refresh_output.status.success());

    // A normal render (no subcommand), sharing the same home so it reads
    // the cache/store the refresh above just wrote. `FERRISBAR_COST_TTL_SECONDS`
    // must not stay at isolated()'s default of "0" here — that value means
    // "disable the daily line entirely," which would hide the very segment
    // this test is checking for, not just skip a background refresh.
    let mut render_cmd = command_with_home(home.path());
    render_cmd
        .env("CLAUDE_CONFIG_DIR", claude_config.path().to_str().unwrap())
        .env_remove("FERRISBAR_COST_TTL_SECONDS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let payload = format!(
        r#"{{"model":{{"display_name":"Sonnet"}},"workspace":{{"current_dir":"{}"}},"context_window":{{"remaining_percentage":100.0,"total_tokens":1000000}}}}"#,
        escape_for_string_literal(repo.path())
    );
    let mut child = render_cmd.spawn().expect("failed to spawn ferrisbar");
    child
        .stdin
        .take()
        .expect("child stdin handle")
        .write_all(payload.as_bytes())
        .expect("failed to write to child stdin");
    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let daily_chip_line = stdout.lines().nth(1).unwrap_or("");
    assert!(
        daily_chip_line.contains("repo"),
        "expected a repo segment on the daily chip line: {stdout}"
    );
    assert!(
        daily_chip_line.contains('$'),
        "expected a dollar amount on the daily chip line: {stdout}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --features analytics daily_chip_shows_a_repo_segment_for_the_current_repo`
Expected: FAIL before Task 1/2 land (if run out of order); PASS immediately if Tasks 1-2 are already in place, since this task only adds coverage for behavior those tasks already implemented — either outcome is fine, this step exists to catch any integration gap between the tasks (e.g. a helper name mismatch).

- [ ] **Step 3: Run the full suite in both configurations**

Run: `cargo test --features analytics`
Expected: PASS, every test including the new one.
Run: `cargo test` (default build)
Expected: PASS, unaffected — this new test is `#[cfg(feature = "analytics")]`-gated, so it's compiled out, not failed, in a plain build.

- [ ] **Step 4: Commit**

```bash
git add tests/cli.rs
git commit -m "test: add end-to-end coverage for the daily chip's repo segment"
```

---

## Self-Review Notes

- **Spec coverage:** the query function and its dual-build shape (Task 1), the formatting/placement/no-separate-toggle decisions (Task 2), and end-to-end proof of the whole pipeline (Task 3) all map directly to the design spec's Architecture/Formatting/Testing sections. Every "Decisions made during brainstorming" row in the spec (today-only scope, `repo $X.XX` label, no toggle, live redb read) is implemented exactly as decided.
- **Out-of-scope items respected:** no change to `ferrisbar report`'s behavior, no all-time total, no new config key, no change to how the background refresh writes to redb (Task 1 only adds a reader).
- **Type consistency check:** `today_repo_cost(enabled: bool, data_dir: &Path, repo_key: &str) -> Option<f64>` is identical across Task 1's stub, Task 1's real implementation, and every call site in Task 1's own tests and Task 2's `daily_chip`. `daily_chip`'s new signature (`cfg`, `data_dir`, `cwd`, `analytics_enabled`) matches between Task 2's own tests and the `main.rs` call site update in the same task.
