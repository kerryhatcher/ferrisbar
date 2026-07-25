use crate::config::Config;
use crate::paths;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off,
    Warn,
    Debug,
}

impl Level {
    /// Unknown values fall back to `Warn` rather than erroring — a typo in
    /// the level should leave the user with the safe default, not silence.
    pub fn from_str_lenient(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Self::Off,
            "debug" => Self::Debug,
            _ => Self::Warn,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Debug => "debug",
        }
    }
}

pub struct Event {
    pub level: Level,
    pub name: &'static str,
    pub session_id: Option<String>,
    pub msg: String,
}

#[allow(dead_code)] // only called from tests; will be used by main.rs
pub fn warn(name: &'static str, msg: impl Into<String>) -> Event {
    Event {
        level: Level::Warn,
        name,
        session_id: None,
        msg: msg.into(),
    }
}

#[allow(dead_code)] // only called from tests; will be used by main.rs
pub fn debug(name: &'static str, msg: impl Into<String>) -> Event {
    Event {
        level: Level::Debug,
        name,
        session_id: None,
        msg: msg.into(),
    }
}

/// Timestamp is a parameter so serialization is testable without a clock.
pub fn line_for(event: &Event, ts_millis: u128) -> String {
    let mut map = serde_json::Map::new();
    map.insert("ts".to_string(), serde_json::json!(ts_millis));
    map.insert("level".to_string(), serde_json::json!(event.level.as_str()));
    map.insert("event".to_string(), serde_json::json!(event.name));
    if let Some(id) = &event.session_id {
        map.insert("session_id".to_string(), serde_json::json!(id));
    }
    map.insert("msg".to_string(), serde_json::json!(event.msg));
    serde_json::Value::Object(map).to_string()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[allow(dead_code)] // fields max_size_bytes and max_archives are read by Task 6
pub struct Logger {
    path: Option<PathBuf>,
    level: Level,
    max_size_bytes: u64,
    max_archives: u8,
}

impl Logger {
    #[allow(dead_code)] // only called from tests; will be constructed by main.rs in Task 8
    pub fn new(cfg: &Config, data_dir: Option<&Path>) -> Self {
        let level = Level::from_str_lenient(&cfg.log.level);
        let path = if cfg.log.enabled && level != Level::Off {
            resolve_log_path(&cfg.log.path, data_dir)
        } else {
            None
        };
        Self {
            path,
            level,
            max_size_bytes: cfg.log.max_size_bytes,
            max_archives: cfg.log.max_archives,
        }
    }

    /// Never returns an error and never panics. Every failure — no data
    /// directory, unwritable path, full disk — silently disables this write
    /// so the statusline still renders.
    #[allow(dead_code)] // only called from tests; will be used by main.rs in Task 8
    pub fn log(&self, event: &Event) {
        let Some(path) = &self.path else { return };
        if self.level == Level::Off || event.level > self.level {
            return;
        }
        let _ = self.append(path, event);
    }

    #[allow(clippy::unused_self)] // self will be used for file locking in Task 6
    fn append(&self, path: &Path, event: &Event) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{}", line_for(event, now_millis()))
    }
}

/// An empty configured path means "use the default under the data dir". A
/// relative path resolves against the data dir, never the process working
/// directory — the cwd is whatever project Claude Code is running in, and
/// resolving there would scatter log files across repositories.
fn resolve_log_path(configured: &str, data_dir: Option<&Path>) -> Option<PathBuf> {
    if configured.is_empty() {
        return data_dir.map(paths::default_log_path);
    }
    let candidate = Path::new(configured);
    if candidate.is_absolute() {
        return Some(candidate.to_path_buf());
    }
    data_dir.map(|d| d.join(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn read_lines(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn level_parses_leniently_and_defaults_to_warn() {
        assert_eq!(Level::from_str_lenient("off"), Level::Off);
        assert_eq!(Level::from_str_lenient("warn"), Level::Warn);
        assert_eq!(Level::from_str_lenient("debug"), Level::Debug);
        assert_eq!(Level::from_str_lenient("DEBUG"), Level::Debug);
        assert_eq!(Level::from_str_lenient("nonsense"), Level::Warn);
        assert_eq!(Level::from_str_lenient(""), Level::Warn);
    }

    #[test]
    fn line_is_one_json_object_with_the_expected_fields() {
        let event = warn("stdin_parse_failed", "expected value");
        let line = line_for(&event, 1_753_467_296_123);

        assert!(!line.contains('\n'), "a JSONL record is exactly one line");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["ts"], 1_753_467_296_123_u64);
        assert_eq!(v["level"], "warn");
        assert_eq!(v["event"], "stdin_parse_failed");
        assert_eq!(v["msg"], "expected value");
        assert!(v.get("session_id").is_none(), "omitted when unknown");
    }

    #[test]
    fn line_includes_session_id_when_present() {
        let mut event = warn("todo_file_unreadable", "no such file");
        event.session_id = Some("abc123".to_string());
        let v: serde_json::Value = serde_json::from_str(&line_for(&event, 1)).unwrap();
        assert_eq!(v["session_id"], "abc123");
    }

    #[test]
    fn disabled_logging_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.enabled = false;

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(
            !dir.path().join("logs").exists(),
            "no directory is created either"
        );
    }

    #[test]
    fn level_off_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.level = "off".to_string();

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(read_lines(&crate::paths::default_log_path(dir.path())).is_empty());
    }

    #[test]
    fn warn_level_suppresses_debug_events_but_keeps_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let logger = Logger::new(&Config::default(), Some(dir.path()));

        logger.log(&debug("render", "should not appear"));
        logger.log(&warn("stdin_parse_failed", "should appear"));

        let lines = read_lines(&crate::paths::default_log_path(dir.path()));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("stdin_parse_failed"));
    }

    #[test]
    fn debug_level_keeps_both() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.level = "debug".to_string();
        let logger = Logger::new(&cfg, Some(dir.path()));

        logger.log(&debug("render", "a"));
        logger.log(&warn("stdin_parse_failed", "b"));

        assert_eq!(
            read_lines(&crate::paths::default_log_path(dir.path())).len(),
            2
        );
    }

    #[test]
    fn appends_rather_than_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let logger = Logger::new(&Config::default(), Some(dir.path()));

        logger.log(&warn("stdin_parse_failed", "first"));
        logger.log(&warn("stdin_read_failed", "second"));

        let lines = read_lines(&crate::paths::default_log_path(dir.path()));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("first"));
        assert!(lines[1].contains("second"));
    }

    #[test]
    fn explicit_relative_path_resolves_against_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.log.path = "custom.jsonl".to_string();

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(
            dir.path().join("custom.jsonl").exists(),
            "relative paths must not resolve against the process cwd, \
             which is whatever project Claude Code is running in"
        );
    }

    #[test]
    fn explicit_absolute_path_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.jsonl");
        let mut cfg = Config::default();
        cfg.log.path = target.to_string_lossy().into_owned();

        let logger = Logger::new(&cfg, Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        assert!(target.exists());
    }

    #[test]
    fn no_data_dir_disables_logging_without_erroring() {
        let logger = Logger::new(&Config::default(), None);
        logger.log(&warn("stdin_parse_failed", "x"));
        // Reaching this line without a panic is the assertion.
    }
}
