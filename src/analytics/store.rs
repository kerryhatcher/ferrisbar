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

// `TABLE` is exercised by this module's own tests and, from Task 7 on, by
// `report`'s reader — but nothing in the non-test build calls into `Sink`
// yet (that's Task 6), so it reads as dead code until then.
#[allow(dead_code)]
pub const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("cost_rows");

// Same story as `TABLE` above: constructed only by `Sink::record` (tested,
// not yet called from non-test code) until Task 6 lands.
#[allow(dead_code)]
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
    // Only called from `Sink::record`, which is itself unreachable from
    // `main` until Task 6.
    #[allow(dead_code)]
    fn accumulate(&mut self, rec: &ParsedRecord, cost: f64) {
        self.cost_usd += cost;
        self.input_tokens += rec.usage.input_tokens;
        self.output_tokens += rec.usage.output_tokens;
        self.cache_creation_tokens += rec.usage.cache_creation_tokens;
        self.cache_read_tokens += rec.usage.cache_read_tokens;
    }
}

// Public for Task 7's report reader; used by `Sink::flush` and this
// module's tests today, but `flush` itself isn't reachable from `main`
// until Task 6.
#[allow(dead_code)]
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("analytics.redb")
}

// Reachable from `Sink::record` (unreachable until Task 6) and from this
// module's own tests directly.
#[allow(dead_code)]
fn encode_key(date: &str, repo_key: &str, model: &str) -> String {
    format!("{date}\0{repo_key}\0{model}")
}

/// Inverse of `encode_key`. `None` for a key that doesn't split into
/// exactly three `\0`-separated parts — defensive against a store written
/// by some future, differently-shaped version of this code.
// Public for Task 7's report reader; not yet called from non-test code.
#[allow(dead_code)]
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

// `Sink` and its methods below are exercised only by this module's tests
// until Task 6 wires a call into `cost.rs`'s transcript-walk hot loop —
// until then, a `--features analytics` build with no test harness sees no
// caller and flags the whole type as dead.
#[allow(dead_code)]
pub struct Sink {
    enabled: bool,
    today: String,
    yesterday: String,
    rows: HashMap<String, Row>,
    repo_cache: HashMap<String, RepoIdentity>,
}

#[allow(dead_code)]
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
}
