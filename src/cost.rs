//! Cost estimation: a per-model pricing table plus a same-day, all-sessions
//! transcript aggregate for the "$X today (Model $Y · Model $Z)" chip.
//!
//! Pricing is embedded (`pricing.json`) rather than fetched or configured —
//! it is a small, rarely-changing table and a network call or extra config
//! surface is not worth it for an estimate that is explicitly not a billing
//! source of record.
//!
//! The daily aggregate reads every transcript under `<claude config
//! dir>/projects/**/*.jsonl`, which is too much I/O to redo on every render
//! (ferrisbar's whole pitch is not starting an interpreter per prompt). It is
//! therefore only ever computed by the hidden `--internal-refresh-daily-cost`
//! re-invocation spawned off the render path by `cost_cache::spawn_refresh`,
//! and read from `cost_cache`'s small on-disk cache everywhere else.

use crate::layout::{DIM, GREEN, RESET};
use crate::{config::CostConfig, cost_cache};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PRICING_JSON: &str = include_str!("pricing.json");

/// How many consecutive unreadable/non-UTF-8 lines `aggregate_today` will
/// skip in one transcript before giving up on that file — see the comment
/// at its call site.
const MAX_CONSECUTIVE_LINE_ERRORS: u32 = 1000;

#[derive(Deserialize, Clone, Copy, Default)]
struct ModelRate {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_write: f64,
    #[serde(default)]
    cache_read: f64,
}

#[derive(Deserialize, Default)]
struct PricingTable {
    #[serde(default)]
    models: HashMap<String, ModelRate>,
    #[serde(default)]
    aliases: HashMap<String, String>,
}

/// The embedded table is checked in and covered by `pricing_json_parses`
/// below, so a parse failure here would mean the asset itself is broken —
/// still handled without panicking, per the never-panic invariant.
fn load_pricing() -> PricingTable {
    serde_json::from_str(PRICING_JSON).unwrap_or_default()
}

/// Exact match, then the longest pricing-table key that is a prefix of
/// `model` (so a dated/suffixed id still resolves to its family), then an
/// alias substring match. `None` for a genuinely unrecognized model, rather
/// than guessing a rate.
fn resolve_model<'a>(model: &str, pricing: &'a PricingTable) -> Option<&'a ModelRate> {
    if model.is_empty() {
        return None;
    }
    if let Some(rate) = pricing.models.get(model) {
        return Some(rate);
    }
    let mut best: Option<&str> = None;
    for key in pricing.models.keys() {
        if model.starts_with(key.as_str()) && best.is_none_or(|b| key.len() > b.len()) {
            best = Some(key);
        }
    }
    if let Some(key) = best {
        return pricing.models.get(key);
    }
    let lowered = model.to_lowercase();
    // `max_by_key` over the longest matching alias, not `find`: `aliases` is
    // a HashMap, so iteration order is unspecified, and a model id matching
    // more than one alias substring would otherwise resolve to an arbitrary
    // one (varying across runs, not just across models).
    pricing
        .aliases
        .iter()
        .filter(|(alias, _)| lowered.contains(alias.as_str()))
        .max_by_key(|(alias, _)| alias.len())
        .and_then(|(_, target)| pricing.models.get(target))
}

// The shared `_tokens` postfix names the unit, not redundant boilerplate —
// dropping it would leave `input`/`output` reading like dollar amounts
// elsewhere in this module.
#[allow(clippy::struct_field_names)]
#[derive(Default, Clone, Copy)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
}

impl Usage {
    const fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_creation_tokens == 0
            && self.cache_read_tokens == 0
    }
}

/// Token counts stay well below 2^53, so the f64 conversion loses no
/// precision worth caring about — the same tradeoff `config.rs` makes for
/// TOML integers.
#[allow(clippy::cast_precision_loss)]
fn cost_for(usage: &Usage, model: &str, pricing: &PricingTable) -> f64 {
    let Some(rate) = resolve_model(model, pricing) else {
        return 0.0;
    };
    let per_mtok = 1_000_000.0_f64;
    (usage.cache_read_tokens as f64 / per_mtok).mul_add(
        rate.cache_read,
        (usage.cache_creation_tokens as f64 / per_mtok).mul_add(
            rate.cache_write,
            (usage.output_tokens as f64 / per_mtok).mul_add(
                rate.output,
                usage.input_tokens as f64 / per_mtok * rate.input,
            ),
        ),
    )
}

