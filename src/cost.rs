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

use crate::context_bar;
use crate::layout::{BLINK_RED, DIM, GREEN, ORANGE, RESET, YELLOW};
use crate::{
    config::{BudgetConfig, CostConfig, DisplayConfig},
    cost_cache,
};
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
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
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
    #[serde(default)]
    cwd: Option<String>,
}

#[allow(dead_code)]
// ParsedRecord is pub and fields are pub because Task 5's Sink module will consume them.
pub struct ParsedRecord {
    pub usage: Usage,
    pub model: String,
    pub date: String,
    /// `None` when `timestamp` doesn't parse as the expected
    /// `YYYY-MM-DDTHH:MM:SS...` shape — such a record still counts toward
    /// `date`-keyed daily aggregation but is excluded from the rolling/
    /// calendar budget windows, which need a real instant to compare against.
    pub timestamp_unix: Option<i64>,
    pub dedup_key: Option<String>,
    pub cwd: Option<String>,
}

#[cfg(any(test, feature = "analytics"))]
impl ParsedRecord {
    // Only called from tests today. It also compiles into a plain
    // `--features analytics` build (no `cfg(test)`) for `analytics::store`'s
    // own tests to use, where it's unused-but-harmless until Task 6 gives
    // `analytics::Sink` a real caller in this module's hot loop.
    #[allow(dead_code)]
    pub fn for_test(date: &str, model: &str, cwd: &str, usage: Usage) -> Self {
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
        timestamp_unix: parse_iso8601_utc(&timestamp),
        date,
        dedup_key,
        cwd: record.cwd,
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

/// Inverse of `civil_from_days` — a proleptic Gregorian (year, month, day)
/// to days-since-epoch, the other half of Howard Hinnant's algorithm. Needed
/// to turn "the 1st of this month" back into a day count for the monthly
/// budget window's start boundary.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
const fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let mp = (if m > 2 { m - 3 } else { m + 9 }) as u64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as u64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe as i64 - 719_468
}

/// Parses the `YYYY-MM-DDTHH:MM:SS` prefix of a transcript timestamp (the
/// fractional-second and `Z` suffix, if present, are ignored) into Unix
/// seconds. `None` for anything that doesn't match — transcript timestamps
/// are always UTC, so there is no offset to handle, but a record with a
/// different shape must degrade to "not counted" rather than panic.
fn parse_iso8601_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let minute: i64 = s.get(14..16)?.parse().ok()?;
    let second: i64 = s.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        // An out-of-range time component (e.g. hour 99) would otherwise
        // still produce *a* Unix timestamp — just a wrong one, silently
        // shifting the record into the wrong budget window rather than
        // excluding it the way a genuinely malformed record should be.
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Start of the Monday-anchored ISO week containing `now`. Epoch day 0
/// (1970-01-01) was a Thursday, so `(day + 3).rem_euclid(7)` gives days
/// since the preceding Monday (Monday = 0 ... Sunday = 6).
const fn start_of_week(now: i64) -> i64 {
    let day = now.div_euclid(86_400);
    let since_monday = (day + 3).rem_euclid(7);
    (day - since_monday) * 86_400
}

/// Start of the calendar month containing `now`, in UTC.
const fn start_of_month(now: i64) -> i64 {
    let (y, m, _) = civil_from_days(now.div_euclid(86_400));
    days_from_civil(y, m, 1) * 86_400
}

/// Start of the rolling 5-hour rate-limit block ending at `now` — unlike the
/// calendar windows above, this one always slides rather than resetting on
/// a boundary.
const fn start_of_block5h(now: i64) -> i64 {
    now - 5 * 3600
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

/// The daily total plus the three longer/shorter budget windows (Decision
/// 0004/0007 in the design this mirrors: session, daily, weekly/monthly,
/// and the rolling 5-hour rate-limit block), all from one transcript pass.
pub struct WindowTotals {
    pub daily: DailyTotal,
    pub weekly_usd: f64,
    pub monthly_usd: f64,
    pub block5h_usd: f64,
}

/// One pass over every transcript line, bucketing each usage-bearing
/// record's cost into whichever windows its timestamp falls in. `today`
/// still keys the daily total by date string (matching the on-disk cache's
/// existing staleness check); the other three windows compare directly
/// against `now`-derived boundaries.
fn aggregate_windows(
    transcripts_root: &Path,
    now: i64,
    today: &str,
    analytics: &mut crate::analytics::Sink,
) -> WindowTotals {
    let pricing = load_pricing();
    let week_start = start_of_week(now);
    let month_start = start_of_month(now);
    let block_start = start_of_block5h(now);

    let mut daily_total = 0.0_f64;
    let mut daily_by_model: HashMap<String, f64> = HashMap::new();
    let mut weekly_total = 0.0_f64;
    let mut monthly_total = 0.0_f64;
    let mut block5h_total = 0.0_f64;
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
            if let Some(key) = &rec.dedup_key {
                if !seen.insert(key.clone()) {
                    continue; // already counted from another transcript
                }
            }
            let cost = cost_for(&rec.usage, &rec.model, &pricing);
            if cost <= 0.0 {
                continue;
            }
            analytics.record(&rec, cost);
            if rec.date == today {
                daily_total += cost;
                *daily_by_model.entry(rec.model.clone()).or_insert(0.0) += cost;
            }
            let Some(ts) = rec.timestamp_unix else {
                continue; // unparsable timestamp: daily still counted above, rolling windows can't be
            };
            if ts > now {
                continue; // clock skew or a test fixture dated after `now`; never count future usage
            }
            if ts >= week_start {
                weekly_total += cost;
            }
            if ts >= month_start {
                monthly_total += cost;
            }
            if ts >= block_start {
                block5h_total += cost;
            }
        }
    }

