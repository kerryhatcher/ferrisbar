//! `ferrisbar report` — reads what `store.rs` writes and renders it as
//! JSON or CSV, either for one repo (default: the one `cwd` resolves to)
//! or, with `--all`, one summary row per tracked repo.

use super::store::{db_path, decode_key, Row, TABLE};
use redb::ReadableTable;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

// Nothing in `main` builds a `Format`, `Options`, or calls `parse_args`/
// `render` yet — that's Task 8's CLI wiring for the `report` subcommand.
// Until then, a `--features analytics` build sees no caller and flags this
// module's public surface (and everything it touches) as dead. Task 7 only
// builds the report engine; Task 8 is what makes it live.
#[allow(dead_code)]
pub enum Format {
    Json,
    Csv,
}

#[allow(dead_code)]
pub struct Options {
    pub repo_key: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub all: bool,
    pub format: Format,
}

// Same story as `Format`/`Options` above: only built by `read_all`, itself
// unreachable from `main` until Task 8.
#[allow(dead_code)]
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

/// Parses `ferrisbar report`'s own flags. `args` excludes the `report`
/// token itself. `Err` holds a human-readable message for an unrecognized
/// flag or one missing its value; the caller prints it to stderr.
// Unreachable from `main` until Task 8 wires the `report` subcommand.
#[allow(dead_code)]
pub fn parse_args(args: &[String]) -> Result<Options, String> {
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
                    other => {
                        return Err(format!("unknown --format {other} (expected json or csv)"))
                    }
                };
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(opts)
}

// Only called from `parse_args`, itself unreachable until Task 8.
#[allow(dead_code)]
fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    let value = args
        .get(*i + 1)
        .ok_or_else(|| format!("{flag} needs a value"))?
        .clone();
    *i += 2;
    Ok(value)
}

// Only called from `render`/`render_summary`, themselves unreachable until
// Task 8.
#[allow(dead_code)]
fn in_range(date: &str, from: Option<&str>, to: Option<&str>) -> bool {
    // YYYY-MM-DD sorts lexically the same as chronologically.
    from.is_none_or(|f| date >= f) && to.is_none_or(|t| date <= t)
}

/// Every stored row, decoded. A missing/unreadable/corrupt database, or
/// any individually undecodable row, is skipped rather than failing —
/// "no data yet" is normal for a freshly enabled feature.
// Only called from `render`, itself unreachable until Task 8.
#[allow(dead_code)]
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

// Unreachable from `main` until Task 8 wires the `report` subcommand.
#[allow(dead_code)]
pub fn render(data_dir: &Path, default_repo_key: &str, opts: &Options) -> String {
    let rows = read_all(data_dir);
    if opts.all {
        return render_summary(&rows, opts);
    }
    let target = opts.repo_key.as_deref().unwrap_or(default_repo_key);
    let mut filtered: Vec<&ReportRow> = rows
        .iter()
        .filter(|r| {
            r.repo_key == target && in_range(&r.date, opts.from.as_deref(), opts.to.as_deref())
        })
        .collect();
    filtered.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.model.cmp(&b.model)));
    render_rows(&filtered, &opts.format)
}

// Only constructed by `render_rows`, itself unreachable until Task 8.
#[allow(dead_code)]
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

// Only called from `render`, itself unreachable until Task 8.
#[allow(dead_code)]
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
// Only called from `render_rows`/`render_summary`, themselves unreachable
// until Task 8.
#[allow(dead_code)]
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

// Only called from `render`, itself unreachable until Task 8.
#[allow(dead_code)]
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
    summary.sort_by(|a, b| {
        b.1 .1
            .partial_cmp(&a.1 .1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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
            "--repo",
            "remote:github.com/a/b",
            "--from",
            "2026-08-01",
            "--to",
            "2026-08-10",
            "--format",
            "csv",
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
        write_row(
            dir.path(),
            "2026-08-11",
            "local:a",
            "a",
            "claude-sonnet-5",
            2.0,
        );
        write_row(
            dir.path(),
            "2026-08-10",
            "local:a",
            "a",
            "claude-sonnet-5",
            1.0,
        );
        write_row(
            dir.path(),
            "2026-08-10",
            "local:b",
            "b",
            "claude-sonnet-5",
            9.0,
        );

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
        write_row(
            dir.path(),
            "2026-08-10",
            "local:b",
            "b",
            "claude-sonnet-5",
            9.0,
        );
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
        write_row(
            dir.path(),
            "2026-08-09",
            "local:a",
            "a",
            "claude-sonnet-5",
            1.0,
        );
        write_row(
            dir.path(),
            "2026-08-10",
            "local:a",
            "a",
            "claude-sonnet-5",
            2.0,
        );
        write_row(
            dir.path(),
            "2026-08-11",
            "local:a",
            "a",
            "claude-sonnet-5",
            3.0,
        );
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
        write_row(
            dir.path(),
            "2026-08-10",
            "local:a",
            "a",
            "claude-sonnet-5",
            1.0,
        );
        write_row(
            dir.path(),
            "2026-08-11",
            "local:a",
            "a",
            "claude-opus-4-8",
            2.0,
        );
        write_row(
            dir.path(),
            "2026-08-10",
            "local:b",
            "b",
            "claude-sonnet-5",
            5.0,
        );
        let opts = Options {
            all: true,
            ..default_options()
        };
        let out = render(dir.path(), "local:a", &opts);
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let rows = parsed.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let a = rows.iter().find(|r| r["repo"] == "local:a").unwrap();
        assert!(
            (a["cost_usd"].as_f64().unwrap() - 3.0).abs() < 1e-9,
            "a's two rows sum to 3.0"
        );
    }

    #[test]
    fn csv_format_has_a_header_and_one_line_per_row() {
        let dir = tempfile::tempdir().unwrap();
        write_row(
            dir.path(),
            "2026-08-10",
            "local:a",
            "a",
            "claude-sonnet-5",
            1.5,
        );
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
        assert_eq!(
            lines.next().unwrap(),
            "2026-08-10,local:a,a,claude-sonnet-5,1.500000,1000,500,0,0"
        );
        assert!(lines.next().is_none());
    }
}
