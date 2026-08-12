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

pub const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("cost_rows");

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Row {
    pub repo_display: String,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

impl Row {
    fn accumulate(&mut self, rec: &ParsedRecord, cost: f64) {
        self.cost_usd += cost;
        // Saturating, not `+=`: token counts are parsed unclamped from
        // transcript JSON (untrusted input), so a plain `+=` risks an
        // overflow panic in a debug build if two records land near
        // `u64::MAX` — the one integer-overflow-panic risk in an otherwise
        // all-`f64` cost codebase. Never panic on input.
        self.input_tokens = self.input_tokens.saturating_add(rec.usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(rec.usage.output_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(rec.usage.cache_creation_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(rec.usage.cache_read_tokens);
    }
}

// Public for `report`'s reader, which opens the same on-disk database from
// a fresh path.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("analytics.redb")
}

fn encode_key(date: &str, repo_key: &str, model: &str) -> String {
    format!("{date}\0{repo_key}\0{model}")
}

/// Inverse of `encode_key`. `None` for a key that doesn't split into
/// exactly three `\0`-separated parts — defensive against a store written
/// by some future, differently-shaped version of this code.
// Public for `report`'s reader.
pub fn decode_key(raw: &str) -> Option<(String, String, String)> {
    let mut parts = raw.split('\0');
    let date = parts.next()?.to_string();
    let repo_key = parts.next()?.to_string();
    let model = parts.next()?.to_string();
    if parts.next().is_some() {
        return None;
    }
    Some((date, repo_key, model))
}

pub struct Sink {
    enabled: bool,
    today: String,
    yesterday: String,
    rows: HashMap<String, Row>,
    repo_cache: HashMap<String, RepoIdentity>,
}

impl Sink {
    pub fn new(enabled: bool, today: String, yesterday: String) -> Self {
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
    pub fn record(&mut self, rec: &ParsedRecord, cost: f64) {
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
    pub fn flush(self, data_dir: &Path) {
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

/// Sums `cost_usd` across every model for today's date and `repo_key`.
/// `None` covers every reason there's nothing to report: disabled, no
/// store yet, genuinely no activity for this repo today, or the store
/// being unreadable right now — locked by a concurrent refresh, or
/// mid-repair after an interrupted write.
/// `Some(0.0)` is a real, different case (e.g. today's only activity was
/// an unpriced model) and is returned as-is; it's `daily_chip`'s job
/// (Task 2), not this function's, to decide that a zero total isn't
/// worth displaying.
/// Read-only: never creates the file and never triggers a refresh —
/// this only ever reads whatever the last background refresh already
/// committed.
///
/// `cost.rs`'s `daily_chip` calls this (Task 2), mirroring `Sink`'s own
/// history in this file.
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
    // Keys are `date\0repo_key\0model`, and redb's `&str` keys sort in
    // plain lexicographic byte order, so every row for `today` — any repo,
    // any model — falls in `"{today}\0" .. "{today}\u{1}"`: `\0` (0x00) is
    // the smallest possible byte, so "{today}\0" followed by *anything*
    // still sorts below "{today}\u{1}" (0x01) at that same position, and
    // `today` itself is a fixed-width `YYYY-MM-DD` string, so it is never a
    // byte-prefix of a different date. Bounding the scan this way touches
    // only today's rows instead of the table's entire un-pruned history, so
    // the date half of the old per-row filter below is no longer needed —
    // only the repo check remains.
    let range_start = format!("{today}\0");
    let range_end = format!("{today}\u{1}");
    let Ok(iter) = table.range(range_start.as_str()..range_end.as_str()) else {
        return None;
    };
    let mut total = 0.0_f64;
    let mut found = false;
    for entry in iter {
        let Ok((key, value)) = entry else { return None };
        let (_date, key_repo, _model) = decode_key(key.value())?;
        if key_repo != repo_key {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<Row>(value.value()) else {
            return None;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_record(date: &str, model: &str, cwd: &str) -> ParsedRecord {
        // `cost.rs`'s `ParsedRecord` fields are `pub`, but its
        // `timestamp_unix`/`dedup_key` are private to that module — this
        // helper needs a constructor. Add one in `src/cost.rs` alongside
        // `ParsedRecord`'s definition (Task 4's struct):
        //
        //     #[cfg(any(test, feature = "analytics"))]
        //     impl ParsedRecord {
        //         pub fn for_test(date: &str, model: &str, cwd: &str, usage: Usage) -> Self {
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
        sink.record(
            &usage_record("2026-08-10", "claude-sonnet-5", "/tmp/repo"),
            1.0,
        );
        sink.flush(dir.path());
        assert!(!db_path(dir.path()).exists());
    }

    #[test]
    fn record_outside_today_or_yesterday_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = Sink::new(true, "2026-08-10".to_string(), "2026-08-09".to_string());
        sink.record(
            &usage_record("2026-08-01", "claude-sonnet-5", "/tmp/repo"),
            1.0,
        );
        sink.flush(dir.path());
        assert!(
            !db_path(dir.path()).exists(),
            "nothing to flush means no file at all"
        );
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
        assert!(
            (row.cost_usd - 5.0).abs() < 1e-9,
            "two records in one pass accumulate"
        );
        assert_eq!(row.input_tokens, 2000);
    }

    #[test]
    fn a_yesterday_dated_record_is_written() {
        // Mirrors `enabled_sink_writes_a_readable_row`, but the record's
        // date matches the `yesterday` argument to `Sink::new` rather than
        // `today` — proving the bucketing accepts yesterday, not just today,
        // rather than only ever proving the negative case (a date that's
        // neither, per `record_outside_today_or_yesterday_is_dropped`).
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap(); // no .git — resolves to a `local:` identity
        let cwd = repo.path().to_str().unwrap();
        let mut sink = Sink::new(true, "2026-08-10".to_string(), "2026-08-09".to_string());
        sink.record(&usage_record("2026-08-09", "claude-sonnet-5", cwd), 4.0);
        sink.flush(dir.path());

        let db = redb::Database::open(db_path(dir.path())).unwrap();
        let txn = db.begin_read().unwrap();
        let table = txn.open_table(TABLE).unwrap();
        let expected_key = encode_key(
            "2026-08-09",
            &repo_identity::resolve(cwd).key,
            "claude-sonnet-5",
        );
        let value = table.get(expected_key.as_str()).unwrap().unwrap();
        let row: Row = serde_json::from_slice(value.value()).unwrap();
        assert!((row.cost_usd - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_second_flush_overwrites_rather_than_accumulates() {
        // The store's whole trustworthiness rests on this: each refresh
        // recomputes a day's full total from scratch and a second flush for
        // the same (date, repo, model) key must replace the first pass's
        // row, not add to it. Two separate `Sink`s (as two separate refresh
        // runs would produce) write different costs for the same key; only
        // the second pass's value must remain afterward.
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap(); // no .git — resolves to a `local:` identity
        let cwd = repo.path().to_str().unwrap();

        let mut first_sink = Sink::new(true, "2026-08-10".to_string(), "2026-08-09".to_string());
        first_sink.record(&usage_record("2026-08-10", "claude-sonnet-5", cwd), 10.0);
        first_sink.flush(dir.path());

        let mut second_sink = Sink::new(true, "2026-08-10".to_string(), "2026-08-09".to_string());
        second_sink.record(&usage_record("2026-08-10", "claude-sonnet-5", cwd), 1.0);
        second_sink.flush(dir.path());

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
        assert!(
            (row.cost_usd - 1.0).abs() < 1e-9,
            "second flush must overwrite the first pass's row (got {}), not sum \
             to 11.0 across the two separate flushes",
            row.cost_usd
        );
        assert_eq!(
            row.input_tokens, 1000,
            "not 2000 — not summed across flushes"
        );
    }

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

    // Guards the range-scan's bounds specifically: seeds rows for dates
    // that sort both lexicographically before ("2020-01-01") and after
    // ("9999-12-31") today, plus a second repo on today's own date, all in
    // the same store, and confirms `today_repo_cost` sums only the rows
    // that are actually today's date *and* the target repo. An off-by-one
    // in either bound of `"{today}\0" .. "{today}\u{1}"` would pull in one
    // of the out-of-range dates here; a bound that's too narrow would drop
    // today's own row.
    #[test]
    fn today_repo_cost_range_scan_excludes_other_dates_and_repos() {
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let other_repo = tempfile::tempdir().unwrap();
        let cwd = repo.path().to_str().unwrap();
        let other_cwd = other_repo.path().to_str().unwrap();
        let today = crate::cost::today_utc_date(crate::cost::now_unix_secs());
        let before_today = "2020-01-01".to_string(); // sorts before any real "today"
        let after_today = "9999-12-31".to_string(); // sorts after any real "today"

        // Sink only accepts records dated "today" or "yesterday" relative to
        // its own construction, so each out-of-range date needs its own
        // `Sink` constructed with that date as its "today" to get a real row
        // written for it.
        let mut sink_before = Sink::new(true, before_today.clone(), "1970-01-01".to_string());
        sink_before.record(&usage_record(&before_today, "claude-sonnet-5", cwd), 5.0);
        sink_before.flush(dir.path());

        let mut sink_after = Sink::new(true, after_today.clone(), "1970-01-01".to_string());
        sink_after.record(&usage_record(&after_today, "claude-sonnet-5", cwd), 7.0);
        sink_after.flush(dir.path());

        let mut sink_today = Sink::new(true, today.clone(), "1970-01-01".to_string());
        sink_today.record(&usage_record(&today, "claude-sonnet-5", cwd), 3.0);
        sink_today.record(&usage_record(&today, "claude-sonnet-5", other_cwd), 100.0);
        sink_today.flush(dir.path());

        let repo_key = repo_identity::resolve(cwd).key;
        let total = today_repo_cost(true, dir.path(), &repo_key);
        assert!(
            (total.unwrap() - 3.0).abs() < 1e-9,
            "expected only today's $3.00 row for this repo, got {total:?}"
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

    #[test]
    fn today_repo_cost_returns_none_when_a_matching_row_is_malformed() {
        // One valid row plus one malformed row, both for today's date and
        // the same repo: a partial total from just the valid row would be
        // as misleading as it is silent, so this must be `None`, not a sum
        // that quietly drops the bad row.
        let dir = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let cwd = repo.path().to_str().unwrap();
        let today = crate::cost::today_utc_date(crate::cost::now_unix_secs());
        let repo_key = repo_identity::resolve(cwd).key;

        let mut sink = Sink::new(true, today.clone(), "1970-01-01".to_string());
        sink.record(&usage_record(&today, "claude-sonnet-5", cwd), 2.0);
        sink.flush(dir.path());

        // `Sink`/`flush` only ever write valid `Row` JSON, so the malformed
        // row has to be inserted directly, bypassing that path.
        let db = redb::Database::create(db_path(dir.path())).unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut table = txn.open_table(TABLE).unwrap();
            let key = encode_key(&today, &repo_key, "claude-opus-5");
            table.insert(key.as_str(), b"not json".as_slice()).unwrap();
        }
        txn.commit().unwrap();

        assert_eq!(today_repo_cost(true, dir.path(), &repo_key), None);
    }
}
