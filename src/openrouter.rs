//! `ferrisbar openrouter`: print the `OpenRouter` key's spend and limit, and the
//! account's credit totals when a management key is available.
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

    let key_info = match get(&format!("{API}/key"), &key) {
        Ok(json) => json,
        Err(msg) => {
            eprintln!("ferrisbar openrouter: {msg}");
            return 1;
        }
    };
    print!("{}", format_key(&key_info["data"]));

    match env_key(MANAGEMENT_VAR) {
        Ok(Some(management)) => match get(&format!("{API}/credits"), &management) {
            Ok(json) => print!("{}", format_credits(&json["data"])),
            Err(msg) => {
                eprintln!("ferrisbar openrouter: {msg}");
                return 1;
            }
        },
        Ok(None) => {}
        Err(msg) => {
            eprintln!("ferrisbar openrouter: {msg}");
            return 1;
        }
    }
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

/// The key's own limit and spend. Missing fields print as `?` rather than
/// failing, so a shape change degrades the output instead of breaking it.
fn format_key(data: &Value) -> String {
    let mut out = String::new();
    // The key's label is deliberately not printed: OpenRouter labels are
    // masked key fragments, and terminal output gets pasted and shared.
    let _ = writeln!(out, "OpenRouter API key");

    if let Some(limit) = data["limit"].as_f64() {
        let remaining = usd_field(data, "limit_remaining");
        let reset = data["limit_reset"].as_str().unwrap_or("no reset");
        let _ = writeln!(
            out,
            "  Limit:     {} ({remaining} remaining, {reset})",
            usd(limit)
        );
    } else {
        let _ = writeln!(out, "  Limit:     none set on this key");
    }

    let _ = writeln!(
        out,
        "  Usage:     today {} · this week {} · this month {} · all time {}",
        usd_field(data, "usage_daily"),
        usd_field(data, "usage_weekly"),
        usd_field(data, "usage_monthly"),
        usd_field(data, "usage"),
    );
    if data["is_free_tier"].as_bool() == Some(true) {
        let _ = writeln!(out, "  Tier:      free");
    }
    if let Some(expires) = data["expires_at"].as_str() {
        let _ = writeln!(out, "  Expires:   {expires}");
    }
    out
}

/// Account-wide totals. Needs a management key; a normal key is refused by
/// `OpenRouter` with a 403, which surfaces as an error before this runs.
fn format_credits(data: &Value) -> String {
    let total = data["total_credits"].as_f64();
    let used = data["total_usage"].as_f64();
    let remaining = match (total, used) {
        (Some(t), Some(u)) => usd(t - u),
        _ => "?".to_string(),
    };
    format!(
        "Account credits: {} purchased, {} used, {remaining} remaining\n",
        total.map_or_else(|| "?".to_string(), usd),
        used.map_or_else(|| "?".to_string(), usd),
    )
}

#[cfg(test)]
// The asserted values are exact literals that round-trip through `{:.2}`.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_with_a_limit_shows_limit_remaining_and_usage() {
        let data = json!({
            "label": "ferrisbar",
            "limit": 50.0,
            "limit_remaining": 12.5,
            "limit_reset": "monthly",
            "usage": 37.5,
            "usage_daily": 1.25,
            "usage_weekly": 9.0,
            "usage_monthly": 30.0,
            "is_free_tier": false,
        });
        let text = format_key(&data);
        assert!(text.starts_with("OpenRouter API key\n"), "{text}");
        assert!(
            !text.contains("ferrisbar"),
            "label must not be printed: {text}"
        );
        assert!(
            text.contains("$50.00 ($12.50 remaining, monthly)"),
            "{text}"
        );
        assert!(text.contains("today $1.25"), "{text}");
        assert!(text.contains("all time $37.50"), "{text}");
        assert!(!text.contains("Tier:"), "{text}");
    }

    #[test]
    fn key_without_a_limit_says_so() {
        let text = format_key(&json!({"label": "x", "limit": null, "usage": 2.0}));
        assert!(text.contains("none set on this key"), "{text}");
    }

    #[test]
    fn missing_fields_print_question_marks_not_panics() {
        let text = format_key(&Value::Null);
        assert!(text.starts_with("OpenRouter API key\n"), "{text}");
        assert!(text.contains("today ?"), "{text}");
    }

    #[test]
    fn credits_show_purchased_used_and_remaining() {
        let text = format_credits(&json!({"total_credits": 100.5, "total_usage": 25.75}));
        assert_eq!(
            text,
            "Account credits: $100.50 purchased, $25.75 used, $74.75 remaining\n"
        );
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
