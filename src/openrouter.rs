//! `ferrisbar openrouter`: print the `OpenRouter` key's budget and spend, and
//! the account's lifetime credits and what remains of them.
//!
//! Runs only when invoked explicitly, never on the statusline render path, so
//! the network call cannot slow a prompt. The HTTP request is made by `curl`,
//! which keeps this crate at its four runtime dependencies. The key is handed
//! to `curl` on stdin as a config block, so it never appears in the process
//! list.

use serde_json::Value;
use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

const API: &str = "https://openrouter.ai/api/v1";
const KEY_VAR: &str = "OPENROUTER_API_KEY";
const MANAGEMENT_VAR: &str = "OPENROUTER_MANAGEMENT_KEY";

/// Entry point for the subcommand. Returns the process exit code.
pub fn run() -> i32 {
    let key = match env_key(KEY_VAR) {
        Ok(Some(key)) => key,
        Ok(None) => {
            eprintln!("ferrisbar openrouter: set {KEY_VAR} (for example with `sops exec-env`)");
            return 1;
        }
        Err(msg) => {
            eprintln!("ferrisbar openrouter: {msg}");
            return 1;
        }
    };

    // The key's own budget is essential, so its failure is fatal.
    let key_info = match get(&format!("{API}/key"), &key) {
        Ok(json) => json,
        Err(msg) => {
            eprintln!("ferrisbar openrouter: {msg}");
            return 1;
        }
    };
    print!("{}", format_key(&key_info["data"]));

    // Credits accept the normal key, so try that unless a management key is
    // set. A refusal is reported in place, not treated as a failure.
    let credits_key = match env_key(MANAGEMENT_VAR) {
        Ok(Some(management)) => management,
        Ok(None) => key,
        Err(msg) => {
            eprintln!("ferrisbar openrouter: {msg}");
            return 1;
        }
    };
    print!("\n{}", credits_section(&credits_key));
    0
}

/// Reads a credential from the environment. Empty counts as unset. A value
/// with characters outside the token alphabet is refused, because it would
/// need escaping inside the curl config block.
fn env_key(name: &str) -> Result<Option<String>, String> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "{name} contains characters that cannot be sent safely; check the value"
        ));
    }
    Ok(Some(value.to_string()))
}