/// Short display label; collapses model versions so e.g. opus-4-8 and
/// opus-4-7 both read as "Opus".
fn model_label(raw: &str) -> String {
    let lower = raw.to_lowercase();
    for (needle, label) in [
        ("opus", "Opus"),
        ("sonnet", "Sonnet"),
        ("haiku", "Haiku"),
        ("fable", "Fable"),
        ("mythos", "Mythos"),
    ] {
        if lower.contains(needle) {
            return label.to_string();
        }
    }
    let trimmed = raw.strip_prefix("claude-").unwrap_or(raw);
    trimmed.chars().take(8).collect()
}

// --- Transcript parsing -------------------------------------------------

// Field names mirror the transcript's own JSON keys exactly (see the module
// docs), so the shared `_tokens` postfix stays rather than getting trimmed.
#[allow(clippy::struct_field_names)]
#[derive(Deserialize)]
struct UsageRaw {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

#[derive(Deserialize)]
struct MessageRaw {
    model: Option<String>,
    id: Option<String>,
    usage: Option<UsageRaw>,
}

#[derive(Deserialize)]
struct RecordRaw {
    message: Option<MessageRaw>,
    timestamp: Option<String>,
    #[serde(rename = "requestId", alias = "request_id")]
    request_id: Option<String>,
}

struct ParsedRecord {
    usage: Usage,
    model: String,
    date: String,
    dedup_key: Option<String>,
}

/// `None` for anything that is not a usage-bearing assistant message —
/// absent/malformed JSON, no usage block, an all-zero usage block, or a
/// timestamp too short to hold a date. Transcript shapes drift across
/// Claude Code versions, so this reads defensively rather than raising.
fn parse_line(line: &str) -> Option<ParsedRecord> {
    if !line.contains("\"usage\"") {
        return None; // cheap prefilter before paying for a JSON parse
    }
    let record: RecordRaw = serde_json::from_str(line).ok()?;
    let message = record.message?;
    let usage_raw = message.usage?;
    let usage = Usage {
        input_tokens: usage_raw.input_tokens,
        output_tokens: usage_raw.output_tokens,
        cache_creation_tokens: usage_raw.cache_creation_input_tokens,
        cache_read_tokens: usage_raw.cache_read_input_tokens,
    };
    if usage.is_empty() {
        return None;
    }
    let timestamp = record.timestamp?;
    // `.get(..10)` rather than `timestamp[..10]`: a byte-length check alone
    // does not guarantee byte 10 falls on a UTF-8 char boundary, and slicing
    // on a non-boundary panics — a real transcript timestamp is always
    // plain ASCII, but the never-panic-on-input invariant covers malformed
    // ones too.
    let date = timestamp.get(..10)?.to_string();
    let dedup_key = match (message.id, record.request_id) {
        (Some(mid), Some(rid)) if !mid.is_empty() && !rid.is_empty() => {
            Some(format!("{mid}:{rid}"))
        }
        _ => None,
    };
    Some(ParsedRecord {
        usage,
        model: message.model.unwrap_or_default(),
        date,
        dedup_key,
    })
}

/// Every `*.jsonl` under `root`, recursively. Manual `read_dir` recursion
/// rather than a `walkdir` dependency — the tree is at most a few hundred
/// entries deep and this is only ever run off the render path (see the
/// module docs).
fn discover_transcripts(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // `entry.file_type()` does not follow symlinks (unlike
            // `path.is_dir()`, which does): a directory symlink pointing at
            // an ancestor would otherwise put that ancestor back on the
            // stack forever. Never following one is simpler than tracking
            // visited paths, and legitimate transcripts are never symlinks.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    out
}

// --- Calendar day (UTC, dependency-free) --------------------------------

/// Days-since-epoch to a proleptic Gregorian (year, month, day) — Howard
/// Hinnant's public-domain `civil_from_days` algorithm
/// (<http://howardhinnant.github.io/date_algorithms.html>). Written out by
/// hand instead of pulling in a date/timezone crate: this is the one piece
/// of calendar math ferrisbar needs (a UTC day boundary), so it is not worth
/// the runtime-dependency budget `CLAUDE.md` deliberately keeps at four.
const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    // Safe: `doe` is `z - era * 146_097`, which the algorithm guarantees
    // lands in [0, 146_096] regardless of era's sign.
    #[allow(clippy::cast_sign_loss)]
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    // Safe: yoe is bounded to [0, 399] by the algorithm.
    #[allow(clippy::cast_possible_wrap)]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    // Safe: doy/mp are bounded ([0,365] and [0,11]) by the algorithm, well
    // within u32.
    #[allow(clippy::cast_possible_truncation)]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn today_utc_date(now_unix_secs: i64) -> String {
    let (y, m, d) = civil_from_days(now_unix_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn now_unix_secs() -> i64 {
    // Safe: `as_secs` cannot exceed i64::MAX for any date this program will
    // ever run on.
    #[allow(clippy::cast_possible_wrap)]
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

// --- Aggregation ---------------------------------------------------------

pub struct DailyTotal {
    pub total_usd: f64,
    /// Raw model id -> estimated USD, unsorted, cost > 0.0 only.
    pub by_model: Vec<(String, f64)>,
}

fn aggregate_today(transcripts_root: &Path, today: &str) -> DailyTotal {
    let pricing = load_pricing();
    let mut total = 0.0_f64;
    let mut by_model: HashMap<String, f64> = HashMap::new();
    let mut seen = std::collections::HashSet::new();

    for path in discover_transcripts(transcripts_root) {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        // Not `.lines().map_while(Result::ok)` (stops the whole file at the
        // first malformed/non-UTF-8 line, silently undercounting everything
        // after it) and not `.filter_map(Result::ok)` either — clippy's
        // `lines_filter_map_ok` is right that a persistent I/O error can
        // make `next()` return the same `Err` forever without advancing,
        // spinning this loop forever. Skipping errors is still correct for
        // the common case, so this bounds how many *consecutive* errors it
        // tolerates before giving up on the file, rather than stopping at
        // the first one or never stopping at all.
        let mut lines = std::io::BufReader::new(file).lines();
        let mut consecutive_errors = 0u32;
        loop {
            let line = match lines.next() {
                None => break,
                Some(Ok(line)) => {
                    consecutive_errors = 0;
                    line
                }
                Some(Err(_)) => {
                    consecutive_errors += 1;
                    if consecutive_errors > MAX_CONSECUTIVE_LINE_ERRORS {
                        break;
                    }
                    continue;
                }
            };
            let Some(rec) = parse_line(&line) else {
                continue;
            };
            if rec.date != today {
                continue;
            }
            if let Some(key) = rec.dedup_key {
                if !seen.insert(key) {
                    continue; // already counted from another transcript
                }
            }
            let cost = cost_for(&rec.usage, &rec.model, &pricing);
            if cost > 0.0 {
                total += cost;
                *by_model.entry(rec.model).or_insert(0.0) += cost;
            }
        }
    }

    DailyTotal {
        total_usd: total,
        by_model: by_model.into_iter().collect(),
    }
}

/// Collapses `daily.by_model`'s raw ids to short labels, drops entries
/// below `min_usd`, and renders the "$X.XX today (Label $Y · Label $Z)"
/// chip. Returns just the "$X.XX today" part when the breakdown is empty
/// (every model folded under `min_usd`, or the day had no cost at all).
fn format_daily_chip(daily: &DailyTotal, min_usd: f64) -> String {
    let mut collapsed: HashMap<String, f64> = HashMap::new();
    for (model, cost) in &daily.by_model {
        if *cost < min_usd {
            continue;
        }
        *collapsed.entry(model_label(model)).or_insert(0.0) += cost;
    }
    let mut collapsed: Vec<(String, f64)> = collapsed.into_iter().collect();
    collapsed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut chip = format!("{GREEN}${:.2}{RESET}{DIM} today{RESET}", daily.total_usd);
    if !collapsed.is_empty() {
        let inner = collapsed
            .iter()
            .map(|(label, cost)| {
                // Whole dollars once a model has spent that much (keeps the
                // parenthetical compact), but a sub-dollar spend keeps its
                // cents — rounding e.g. $0.04 down to "$0" reads as free.
                if *cost < 1.0 {
                    format!("{label} ${cost:.2}")
                } else {
                    format!("{label} ${cost:.0}")
                }
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let _ = write!(chip, " {DIM}({inner}){RESET}");
    }
    chip
}

/// Entry point for the hidden `--internal-refresh-daily-cost` re-invocation:
/// recompute today's aggregate and rewrite the cache. Infallible by
/// construction — a write failure is swallowed by `cost_cache::write_cache`,
/// and the lock is always released so a stuck refresh cannot wedge every
/// later render out of ever retrying.
pub fn refresh_daily_cache(transcripts_root: &Path, data_dir: &Path) {
    let today = today_utc_date(now_unix_secs());
    let daily = aggregate_today(transcripts_root, &today);
    let payload = cost_cache::CachePayload {
        date: today,
        total_usd: daily.total_usd,
        by_model: daily.by_model,
    };
    let _ = cost_cache::write_cache(data_dir, &payload);
    cost_cache::release_lock(data_dir);
}

/// The daily cost chip for the statusline's second line, or `None` when the
/// feature is disabled, no data directory is available, or no cache has
/// been populated yet (the first-ever render after install has nothing to
/// show until the background refresh it triggers completes).
///
/// Never blocks: a stale or missing cache triggers `cost_cache::spawn_refresh`
/// (a detached, non-blocking re-invocation of this binary) and this render
/// still uses whatever cache is currently on disk, even if stale.
pub fn daily_chip(cfg: &CostConfig, data_dir: Option<&Path>) -> Option<String> {
    if !cfg.show_daily || cfg.ttl_seconds == 0 {
        return None;
    }
    let data_dir = data_dir?;
    let today = today_utc_date(now_unix_secs());
    let cached = cost_cache::read_cache(data_dir);
    // A cache dated to a previous day is stale regardless of its age: right
    // after the UTC day boundary it can still be well under `ttl_seconds`
    // old, and without this check the chip would stay hidden until the TTL
    // naturally elapses instead of refreshing right away.
    let stale = cached
        .as_ref()
        .is_none_or(|(payload, age)| age.as_secs() >= cfg.ttl_seconds || payload.date != today);
    if stale {
        cost_cache::spawn_refresh(data_dir);
    }
    let (payload, _age) = cached?;
    if payload.date != today {
        return None; // cache is from a previous day; wait for the refresh
    }
    let daily = DailyTotal {
        total_usd: payload.total_usd,
        by_model: payload.by_model,
    };
    Some(format_daily_chip(&daily, cfg.breakdown_min_usd))
}

#[cfg(test)]
// These assertions check exact literal fallback/zero constants, not
// accumulated float arithmetic, so exact equality is the correct check —
// same rationale as `payload.rs`'s test module.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn pricing_json_parses() {
        let pricing = load_pricing();
        assert!(!pricing.models.is_empty());
        assert!(!pricing.aliases.is_empty());
    }

    #[test]
    fn resolve_model_exact_match() {
        let pricing = load_pricing();
        assert!(resolve_model("claude-sonnet-5", &pricing).is_some());
    }

    #[test]
    fn resolve_model_longest_prefix_wins() {
        let pricing = load_pricing();
        let rate = resolve_model("claude-opus-4-8-preview", &pricing).unwrap();
        let exact = pricing.models.get("claude-opus-4-8").unwrap();
        assert!((rate.input - exact.input).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_model_alias_fallback() {
        let pricing = load_pricing();
        assert!(resolve_model("claude-opus-5", &pricing).is_some());
    }

    #[test]
    fn resolve_model_alias_picks_the_longest_deterministically() {
        let mut pricing = PricingTable::default();
        pricing.models.insert(
            "short-target".to_string(),
            ModelRate {
                input: 1.0,
                ..ModelRate::default()
            },
        );
        pricing.models.insert(
            "long-target".to_string(),
            ModelRate {
                input: 2.0,
                ..ModelRate::default()
            },
        );
        pricing
            .aliases
            .insert("op".to_string(), "short-target".to_string());
        pricing
            .aliases
            .insert("opus".to_string(), "long-target".to_string());

        // Both "op" and "opus" match; the longer alias must win regardless
        // of HashMap iteration order, or this test would be flaky.
        let rate = resolve_model("claude-opus-9", &pricing).unwrap();
        assert!((rate.input - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_model_unknown_is_none() {
        let pricing = load_pricing();
        assert!(resolve_model("some-other-vendor-model", &pricing).is_none());
        assert!(resolve_model("", &pricing).is_none());
    }

    #[test]
    fn cost_for_unknown_model_is_zero() {
        let pricing = load_pricing();
        let usage = Usage {
            input_tokens: 1_000_000,
            ..Usage::default()
        };
        assert_eq!(cost_for(&usage, "unknown", &pricing), 0.0);
    }

    #[test]
    fn cost_for_known_model_prices_each_token_class() {
        let pricing = load_pricing();
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_creation_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
        };
        // claude-sonnet-5: input 2.0, output 10.0, cache_write 2.5, cache_read 0.2
        let cost = cost_for(&usage, "claude-sonnet-5", &pricing);
        assert!((cost - 14.7).abs() < 1e-9);
    }

    #[test]
    fn model_label_collapses_variants() {
        assert_eq!(model_label("claude-opus-4-8"), "Opus");
        assert_eq!(model_label("claude-opus-4-1"), "Opus");
        assert_eq!(model_label("claude-sonnet-5"), "Sonnet");
        assert_eq!(model_label("claude-haiku-4-5-20251001"), "Haiku");
    }

    #[test]
    fn model_label_unknown_falls_back_to_trimmed_id() {
        assert_eq!(model_label("claude-zeta-9"), "zeta-9");
        assert_eq!(model_label(""), "");
    }

    #[test]
    fn parse_line_extracts_usage_model_and_date() {
        let line = r#"{"timestamp":"2026-08-10T16:56:48.920Z","requestId":"req_1","message":{"model":"claude-sonnet-5","id":"msg_1","usage":{"input_tokens":2,"output_tokens":384,"cache_creation_input_tokens":17274,"cache_read_input_tokens":30156}}}"#;
        let rec = parse_line(line).unwrap();
        assert_eq!(rec.date, "2026-08-10");
        assert_eq!(rec.model, "claude-sonnet-5");
        assert_eq!(rec.usage.output_tokens, 384);
        assert_eq!(rec.dedup_key, Some("msg_1:req_1".to_string()));
    }

    #[test]
    fn parse_line_rejects_non_usage_lines() {
        assert!(parse_line(r#"{"type":"user","message":{"role":"user"}}"#).is_none());
        assert!(parse_line("not json at all").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn parse_line_rejects_all_zero_usage() {
        let line = r#"{"timestamp":"2026-08-10T00:00:00Z","message":{"model":"x","usage":{"input_tokens":0,"output_tokens":0}}}"#;
        assert!(parse_line(line).is_none());
    }

    #[test]
    fn parse_line_missing_dedup_fields_yields_no_key() {
        let line = r#"{"timestamp":"2026-08-10T00:00:00Z","message":{"model":"x","usage":{"input_tokens":5}}}"#;
        let rec = parse_line(line).unwrap();
        assert_eq!(rec.dedup_key, None);
    }

    #[test]
    fn parse_line_does_not_panic_on_a_non_char_boundary_timestamp() {
        // "日" is 3 bytes; byte offset 10 lands inside the third one, so a
        // plain `timestamp[..10]` slice would panic. Must degrade to `None`
        // instead, per the never-panic-on-input invariant.
        let line = r#"{"timestamp":"日本語日本語日本語","message":{"model":"x","usage":{"input_tokens":5}}}"#;
        assert!(parse_line(line).is_none());
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(20_675), (2026, 8, 10));
    }

    #[test]
    fn today_utc_date_formats_with_zero_padding() {
        // 2022-01-08, 19000 days after epoch, at noon UTC.
        let unix = 19_000 * 86_400 + 12 * 3600;
        assert_eq!(today_utc_date(unix), "2022-01-08");
    }

    fn write_transcript(dir: &Path, name: &str, lines: &[&str]) {
        std::fs::write(dir.join(name), lines.join("\n")).unwrap();
    }

    fn usage_line(
        timestamp: &str,
        model: &str,
        request_id: &str,
        msg_id: &str,
        tokens: u64,
    ) -> String {
        format!(
            r#"{{"timestamp":"{timestamp}","requestId":"{request_id}","message":{{"model":"{model}","id":"{msg_id}","usage":{{"input_tokens":{tokens}}}}}}}"#
        )
    }

    #[test]
    fn aggregate_today_sums_matching_dates_across_files_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("proj");
        std::fs::create_dir_all(&sub).unwrap();

        write_transcript(
            &sub,
            "a.jsonl",
            &[
                &usage_line(
                    "2026-08-10T10:00:00Z",
                    "claude-sonnet-5",
                    "req_1",
                    "msg_1",
                    1_000_000,
                ),
                &usage_line(
                    "2026-08-09T10:00:00Z",
                    "claude-sonnet-5",
                    "req_2",
                    "msg_2",
                    1_000_000,
                ),
            ],
        );
        write_transcript(
            &dir.path().join("proj"),
            "b.jsonl",
            &[
                // Same message resumed into a second transcript — must not double-count.
                &usage_line(
                    "2026-08-10T10:00:00Z",
                    "claude-sonnet-5",
                    "req_1",
                    "msg_1",
                    1_000_000,
                ),
                &usage_line(
                    "2026-08-10T11:00:00Z",
                    "claude-opus-4-8",
                    "req_3",
                    "msg_3",
                    1_000_000,
                ),
            ],
        );

        let daily = aggregate_today(dir.path(), "2026-08-10");

        // sonnet-5 input rate is $2/Mtok, so one deduped 1M-token message = $2.
        // opus-4-8 input rate is $5/Mtok, so one 1M-token message = $5.
        assert!((daily.total_usd - 7.0).abs() < 1e-9);
        assert_eq!(daily.by_model.len(), 2);
    }

    #[test]
    fn aggregate_today_keeps_reading_after_a_malformed_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        let mut bytes = vec![0xFF_u8, 0xFE, b'\n']; // not valid UTF-8
        bytes.extend_from_slice(
            usage_line(
                "2026-08-10T10:00:00Z",
                "claude-sonnet-5",
                "req_1",
                "msg_1",
                1_000_000,
            )
            .as_bytes(),
        );
        std::fs::write(&path, bytes).unwrap();

        let daily = aggregate_today(dir.path(), "2026-08-10");
        assert!(
            (daily.total_usd - 2.0).abs() < 1e-9,
            "the valid line after the malformed one must still be counted"
        );
    }

    #[cfg(unix)]
    #[test]
    fn discover_transcripts_does_not_follow_a_directory_symlink_loop() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("a.jsonl"), "").unwrap();
        // If followed, this would put `dir` back on the traversal stack
        // from inside `sub`, forever.
        std::os::unix::fs::symlink(dir.path(), sub.join("loop")).unwrap();

        let found = discover_transcripts(dir.path());
        assert_eq!(found, vec![sub.join("a.jsonl")]);
    }

    #[test]
    fn aggregate_today_ignores_unreadable_root() {
        let daily = aggregate_today(Path::new("/does/not/exist"), "2026-08-10");
        assert_eq!(daily.total_usd, 0.0);
        assert!(daily.by_model.is_empty());
    }

    #[test]
    fn format_daily_chip_folds_small_models_but_keeps_total() {
        let daily = DailyTotal {
            total_usd: 10.005,
            by_model: vec![
                ("claude-sonnet-5".to_string(), 10.0),
                ("claude-haiku-4-5".to_string(), 0.005),
            ],
        };
        let chip = format_daily_chip(&daily, 0.01);
        assert!(chip.contains("$10.01") || chip.contains("$10.00"));
        assert!(chip.contains("Sonnet"));
        assert!(!chip.contains("Haiku"));
    }

    #[test]
    fn format_daily_chip_keeps_cents_for_a_sub_dollar_model() {
        let daily = DailyTotal {
            total_usd: 10.04,
            by_model: vec![
                ("claude-sonnet-5".to_string(), 10.0),
                ("claude-haiku-4-5".to_string(), 0.04),
            ],
        };
        // Above breakdown_min_usd but below $1 — rounding to `.0` would
        // read as "Haiku $0", i.e. free.
        let chip = format_daily_chip(&daily, 0.005);
        assert!(chip.contains("Haiku $0.04"));
        assert!(chip.contains("Sonnet $10"));
    }

    #[test]
    fn format_daily_chip_with_empty_breakdown_shows_just_the_total() {
        let daily = DailyTotal {
            total_usd: 0.0,
            by_model: Vec::new(),
        };
        let chip = format_daily_chip(&daily, 0.005);
        assert!(chip.contains("$0.00"));
        assert!(!chip.contains('('));
    }

    #[test]
    fn daily_chip_disabled_returns_none() {
        let cfg = CostConfig {
            show_daily: false,
            ..CostConfig::default()
        };
        assert!(daily_chip(&cfg, Some(Path::new("/tmp"))).is_none());
    }

    #[test]
    fn daily_chip_zero_ttl_returns_none() {
        let cfg = CostConfig {
            ttl_seconds: 0,
            ..CostConfig::default()
        };
        assert!(daily_chip(&cfg, Some(Path::new("/tmp"))).is_none());
    }

    #[test]
    fn daily_chip_no_data_dir_returns_none() {
        assert!(daily_chip(&CostConfig::default(), None).is_none());
    }

    #[test]
    fn daily_chip_no_cache_yet_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(daily_chip(&CostConfig::default(), Some(dir.path())).is_none());
    }

    #[test]
    fn daily_chip_reads_a_fresh_same_day_cache() {
        let dir = tempfile::tempdir().unwrap();
        let payload = cost_cache::CachePayload {
            date: today_utc_date(now_unix_secs()),
            total_usd: 4.2,
            by_model: vec![("claude-sonnet-5".to_string(), 4.2)],
        };
        cost_cache::write_cache(dir.path(), &payload).unwrap();

        let chip = daily_chip(&CostConfig::default(), Some(dir.path())).unwrap();
        assert!(chip.contains("$4.20"));
        assert!(chip.contains("Sonnet"));
    }

    #[test]
    fn daily_chip_ignores_a_stale_dated_cache() {
        let dir = tempfile::tempdir().unwrap();
        let payload = cost_cache::CachePayload {
            date: "2000-01-01".to_string(),
            total_usd: 4.2,
            by_model: Vec::new(),
        };
        cost_cache::write_cache(dir.path(), &payload).unwrap();

        assert!(daily_chip(&CostConfig::default(), Some(dir.path())).is_none());
    }

    #[test]
    fn daily_chip_refreshes_a_wrong_dated_cache_even_when_ttl_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let payload = cost_cache::CachePayload {
            date: "2000-01-01".to_string(), // wrong day, but just written: well under any TTL
            total_usd: 4.2,
            by_model: Vec::new(),
        };
        cost_cache::write_cache(dir.path(), &payload).unwrap();

        daily_chip(&CostConfig::default(), Some(dir.path()));

        // A refresh must fire right away rather than waiting out the TTL —
        // `spawn_refresh` leaves a lock file behind as it starts one.
        assert!(dir.path().join("cost-cache.lock").exists());
    }

    #[test]
    fn refresh_daily_cache_writes_a_cache_and_releases_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("cost-cache.lock"), b"").unwrap();

        refresh_daily_cache(&root, dir.path());

        assert!(dir.path().join("cost-cache.json").exists());
        assert!(!dir.path().join("cost-cache.lock").exists());
    }
}
