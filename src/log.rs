use crate::config::Config;
use crate::paths;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

pub fn warn(name: &'static str, msg: impl Into<String>) -> Event {
    Event {
        level: Level::Warn,
        name,
        session_id: None,
        msg: msg.into(),
    }
}

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

pub struct Logger {
    path: Option<PathBuf>,
    level: Level,
    max_size_bytes: u64,
    max_archives: u8,
}

impl Logger {
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
    pub fn log(&self, event: &Event) {
        let Some(path) = &self.path else { return };
        if self.level == Level::Off || event.level > self.level {
            return;
        }
        let _ = self.append(path, event);
    }

    fn append(&self, path: &Path, event: &Event) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let rotate_error = self.rotate_if_needed(path);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        // Written inline rather than via `log`, which would recurse back
        // into rotation. The file is known to be over its limit here, so
        // this line is the one permitted overshoot.
        if let Some(msg) = rotate_error {
            let event = warn("log_rotate_failed", msg);
            writeln!(file, "{}", line_for(&event, now_millis()))?;
        }
        writeln!(file, "{}", line_for(event, now_millis()))
    }

    /// Ordering is load-bearing: lock, then stat, then rotate. The caller
    /// opens the log only after this returns, so no descriptor can outlive
    /// the rename.
    ///
    /// Returns a message when rotation was attempted and failed. Deferring
    /// the report to the caller keeps this function from calling `log`,
    /// which would re-enter rotation.
    fn rotate_if_needed(&self, path: &Path) -> Option<String> {
        let dir = path.parent()?;
        let over_limit = std::fs::metadata(path).is_ok_and(|m| m.len() >= self.max_size_bytes);
        if !over_limit {
            return None;
        }
        // Lock held for the whole rotation; released on drop.
        //
        // The `?` here returns None, meaning "nothing to report". Losing
        // the lock race is a deferral, not a failure — another process is
        // rotating and this one simply appends. Do not "fix" this into
        // `return Some("lock unavailable")`: that would emit a
        // log_rotate_failed line on every render of every losing process.
        let _guard = acquire_lock(dir)?;

        // Re-check under the lock: another process may have just rotated.
        if !std::fs::metadata(path).is_ok_and(|m| m.len() >= self.max_size_bytes) {
            return None;
        }

        // Shift downward from the oldest, so .N.gz is overwritten rather
        // than accumulating past max_archives.
        for n in (1..self.max_archives).rev() {
            let _ = std::fs::rename(archive_path(path, n), archive_path(path, n + 1));
        }

        let staged = path.with_extension("rotating");
        if let Err(e) = std::fs::rename(path, &staged) {
            return Some(format!("staging rename failed: {e}"));
        }
        if let Err(e) = gzip_into(&staged, &archive_path(path, 1)) {
            // Put it back rather than losing the records outright.
            let _ = std::fs::rename(&staged, path);
            return Some(format!("gzip failed: {e}"));
        }
        let _ = std::fs::remove_file(&staged);
        None
    }
}

const LOCK_STALE_AFTER: Duration = Duration::from_secs(60);

pub fn archive_path(base: &Path, n: u8) -> PathBuf {
    let mut name = base.as_os_str().to_os_string();
    name.push(format!(".{n}.gz"));
    PathBuf::from(name)
}

/// Removes the lock file on drop so a rotation that fails partway through
/// does not block every later rotation for 60 seconds.
struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `create_new` is an atomic `O_EXCL` create: exactly one concurrent
/// process gets the lock.
///
/// A lock older than `LOCK_STALE_AFTER` is assumed to be from a process
/// that died mid-rotation and is reclaimed. Two processes can both judge
/// the same lock stale and both proceed; the consequence is a dropped
/// archive generation rather than corruption, and it is accepted rather
/// than engineered around.
fn acquire_lock(dir: &Path) -> Option<LockGuard> {
    let path = dir.join(".rotate.lock");
    match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(_) => return Some(LockGuard(path)),
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return None,
        Err(_) => {}
    }

    let stale = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .and_then(|t| {
            SystemTime::now()
                .duration_since(t)
                .map_err(|_| std::io::Error::other("clock went backwards"))
        })
        .is_ok_and(|age| age > LOCK_STALE_AFTER);

    if !stale {
        return None;
    }
    std::fs::remove_file(&path).ok()?;
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .ok()
        .map(|_| LockGuard(path))
}