/// GETs `url` with a bearer token and parses the JSON body.
fn get(url: &str, key: &str) -> Result<Value, String> {
    let config = format!("url = \"{url}\"\nheader = \"Authorization: Bearer {key}\"\n");
    let mut child = Command::new("curl")
        .args(["-sS", "--fail-with-body", "--max-time", "15", "-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run curl: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(config.as_bytes())
            .map_err(|e| format!("could not pass the request to curl: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if !output.status.success() {
        let detail = serde_json::from_slice::<Value>(&output.stdout)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| String::from_utf8_lossy(&output.stderr).trim().to_string());
        return Err(format!("request to {url} failed: {detail}"));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("unexpected response from {url}: {e}"))
}

fn usd(value: f64) -> String {
    format!("${value:.2}")
}

fn usd_field(data: &Value, key: &str) -> String {
    data[key].as_f64().map_or_else(|| "?".to_string(), usd)
}

/// The budget that blocks requests first, normalised across the two shapes
/// `OpenRouter` uses: `effective_budget` (`limit_usd`, `remaining_usd`,
/// `reset_interval`) and the key's own top-level fields (`limit`,
/// `limit_remaining`, `limit_reset`).
struct Budget {
    limit: Option<f64>,
    remaining: Option<f64>,
    reset: Option<String>,
    scope: Option<String>,
}

fn budget(data: &Value) -> Budget {
    let effective = &data["effective_budget"];
    if effective.is_object() {
        Budget {
            limit: effective["limit_usd"].as_f64(),
            remaining: effective["remaining_usd"].as_f64(),
            reset: effective["reset_interval"].as_str().map(str::to_string),
            scope: effective["scope"].as_str().map(|s| s.replace('_', " ")),
        }
    } else {
        Budget {
            limit: data["limit"].as_f64(),
            remaining: data["limit_remaining"].as_f64(),
            reset: data["limit_reset"].as_str().map(str::to_string),
            scope: None,
        }
    }
}

fn budget_line(data: &Value) -> String {
    let b = budget(data);
    let Some(limit) = b.limit else {
        return "none set on this key".to_string();
    };
    let mut line = usd(limit);
    if let Some(reset) = &b.reset {
        let _ = write!(line, " {reset}");
    }
    let remaining = b.remaining.map_or_else(|| "?".to_string(), usd);
    let _ = write!(line, " · {remaining} remaining");
    if let Some(scope) = &b.scope {
        let _ = write!(line, " ({scope})");
    }
    line
}

/// The key's own budget and spend. Missing fields print as `?` rather than
/// failing, so a shape change degrades the output instead of breaking it.
fn format_key(data: &Value) -> String {
    let mut out = String::new();
    // The key's label is deliberately not printed: OpenRouter labels are
    // masked key fragments, and terminal output gets pasted and shared.
    let _ = writeln!(out, "OpenRouter API key");
    let _ = writeln!(out, "  Budget:    {}", budget_line(data));
    let _ = writeln!(
        out,
        "  Usage:     today {} · this week {} · this month {} · all time {}",
        usd_field(data, "usage_daily"),
        usd_field(data, "usage_weekly"),
        usd_field(data, "usage_monthly"),
        usd_field(data, "usage"),
    );
    // Free-model requests are a per-day quota separate from the spend budget.
    let free = &data["free_model_daily_requests"];
    if free.is_object() {
        let count = |key: &str| {
            free[key]
                .as_u64()
                .map_or_else(|| "?".to_string(), |n| n.to_string())
        };
        let _ = writeln!(
            out,
            "  Free models: {} of {} requests today ({} left)",
            count("used"),
            count("limit"),
            count("remaining"),
        );
    }
    if data["is_free_tier"].as_bool() == Some(true) {
        let _ = writeln!(out, "  Tier:      free");
    }
    if let Some(expires) = data["expires_at"].as_str() {
        let _ = writeln!(out, "  Expires:   {expires}");
    }
    out
}

/// Fetches the account credits and renders them, or a one-line note when the
/// key is not allowed to read them.
fn credits_section(key: &str) -> String {
    match get(&format!("{API}/credits"), key) {
        Ok(json) => format_credits(&json["data"]),
        Err(msg) => format!("Account credits: unavailable ({msg})\n"),
    }
}

/// Lifetime credit totals for the account. `total_credits` is everything ever
/// purchased and `total_usage` everything ever spent, both in USD, so the
/// remaining balance is their difference.
fn format_credits(data: &Value) -> String {
    let purchased = data["total_credits"].as_f64();
    let used = data["total_usage"].as_f64();
    let remaining = match (purchased, used) {
        (Some(p), Some(u)) => usd(p - u),
        _ => "?".to_string(),
    };
    let mut out = String::new();
    let _ = writeln!(out, "Account credits (lifetime)");
    let _ = writeln!(out, "  Remaining: {remaining}");
    let _ = writeln!(
        out,
        "  Purchased: {}",
        purchased.map_or_else(|| "?".to_string(), usd)
    );
    let _ = writeln!(
        out,
        "  Used:      {}",
        used.map_or_else(|| "?".to_string(), usd)
    );
    out
}

#[cfg(test)]
// The asserted values are exact literals that round-trip through `{:.2}`.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_budget_comes_from_effective_budget_when_present() {
        let data = json!({
            "label": "ferrisbar",
            "limit": 50.0,
            "limit_remaining": 12.5,
            "limit_reset": "monthly",
            "effective_budget": {
                "scope": "key_limit",
                "limit_usd": 5.0,
                "remaining_usd": 4.0,
                "reset_interval": "daily",
            },
            "usage": 37.5,
            "usage_daily": 1.25,
            "usage_weekly": 9.0,
            "usage_monthly": 30.0,
            "is_free_tier": false,
            "free_model_daily_requests": {"used": 125, "limit": 1000, "remaining": 875},
        });
        let text = format_key(&data);
        assert!(text.starts_with("OpenRouter API key\n"), "{text}");
        assert!(
            !text.contains("ferrisbar"),
            "label must not be printed: {text}"
        );
        // The effective budget wins over the top-level limit.
        assert!(
            text.contains("$5.00 daily · $4.00 remaining (key limit)"),
            "{text}"
        );
        assert!(!text.contains("$50.00"), "{text}");
        assert!(text.contains("today $1.25"), "{text}");
        assert!(text.contains("all time $37.50"), "{text}");
        assert!(!text.contains("Tier:"), "{text}");
        assert!(
            text.contains("Free models: 125 of 1000 requests today (875 left)"),
            "{text}"
        );
    }

    #[test]
    fn key_budget_falls_back_to_top_level_limit() {
        let data = json!({
            "limit": 50.0,
            "limit_remaining": 12.5,
            "limit_reset": "monthly",
            "effective_budget": null,
        });
        let text = format_key(&data);
        assert!(text.contains("$50.00 monthly · $12.50 remaining"), "{text}");
    }

    #[test]
    fn key_without_a_budget_says_so() {
        let text = format_key(&json!({"limit": null, "effective_budget": null, "usage": 2.0}));
        assert!(text.contains("none set on this key"), "{text}");
    }

    #[test]
    fn missing_fields_print_question_marks_not_panics() {
        let text = format_key(&Value::Null);
        assert!(text.starts_with("OpenRouter API key\n"), "{text}");
        assert!(text.contains("today ?"), "{text}");
    }

    #[test]
    fn credits_show_remaining_purchased_and_used() {
        let text = format_credits(&json!({"total_credits": 400.0, "total_usage": 371.73}));
        assert_eq!(
            text,
            "Account credits (lifetime)\n  Remaining: $28.27\n  Purchased: $400.00\n  Used:      $371.73\n"
        );
    }

    #[test]
    fn credits_with_missing_totals_print_question_marks() {
        let text = format_credits(&Value::Null);
        assert!(text.contains("Remaining: ?"), "{text}");
    }

    #[test]
    fn env_key_refuses_characters_that_would_need_escaping() {
        // A variable name used only by this test, so parallel tests cannot
        // collide with it.
        let name = "FERRISBAR_TEST_OPENROUTER_KEY_VALIDATION";
        std::env::set_var(name, "bad\"value");
        assert!(env_key(name).is_err());
        std::env::set_var(name, "sk-or-v1_abc.123");
        assert_eq!(env_key(name).unwrap().as_deref(), Some("sk-or-v1_abc.123"));
        std::env::set_var(name, "   ");
        assert_eq!(env_key(name).unwrap(), None);
        std::env::remove_var(name);
    }
}