    WindowTotals {
        daily: DailyTotal {
            total_usd: daily_total,
            by_model: daily_by_model.into_iter().collect(),
        },
        weekly_usd: weekly_total,
        monthly_usd: monthly_total,
        block5h_usd: block5h_total,
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
pub fn refresh_daily_cache(transcripts_root: &Path, data_dir: &Path, analytics_enabled: bool) {
    let now = now_unix_secs();
    let today = today_utc_date(now);
    let yesterday = today_utc_date(now - 86_400);
    let mut analytics = crate::analytics::Sink::new(analytics_enabled, today.clone(), yesterday);
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

/// Reads the on-disk cache, honoring `ttl_seconds` for staleness (which
/// triggers a background `cost_cache::spawn_refresh`, never a blocking
/// recompute), and discards it if it's dated to a previous day — the day
/// boundary is also the week and month boundary, so this one check covers
/// invalidating the daily, weekly, and monthly windows alike. `None` covers
/// every reason there's nothing usable yet: disabled TTL, no cache written,
/// or a stale previous-day cache still awaiting its refresh.
fn fresh_same_day_cache(ttl_seconds: u64, data_dir: &Path) -> Option<cost_cache::CachePayload> {
    if ttl_seconds == 0 {
        return None;
    }
    let today = today_utc_date(now_unix_secs());
    let cached = cost_cache::read_cache(data_dir);
    // A cache dated to a previous day is stale regardless of its age: right
    // after the UTC day boundary it can still be well under `ttl_seconds`
    // old, and without this check the chip would stay hidden until the TTL
    // naturally elapses instead of refreshing right away.
    let stale = cached
        .as_ref()
        .is_none_or(|(payload, age)| age.as_secs() >= ttl_seconds || payload.date != today);
    if stale {
        cost_cache::spawn_refresh(data_dir);
    }
    let (payload, _age) = cached?;
    if payload.date != today {
        return None; // cache is from a previous day; wait for the refresh
    }
    Some(payload)
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

/// A tiered-color mini progress bar for one budget window: `label` dimmed,
/// then the bar and the used-of-limit percentage colored by how close
/// `spent` is to `limit`, reusing `context_bar`'s block-character renderer
/// and `display`'s existing yellow/orange/critical thresholds.
fn render_budget_window(
    label: &str,
    spent: f64,
    limit: f64,
    width: u8,
    display: &DisplayConfig,
) -> String {
    let pct = pct_of_budget(spent, limit);
    let bar = context_bar::render_bar(pct.min(100), width as usize);
    let colored = if pct < display.threshold_yellow {
        format!("{GREEN}{bar} {pct}%{RESET}")
    } else if pct < display.threshold_orange {
        format!("{YELLOW}{bar} {pct}%{RESET}")
    } else if pct < display.threshold_critical {
        format!("{ORANGE}{bar} {pct}%{RESET}")
    } else {
        format!("{BLINK_RED}{bar} {pct}%{RESET}")
    };
    format!("{DIM}{label}{RESET} {colored}")
}

/// Percentage of `limit` that `spent` represents, clamped to fit a `u8` —
/// spending can run well past the limit, but the bar and label only need to
/// communicate "how far past", not the exact multiple.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pct_of_budget(spent: f64, limit: f64) -> u8 {
    if limit <= 0.0 || spent <= 0.0 {
        return 0;
    }
    (spent / limit * 100.0).round().clamp(0.0, 255.0) as u8
}

/// The budget-windows line: one mini progress bar per enabled window
/// (session, daily, weekly, monthly, the rolling 5-hour block), or `None`
/// when the feature is off or no window has anything to show yet. Session
/// comes from the live payload and needs no cache; the other four windows
/// share the same on-disk aggregate as `daily_chip` (see
/// `fresh_same_day_cache`), which still returns a same-day cache even
/// while it's stale by TTL (that only triggers a background refresh, never
/// a blocking one) — those four windows are omitted only when there is no
/// cache yet, or the cached date doesn't match today.
pub fn budget_line(
    cfg: &BudgetConfig,
    cost_cfg: &CostConfig,
    display: &DisplayConfig,
    data_dir: Option<&Path>,
    session_cost_usd: Option<f64>,
) -> Option<String> {
    if !cfg.enabled {
        return None;
    }
    let mut segments = Vec::new();

    if cfg.show_session {
        if let Some(spent) = session_cost_usd {
            segments.push(render_budget_window(
                "sess",
                spent,
                cfg.session_usd,
                cfg.bar_width,
                display,
            ));
        }
    }

    let needs_cache = cfg.show_daily || cfg.show_weekly || cfg.show_monthly || cfg.show_block5h;
    if let Some(payload) = needs_cache
        .then(|| data_dir.and_then(|d| fresh_same_day_cache(cost_cfg.ttl_seconds, d)))
        .flatten()
    {
        if cfg.show_daily {
            segments.push(render_budget_window(
                "day",
                payload.total_usd,
                cfg.daily_usd(),
                cfg.bar_width,
                display,
            ));
        }
        if cfg.show_weekly {
            segments.push(render_budget_window(
                "wk",
                payload.weekly_usd,
                cfg.weekly_usd,
                cfg.bar_width,
                display,
            ));
        }
        if cfg.show_monthly {
            segments.push(render_budget_window(
                "mo",
                payload.monthly_usd,
                cfg.monthly_usd(),
                cfg.bar_width,
                display,
            ));
        }
        if cfg.show_block5h {
            segments.push(render_budget_window(
                "5h",
                payload.block5h_usd,
                cfg.block5h_usd,
                cfg.bar_width,
                display,
            ));
        }
    }

    if segments.is_empty() {
        return None;
    }
    Some(segments.join(&format!(" {DIM}│{RESET} ")))
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
    fn parse_line_captures_the_top_level_cwd_field() {
        let line = r#"{"cwd":"/Users/dev/myrepo","timestamp":"2026-08-10T10:00:00Z","requestId":"req_1","message":{"model":"claude-sonnet-5","id":"msg_1","usage":{"input_tokens":100}}}"#;
        let rec = parse_line(line).unwrap();
        assert_eq!(rec.cwd, Some("/Users/dev/myrepo".to_string()));
    }

    #[test]
    fn parse_line_missing_cwd_is_none() {
        let line = &usage_line(
            "2026-08-10T10:00:00Z",
            "claude-sonnet-5",
            "req_1",
            "msg_1",
            100,
        );
        let rec = parse_line(line).unwrap();
        assert_eq!(rec.cwd, None);
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
    fn aggregate_windows_sums_matching_dates_across_files_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("proj");
        std::fs::create_dir_all(&sub).unwrap();
        // 5h before this is 10:31, so the block5h window below excludes the
        // 10:00 record and keeps only the 11:00 one.
        let now = parse_iso8601_utc("2026-08-10T15:31:00Z").unwrap();

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

        let mut analytics = crate::analytics::Sink::new(false, String::new(), String::new());
        let windows = aggregate_windows(dir.path(), now, "2026-08-10", &mut analytics);

        // sonnet-5 input rate is $2/Mtok, so one deduped 1M-token message = $2.
        // opus-4-8 input rate is $5/Mtok, so one 1M-token message = $5.
        assert!((windows.daily.total_usd - 7.0).abs() < 1e-9);
        assert_eq!(windows.daily.by_model.len(), 2);
        // 08-09 falls outside today (excluded above) and outside this week
        // (Aug 10 2026 is a Monday, so the week starts on it), but it is
        // still within the same calendar month.
        assert!((windows.weekly_usd - 7.0).abs() < 1e-9);
        assert!((windows.monthly_usd - 9.0).abs() < 1e-9);
        assert!((windows.block5h_usd - 5.0).abs() < 1e-9); // only the 11:00 record is within 5h of `now`
    }

    #[test]
    fn aggregate_windows_keeps_reading_after_a_malformed_line() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("proj");
        std::fs::create_dir_all(&sub).unwrap();
        let path = sub.join("a.jsonl");
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

        let now = parse_iso8601_utc("2026-08-10T12:00:00Z").unwrap();
        let mut analytics = crate::analytics::Sink::new(false, String::new(), String::new());
        let windows = aggregate_windows(dir.path(), now, "2026-08-10", &mut analytics);
        assert!(
            (windows.daily.total_usd - 2.0).abs() < 1e-9,
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
    fn aggregate_windows_ignores_unreadable_root() {
        let now = parse_iso8601_utc("2026-08-10T12:00:00Z").unwrap();
        let mut analytics = crate::analytics::Sink::new(false, String::new(), String::new());
        let windows = aggregate_windows(
            Path::new("/does/not/exist"),
            now,
            "2026-08-10",
            &mut analytics,
        );
        assert_eq!(windows.daily.total_usd, 0.0);
        assert!(windows.daily.by_model.is_empty());
        assert_eq!(windows.weekly_usd, 0.0);
        assert_eq!(windows.monthly_usd, 0.0);
        assert_eq!(windows.block5h_usd, 0.0);
    }

    #[test]
    fn aggregate_windows_excludes_records_outside_each_window() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("proj");
        std::fs::create_dir_all(&sub).unwrap();
        let now = parse_iso8601_utc("2026-08-10T12:00:00Z").unwrap(); // a Monday

        write_transcript(
            &sub,
            "a.jsonl",
            &[
                // Last month — outside daily/weekly/monthly/block5h alike.
                &usage_line(
                    "2026-07-02T10:00:00Z",
                    "claude-sonnet-5",
                    "req_old",
                    "msg_old",
                    1_000_000,
                ),
            ],
        );

        let mut analytics = crate::analytics::Sink::new(false, String::new(), String::new());
        let windows = aggregate_windows(dir.path(), now, "2026-08-10", &mut analytics);
        assert_eq!(windows.daily.total_usd, 0.0);
        assert_eq!(windows.weekly_usd, 0.0);
        assert_eq!(windows.monthly_usd, 0.0);
        assert_eq!(windows.block5h_usd, 0.0);
    }

    #[test]
    fn days_from_civil_is_the_inverse_of_civil_from_days() {
        for days in [0, 11_017, 19_000, -1, 20_675] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn parse_iso8601_utc_parses_a_standard_transcript_timestamp() {
        assert_eq!(
            parse_iso8601_utc("2026-08-10T16:56:48.920Z"),
            Some(20_675 * 86_400 + 16 * 3600 + 56 * 60 + 48)
        );
    }

    #[test]
    fn parse_iso8601_utc_rejects_malformed_input() {
        assert_eq!(parse_iso8601_utc(""), None);
        assert_eq!(parse_iso8601_utc("not a timestamp"), None);
        assert_eq!(parse_iso8601_utc("2026-13-01T00:00:00Z"), None); // month 13
        assert_eq!(parse_iso8601_utc("2026-08-32T00:00:00Z"), None); // day 32
    }

    #[test]
    fn parse_iso8601_utc_rejects_out_of_range_time_components() {
        // An out-of-range hour/minute/second must not silently produce a
        // wrong-but-valid-looking timestamp that shifts the record into the
        // wrong budget window.
        assert_eq!(parse_iso8601_utc("2026-08-10T99:00:00Z"), None); // hour 99
        assert_eq!(parse_iso8601_utc("2026-08-10T00:99:00Z"), None); // minute 99
        assert_eq!(parse_iso8601_utc("2026-08-10T00:00:99Z"), None); // second 99
        assert!(parse_iso8601_utc("2026-08-10T23:59:59Z").is_some());
    }

    #[test]
    fn start_of_week_anchors_to_the_preceding_monday() {
        // 2026-08-10 is a Monday.
        let monday = parse_iso8601_utc("2026-08-10T00:00:00Z").unwrap();
        let mid_week = parse_iso8601_utc("2026-08-13T15:00:00Z").unwrap();
        let next_monday = parse_iso8601_utc("2026-08-17T00:00:00Z").unwrap();
        assert_eq!(start_of_week(mid_week), monday);
        assert_eq!(start_of_week(monday), monday);
        assert_eq!(start_of_week(next_monday), next_monday);
    }

    #[test]
    fn start_of_month_anchors_to_the_first() {
        let first = parse_iso8601_utc("2026-08-01T00:00:00Z").unwrap();
        let mid_month = parse_iso8601_utc("2026-08-27T09:00:00Z").unwrap();
        assert_eq!(start_of_month(mid_month), first);
    }

    #[test]
    fn start_of_block5h_is_five_hours_before_now() {
        let now = parse_iso8601_utc("2026-08-10T12:00:00Z").unwrap();
        assert_eq!(start_of_block5h(now), now - 5 * 3600);
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
            ..cost_cache::CachePayload::default()
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
            ..cost_cache::CachePayload::default()
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
            ..cost_cache::CachePayload::default()
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

        refresh_daily_cache(&root, dir.path(), false);

        assert!(dir.path().join("cost-cache.json").exists());
        assert!(!dir.path().join("cost-cache.lock").exists());
    }

    // `analytics::store` (and its `db_path`) only exists when the `analytics`
    // feature is compiled in; a plain build has nothing on disk for these
    // two tests to inspect.
    #[cfg(feature = "analytics")]
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

    #[cfg(feature = "analytics")]
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

    #[test]
    fn pct_of_budget_computes_and_clamps() {
        assert_eq!(pct_of_budget(0.0, 10.0), 0);
        assert_eq!(pct_of_budget(5.0, 10.0), 50);
        assert_eq!(pct_of_budget(20.0, 10.0), 200);
        assert_eq!(
            pct_of_budget(-1.0, 10.0),
            0,
            "negative spend never happens, but must not panic"
        );
        assert_eq!(
            pct_of_budget(5.0, 0.0),
            0,
            "a zero limit must not divide by zero"
        );
    }

    #[test]
    fn budget_line_disabled_returns_none() {
        let cfg = BudgetConfig {
            enabled: false,
            ..BudgetConfig::default()
        };
        assert!(budget_line(
            &cfg,
            &CostConfig::default(),
            &DisplayConfig::default(),
            Some(Path::new("/tmp")),
            Some(1.0)
        )
        .is_none());
    }

    #[test]
    fn budget_line_shows_session_without_a_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = BudgetConfig {
            enabled: true,
            show_daily: false,
            show_weekly: false,
            show_monthly: false,
            show_block5h: false,
            session_usd: 10.0,
            ..BudgetConfig::default()
        };
        let line = budget_line(
            &cfg,
            &CostConfig::default(),
            &DisplayConfig::default(),
            Some(dir.path()),
            Some(5.0),
        )
        .unwrap();
        assert!(line.contains("sess"));
        assert!(line.contains("50%"));
        assert!(!line.contains("day"));
    }

    #[test]
    fn budget_line_omits_session_when_no_session_cost_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = BudgetConfig {
            enabled: true,
            show_daily: false,
            show_weekly: false,
            show_monthly: false,
            show_block5h: false,
            ..BudgetConfig::default()
        };
        assert!(budget_line(
            &cfg,
            &CostConfig::default(),
            &DisplayConfig::default(),
            Some(dir.path()),
            None
        )
        .is_none());
    }

    #[test]
    fn budget_line_skips_the_cache_entirely_when_no_cached_window_is_shown() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = BudgetConfig {
            enabled: true,
            show_daily: false,
            show_weekly: false,
            show_monthly: false,
            show_block5h: false,
            ..BudgetConfig::default()
        };
        budget_line(
            &cfg,
            &CostConfig::default(),
            &DisplayConfig::default(),
            Some(dir.path()),
            Some(1.0),
        );
        assert!(
            !dir.path().join("cost-cache.lock").exists(),
            "no cached window is enabled, so the cache must never be read or refreshed"
        );
    }

    #[test]
    fn budget_line_reads_daily_weekly_monthly_block5h_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let payload = cost_cache::CachePayload {
            date: today_utc_date(now_unix_secs()),
            total_usd: 4.0,
            by_model: Vec::new(),
            weekly_usd: 20.0,
            monthly_usd: 80.0,
            block5h_usd: 3.0,
        };
        cost_cache::write_cache(dir.path(), &payload).unwrap();
        let cfg = BudgetConfig {
            enabled: true,
            show_session: false,
            weekly_usd: 100.0,
            workdays: 5.0,
            block5h_usd: 15.0,
            ..BudgetConfig::default()
        };

        let line = budget_line(
            &cfg,
            &CostConfig::default(),
            &DisplayConfig::default(),
            Some(dir.path()),
            None,
        )
        .unwrap();
        assert!(line.contains("day"));
        assert!(line.contains("wk"));
        assert!(line.contains("mo"));
        assert!(line.contains("5h"));
    }