fn gzip_into(source: &Path, dest: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(source)?;
    let mut encoder = GzEncoder::new(std::fs::File::create(dest)?, Compression::default());
    std::io::copy(&mut input, &mut encoder)?;
    encoder.finish()?;
    Ok(())
}

/// An empty configured path means "use the default under the data dir". A
/// relative path resolves against the data dir, never the process working
/// directory — the cwd is whatever project Claude Code is running in, and
/// resolving there would scatter log files across repositories.
pub fn resolve_log_path(configured: &str, data_dir: Option<&Path>) -> Option<PathBuf> {
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

    /// Shells out to `id -u` rather than an `unsafe extern "C" geteuid`
    /// binding — this is test-only code and the project otherwise has zero
    /// unsafe in its dependency-light build.
    #[cfg(unix)]
    fn running_as_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .is_some_and(|s| s.trim() == "0")
    }

    /// Unix-only: Windows has no equivalent of a mode-based read-only
    /// directory reachable from a normal test process. Skipped rather than
    /// asserted-vacuously when running as root, since root bypasses
    /// directory permission checks entirely and the write would succeed.
    #[cfg(unix)]
    #[test]
    fn read_only_log_directory_disables_logging_without_erroring() {
        use std::os::unix::fs::PermissionsExt;

        if running_as_root() {
            eprintln!("skipping: running as root, permission checks do not apply");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        let original_mode = std::fs::metadata(&logs_dir).unwrap().permissions().mode();
        std::fs::set_permissions(&logs_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let logger = Logger::new(&Config::default(), Some(dir.path()));
        logger.log(&warn("stdin_parse_failed", "x"));

        // Restore before the tempdir is cleaned up.
        std::fs::set_permissions(&logs_dir, std::fs::Permissions::from_mode(original_mode))
            .unwrap();

        let base = crate::paths::default_log_path(dir.path());
        assert!(
            !base.exists(),
            "a read-only log directory must not produce a log file, and must not panic"
        );
    }

    use std::io::Read as _;

    fn tiny_logger(dir: &std::path::Path, max_archives: u8) -> Logger {
        let mut cfg = Config::default();
        cfg.log.max_size_bytes = 4096;
        cfg.log.max_archives = max_archives;
        Logger::new(&cfg, Some(dir))
    }

    fn gunzip(path: &std::path::Path) -> String {
        let file = std::fs::File::open(path).unwrap();
        let mut out = String::new();
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut out)
            .unwrap();
        out
    }

    #[test]
    fn no_rotation_below_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 7);
        let base = crate::paths::default_log_path(dir.path());

        logger.log(&warn("stdin_parse_failed", "small"));

        assert!(base.exists());
        assert!(
            !archive_path(&base, 1).exists(),
            "must not rotate under the limit"
        );
    }

    #[test]
    fn rotation_archives_the_full_file_and_starts_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 7);
        let base = crate::paths::default_log_path(dir.path());

        let filler = "x".repeat(5000);
        logger.log(&warn("stdin_parse_failed", filler.clone()));
        logger.log(&warn("stdin_read_failed", "after rotation"));

        assert!(archive_path(&base, 1).exists(), "archive .1.gz must exist");
        let current = std::fs::read_to_string(&base).unwrap();
        assert!(current.contains("after rotation"));
        assert!(
            !current.contains(&filler),
            "the big line moved into the archive"
        );
    }

    #[test]
    fn archive_decompresses_to_exactly_the_bytes_written() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 7);
        let base = crate::paths::default_log_path(dir.path());

        logger.log(&warn("stdin_parse_failed", "y".repeat(5000)));
        let before_rotation = std::fs::read_to_string(&base).unwrap();
        logger.log(&warn("stdin_read_failed", "trigger"));

        assert_eq!(gunzip(&archive_path(&base, 1)), before_rotation);
    }

    #[test]
    fn archives_shift_and_cap_at_max_archives() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 2);
        let base = crate::paths::default_log_path(dir.path());

        for i in 0..4 {
            logger.log(&warn(
                "stdin_parse_failed",
                format!("gen{i}{}", "z".repeat(5000)),
            ));
        }

        assert!(archive_path(&base, 1).exists());
        assert!(archive_path(&base, 2).exists());
        assert!(
            !archive_path(&base, 3).exists(),
            "oldest generation is dropped"
        );
        assert!(
            gunzip(&archive_path(&base, 1)).contains("gen2"),
            ".1.gz always holds the most recent archive"
        );
    }

    #[test]
    fn a_held_lock_defers_rotation_without_losing_the_line() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 7);
        let base = crate::paths::default_log_path(dir.path());

        logger.log(&warn("stdin_parse_failed", "w".repeat(5000)));
        // Simulate another live process mid-rotation.
        let lock = base.parent().unwrap().join(".rotate.lock");
        std::fs::write(&lock, "").unwrap();

        logger.log(&warn("stdin_read_failed", "deferred"));

        assert!(!archive_path(&base, 1).exists(), "loser must not rotate");
        assert!(
            std::fs::read_to_string(&base).unwrap().contains("deferred"),
            "loser still appends — the line is never dropped"
        );
    }

    #[test]
    fn a_stale_lock_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 7);
        let base = crate::paths::default_log_path(dir.path());

        logger.log(&warn("stdin_parse_failed", "v".repeat(5000)));
        let lock = base.parent().unwrap().join(".rotate.lock");
        std::fs::write(&lock, "").unwrap();
        let stale =
            filetime::FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() - 120, 0);
        filetime::set_file_mtime(&lock, stale).unwrap();

        logger.log(&warn("stdin_read_failed", "after reclaim"));

        assert!(
            archive_path(&base, 1).exists(),
            "a lock older than 60s is reclaimed"
        );
    }

    /// Unix-only: the test wedges rotation by making the archive paths
    /// directories, and `rename`-onto-non-empty-directory and
    /// `File::create`-on-a-directory have different error semantics on
    /// Windows. The production code path is platform-independent; only this
    /// way of provoking a failure is not.
    #[cfg(unix)]
    #[test]
    fn a_failed_rotation_is_reported_and_the_records_survive() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 3);
        let base = crate::paths::default_log_path(dir.path());

        logger.log(&warn("stdin_parse_failed", "t".repeat(5000)));

        // .1.gz is a directory, so gzip cannot create it. .2.gz and .3.gz are
        // non-empty directories, so the shift loop cannot move .1.gz out of
        // the way first.
        std::fs::create_dir_all(archive_path(&base, 1)).unwrap();
        std::fs::create_dir_all(archive_path(&base, 2).join("occupied")).unwrap();
        std::fs::create_dir_all(archive_path(&base, 3).join("occupied")).unwrap();

        logger.log(&warn("stdin_read_failed", "trigger"));

        let current = std::fs::read_to_string(&base).unwrap();
        assert!(
            current.contains("log_rotate_failed"),
            "the failure is reported"
        );
        assert!(
            current.contains("trigger"),
            "the triggering line is still written"
        );
        assert!(
            current.contains("ttttt"),
            "the staged file was restored, not lost"
        );
        assert!(
            !base.parent().unwrap().join(".rotate.lock").exists(),
            "the guard releases the lock even on the failure path"
        );
    }

    #[test]
    fn the_lock_is_released_after_a_successful_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let logger = tiny_logger(dir.path(), 7);
        let base = crate::paths::default_log_path(dir.path());

        logger.log(&warn("stdin_parse_failed", "u".repeat(5000)));
        logger.log(&warn("stdin_read_failed", "trigger"));

        assert!(
            !base.parent().unwrap().join(".rotate.lock").exists(),
            "a leaked lock would block rotation for the next 60 seconds"
        );
    }
}
