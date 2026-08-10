use std::path::Path;

/// Written verbatim when no config file exists. A static string rather than
/// a serialized `Config`, because serializing would strip the comments —
/// which are the entire reason TOML was chosen over the already-vendored
/// `serde_json`.
// Used by Task 4 (file I/O) and tests; Task 3 only declares the constant.
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

[display]
bar_width          = 10
threshold_yellow   = 50
threshold_orange   = 65
threshold_critical = 80
show_task          = true

[cost]
show_session      = true   # session-to-date cost next to the context bar
show_daily        = true   # today's total + per-model split, on its own line
ttl_seconds       = 90     # daily total cache lifetime in seconds; a stale
                            # cache triggers a background refresh; 0 disables
                            # the daily line entirely
breakdown_min_usd = 0.005  # per-model entries below this are folded into the
                            # total rather than listed individually
"#;

pub const MIN_MAX_SIZE_BYTES: u64 = 4096;
pub const MIN_MAX_ARCHIVES: u8 = 1;
pub const MAX_MAX_ARCHIVES: u8 = 64;
pub const MIN_BAR_WIDTH: u8 = 1;
pub const MAX_BAR_WIDTH: u8 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseWarning {
    Syntax(String),
    Create(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    pub enabled: bool,
    pub level: String,
    pub path: String,
    pub max_size_bytes: u64,
    pub max_archives: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeConfig {
    pub config_dir: String,
    pub auto_compact_window: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayConfig {
    pub bar_width: u8,
    pub threshold_yellow: u8,
    pub threshold_orange: u8,
    pub threshold_critical: u8,
    pub show_task: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            bar_width: 10,
            threshold_yellow: 50,
            threshold_orange: 65,
            threshold_critical: 80,
            show_task: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostConfig {
    pub show_session: bool,
    pub show_daily: bool,
    pub ttl_seconds: u64,
    pub breakdown_min_usd: f64,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            show_session: true,
            show_daily: true,
            ttl_seconds: 90,
            breakdown_min_usd: 0.005,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub log: LogConfig,
    pub claude: ClaudeConfig,
    pub display: DisplayConfig,
    pub cost: CostConfig,
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

// Although Config could be derived Default (since LogConfig, ClaudeConfig, and
// DisplayConfig have Default impls), we keep the manual impl for consistency
// with the nested manual Default implementations.
#[allow(clippy::derivable_impls)]
impl Default for Config {
    fn default() -> Self {
        Self {
            log: LogConfig::default(),
            claude: ClaudeConfig::default(),
            display: DisplayConfig::default(),
            cost: CostConfig::default(),
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
        display: {
            let display = section(&table, "display");
            let defaults_display = DisplayConfig::default();

            let bar_width = get_integer(display, "bar_width")
                .and_then(|v| u8::try_from(v).ok())
                .map_or(defaults_display.bar_width, |v| {
                    v.clamp(MIN_BAR_WIDTH, MAX_BAR_WIDTH)
                });

            let threshold_yellow = get_integer(display, "threshold_yellow")
                .and_then(|v| u8::try_from(v).ok())
                .unwrap_or(defaults_display.threshold_yellow);

            let threshold_orange = get_integer(display, "threshold_orange")
                .and_then(|v| u8::try_from(v).ok())
                .unwrap_or(defaults_display.threshold_orange);

            let threshold_critical = get_integer(display, "threshold_critical")
                .and_then(|v| u8::try_from(v).ok())
                .unwrap_or(defaults_display.threshold_critical);

            // Thresholds must be monotonically increasing. When they are
            // not, all three fall back to defaults — a single out-of-order
            // value is a configuration error, not a reason to guess which
            // one the user meant.
            let (threshold_yellow, threshold_orange, threshold_critical) =
                if threshold_yellow < threshold_orange && threshold_orange < threshold_critical {
                    (threshold_yellow, threshold_orange, threshold_critical)
                } else {
                    (
                        defaults_display.threshold_yellow,
                        defaults_display.threshold_orange,
                        defaults_display.threshold_critical,
                    )
                };

            let show_task = get_bool(display, "show_task").unwrap_or(defaults_display.show_task);

            DisplayConfig {
                bar_width,
                threshold_yellow,
                threshold_orange,
                threshold_critical,
                show_task,
            }
        },
        cost: {
            let cost = section(&table, "cost");
            let defaults_cost = CostConfig::default();

            CostConfig {
                show_session: get_bool(cost, "show_session").unwrap_or(defaults_cost.show_session),
                show_daily: get_bool(cost, "show_daily").unwrap_or(defaults_cost.show_daily),
                ttl_seconds: get_integer(cost, "ttl_seconds")
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(defaults_cost.ttl_seconds),
                breakdown_min_usd: get_number(cost, "breakdown_min_usd")
                    .filter(|v| *v >= 0.0)
                    .unwrap_or(defaults_cost.breakdown_min_usd),
            }
        },
    };

    (config, Vec::new())
}

/// Reads the config file, creating it from `TEMPLATE` when absent.
///
/// Infallible by construction: a missing home directory, an unreadable
/// file, a read-only directory, or malformed TOML all yield defaults and
/// let the statusline render. Warnings are returned as data rather than
/// logged directly, because the config is what determines where the log
/// lives — the caller flushes them once the logger exists.
// Public API; wired into main by Task 8.
pub fn load(path: Option<&Path>) -> (Config, Vec<ParseWarning>) {
    let Some(path) = path else {
        return (Config::default(), Vec::new());
    };

    match std::fs::read_to_string(path) {
        Ok(contents) => from_toml_str(&contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut warnings = Vec::new();
            if let Err(e) = create_template(path) {
                warnings.push(ParseWarning::Create(e.to_string()));
            }
            (Config::default(), warnings)
        }
        Err(e) => (Config::default(), vec![ParseWarning::Create(e.to_string())]),
    }
}

fn create_template(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, TEMPLATE)
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
    fn template_includes_display_block() {
        assert!(TEMPLATE.contains("[display]"));
        assert!(TEMPLATE.contains("bar_width"));
        assert!(TEMPLATE.contains("threshold_yellow"));
        assert!(TEMPLATE.contains("show_task"));
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

    #[test]
    fn load_with_no_path_yields_defaults_silently() {
        let (c, warnings) = load(None);
        assert_eq!(c, Config::default());
        assert!(
            warnings.is_empty(),
            "an unresolvable home is not a fault to report"
        );
    }

    #[test]
    fn load_creates_the_file_and_its_parent_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");

        let (c, warnings) = load(Some(&path));

        assert!(path.exists(), "config file must be created on first run");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
        assert_eq!(c, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_reads_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[log]\nmax_archives = 3\n").unwrap();

        let (c, warnings) = load(Some(&path));

        assert_eq!(c.log.max_archives, 3);
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_does_not_overwrite_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "[log]\nmax_archives = 3\n";
        std::fs::write(&path, original).unwrap();

        let _ = load(Some(&path));

        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn load_of_malformed_file_warns_once_and_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "not = = toml";
        std::fs::write(&path, original).unwrap();

        let (c, warnings) = load(Some(&path));

        assert_eq!(c, Config::default());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "a malformed config is never overwritten — the user's edits are theirs"
        );
    }

    #[test]
    fn display_defaults_match_the_documented_values() {
        let d = DisplayConfig::default();
        assert_eq!(d.bar_width, 10);
        assert_eq!(d.threshold_yellow, 50);
        assert_eq!(d.threshold_orange, 65);
        assert_eq!(d.threshold_critical, 80);
        assert!(d.show_task);
    }

    #[test]
    fn display_values_are_read_from_toml() {
        let (c, _) = from_toml_str(
            "[display]\nbar_width = 20\nthreshold_yellow = 40\n\
             threshold_orange = 60\nthreshold_critical = 90\nshow_task = false\n",
        );
        assert_eq!(c.display.bar_width, 20);
        assert_eq!(c.display.threshold_yellow, 40);
        assert_eq!(c.display.threshold_orange, 60);
        assert_eq!(c.display.threshold_critical, 90);
        assert!(!c.display.show_task);
    }

    #[test]
    fn bar_width_clamps_to_range() {
        let (zero, _) = from_toml_str("[display]\nbar_width = 0\n");
        assert_eq!(zero.display.bar_width, MIN_BAR_WIDTH);
        let (huge, _) = from_toml_str("[display]\nbar_width = 200\n");
        assert_eq!(huge.display.bar_width, MAX_BAR_WIDTH);
    }

    #[test]
    fn bar_width_negative_falls_back() {
        let (c, _) = from_toml_str("[display]\nbar_width = -5\n");
        assert_eq!(c.display.bar_width, DisplayConfig::default().bar_width);
    }

    #[test]
    fn out_of_order_thresholds_fall_back_to_defaults() {
        let (c, _) = from_toml_str(
            "[display]\nthreshold_yellow = 80\nthreshold_orange = 50\nthreshold_critical = 90\n",
        );
        // yellow >= orange, so all three fall back.
        assert_eq!(c.display.threshold_yellow, 50);
        assert_eq!(c.display.threshold_orange, 65);
        assert_eq!(c.display.threshold_critical, 80);
    }

    #[test]
    fn equal_thresholds_fall_back() {
        let (c, _) = from_toml_str(
            "[display]\nthreshold_yellow = 50\nthreshold_orange = 50\nthreshold_critical = 80\n",
        );
        assert_eq!(c.display.threshold_yellow, 50);
        assert_eq!(c.display.threshold_orange, 65);
        assert_eq!(c.display.threshold_critical, 80);
    }

    #[test]
    fn critical_below_orange_falls_back() {
        let (c, _) = from_toml_str(
            "[display]\nthreshold_yellow = 30\nthreshold_orange = 60\nthreshold_critical = 40\n",
        );
        assert_eq!(c.display.threshold_yellow, 50);
        assert_eq!(c.display.threshold_orange, 65);
        assert_eq!(c.display.threshold_critical, 80);
    }

    #[test]
    fn partial_display_block_fills_in_defaults() {
        let (c, _) = from_toml_str("[display]\nbar_width = 5\n");
        assert_eq!(c.display.bar_width, 5);
        assert_eq!(c.display.threshold_yellow, 50);
        assert!(c.display.show_task);
    }

    #[test]
    fn template_round_trips_including_display() {
        let (c, warnings) = from_toml_str(TEMPLATE);
        assert!(warnings.is_empty());
        assert_eq!(c, Config::default());
    }

    #[test]
    fn cost_defaults_match_the_documented_values() {
        let c = CostConfig::default();
        assert!(c.show_session);
        assert!(c.show_daily);
        assert_eq!(c.ttl_seconds, 90);
        assert!((c.breakdown_min_usd - 0.005).abs() < f64::EPSILON);
    }

    #[test]
    fn template_includes_cost_block() {
        assert!(TEMPLATE.contains("[cost]"));
        assert!(TEMPLATE.contains("show_session"));
        assert!(TEMPLATE.contains("show_daily"));
        assert!(TEMPLATE.contains("ttl_seconds"));
        assert!(TEMPLATE.contains("breakdown_min_usd"));
    }

    #[test]
    fn cost_values_are_read_from_toml() {
        let (c, _) = from_toml_str(
            "[cost]\nshow_session = false\nshow_daily = false\n\
             ttl_seconds = 30\nbreakdown_min_usd = 0.05\n",
        );
        assert!(!c.cost.show_session);
        assert!(!c.cost.show_daily);
        assert_eq!(c.cost.ttl_seconds, 30);
        assert!((c.cost.breakdown_min_usd - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_ttl_seconds_zero_is_honored_not_clamped() {
        let (c, _) = from_toml_str("[cost]\nttl_seconds = 0\n");
        assert_eq!(
            c.cost.ttl_seconds, 0,
            "0 is the documented \"disable\" value"
        );
    }

    #[test]
    fn cost_negative_ttl_falls_back_to_default() {
        let (c, _) = from_toml_str("[cost]\nttl_seconds = -5\n");
        assert_eq!(c.cost.ttl_seconds, CostConfig::default().ttl_seconds);
    }

    #[test]
    fn cost_negative_breakdown_min_falls_back_to_default() {
        let (c, _) = from_toml_str("[cost]\nbreakdown_min_usd = -1.0\n");
        assert!(
            (c.cost.breakdown_min_usd - CostConfig::default().breakdown_min_usd).abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn cost_partial_block_fills_in_defaults() {
        let (c, _) = from_toml_str("[cost]\nshow_daily = false\n");
        assert!(!c.cost.show_daily);
        assert!(c.cost.show_session);
        assert_eq!(c.cost.ttl_seconds, 90);
    }
}