    #[test]
    fn budget_line_respects_individual_show_toggles() {
        let dir = tempfile::tempdir().unwrap();
        let payload = cost_cache::CachePayload {
            date: today_utc_date(now_unix_secs()),
            total_usd: 4.0,
            ..cost_cache::CachePayload::default()
        };
        cost_cache::write_cache(dir.path(), &payload).unwrap();
        let cfg = BudgetConfig {
            enabled: true,
            show_session: false,
            show_weekly: false,
            show_monthly: false,
            show_block5h: false,
            ..BudgetConfig::default()
        };

        let line = budget_line(
            &cfg,
            &CostConfig::default(),
            &DisplayConfig::default(),
            Some(dir.path()),
            Some(1.0),
        )
        .unwrap();
        assert!(line.contains("day"));
        assert!(!line.contains("wk"));
        assert!(!line.contains("sess"));
    }

    #[test]
    fn render_budget_window_colors_by_threshold() {
        let display = DisplayConfig::default();
        let green = render_budget_window("x", 1.0, 100.0, 6, &display); // 1%
        let yellow = render_budget_window("x", 55.0, 100.0, 6, &display); // 55%
        let orange = render_budget_window("x", 70.0, 100.0, 6, &display); // 70%
        let red = render_budget_window("x", 95.0, 100.0, 6, &display); // 95%
        assert!(green.contains(GREEN));
        assert!(yellow.contains(YELLOW));
        assert!(orange.contains(ORANGE));
        assert!(red.contains(BLINK_RED));
    }
}
