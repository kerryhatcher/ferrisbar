/// Written verbatim when no config file exists. A static string rather than
/// a serialized `Config`, because serializing would strip the comments —
/// which are the entire reason TOML was chosen over the already-vendored
/// `serde_json`.
///
/// The spec's `[display]` block is deliberately absent: Phase 1 does not
/// parse those keys, and emitting keys the binary ignores invites bug
/// reports from users who set `bar_width` and see nothing change.
// Used by Task 4 (file I/O) and tests; Task 3 only declares the constant.
#[allow(dead_code)]
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

// Used by Task 4 and tests; Task 3 only declares these constants.
#[allow(dead_code)]
pub const MIN_MAX_SIZE_BYTES: u64 = 4096;
#[allow(dead_code)]
pub const MIN_MAX_ARCHIVES: u8 = 1;
#[allow(dead_code)]
pub const MAX_MAX_ARCHIVES: u8 = 64;

// Used by Task 4 and tests; Task 3 only declares this enum.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseWarning {
    Syntax(String),
}

// Used by Task 4 and tests; Task 3 only declares this struct.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    pub enabled: bool,
    pub level: String,
    pub path: String,
    pub max_size_bytes: u64,
    pub max_archives: u8,
}

// Used by Task 4 and tests; Task 3 only declares this struct.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeConfig {
    pub config_dir: String,
    pub auto_compact_window: f64,
}

// Used by Task 4 and tests; Task 3 only declares this struct.
#[allow(dead_code)]
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

// Although Config could be derived Default (since LogConfig and ClaudeConfig have
// Default impls), we keep the manual impl for consistency with the nested manual
// Default implementations for LogConfig and ClaudeConfig.
#[allow(clippy::derivable_impls)]
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
// Used by Task 4 (file I/O) and tests; Task 3 only declares this public function.
#[allow(dead_code)]
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
        .map_or(defaults.log.max_size_bytes, |v| v.max(MIN_MAX_SIZE_BYTES));

    let max_archives = get_integer(log, "max_archives").map_or(defaults.log.max_archives, |v| {
        if v < 0 {
            defaults.log.max_archives
        } else {
            // After clamping to [1, 64], the value is guaranteed to fit in u8.
            // The casts are safe: the range is always positive and fits in u8.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let clamped = v.clamp(i64::from(MIN_MAX_ARCHIVES), i64::from(MAX_MAX_ARCHIVES)) as u8;
            clamped
        }
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
        assert_eq!(
            warnings.len(),
            1,
            "one warning per render, never one per key"
        );
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
