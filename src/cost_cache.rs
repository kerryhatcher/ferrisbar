//! On-disk cache for the daily cost aggregate, and the detached self-exec
//! refresh that keeps it warm without ever blocking a render.
//!
//! Aggregating every transcript is too slow to do on the render path, so
//! `cost::daily_chip` reads whatever is here (even if stale) and, when it is
//! older than the configured TTL, calls `spawn_refresh` to kick off a
//! background re-invocation of this same binary with the hidden
//! `--internal-refresh-daily-cost` flag. That child computes the real
//! numbers and calls `write_cache`/`release_lock` itself — see `cost.rs`'s
//! `refresh_daily_cache`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
pub struct CachePayload {
    /// UTC calendar date (`YYYY-MM-DD`) the totals below cover.
    pub date: String,
    pub total_usd: f64,
    pub by_model: Vec<(String, f64)>,
}

/// A lock older than this is assumed to belong to a refresh that hung or
/// crashed, and may be reclaimed by a later render. Set above the
/// background refresh's own expected running time.
const LOCK_STALE_AFTER: Duration = Duration::from_secs(75);

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cost-cache.json")
}

fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cost-cache.lock")
}

/// The cached payload plus its age, or `None` on any failure — missing
/// file, unreadable, or malformed JSON. A degraded read is treated as "no
/// cache yet", never a fault to report.
pub fn read_cache(data_dir: &Path) -> Option<(CachePayload, Duration)> {
    let path = cache_path(data_dir);
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    let contents = std::fs::read_to_string(&path).ok()?;
    let payload = serde_json::from_str(&contents).ok()?;
    Some((payload, age))
}

/// Atomic write (temp file + rename) so a render reading the cache mid-write
/// never sees a truncated file.
pub fn write_cache(data_dir: &Path, payload: &CachePayload) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = cache_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string(payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, body)?;
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(first_err) => {
            // A rename that fails to replace an existing destination (seen
            // on Windows when another process has it open) gets one retry
            // after removing the destination outright — losing that race
            // just means a render sees no cache for one cycle, not a
            // corrupt one, since the temp file was already fully written.
            let _ = std::fs::remove_file(&path);
            std::fs::rename(&tmp, &path).map_err(|_| first_err)
        }
    }
}

pub fn release_lock(data_dir: &Path) {
    let _ = std::fs::remove_file(lock_path(data_dir));
}

/// Fires a detached, non-blocking re-invocation of this binary
/// (`--internal-refresh-daily-cost`) unless a fresh lock shows one is
/// already in flight. Every failure — no lock directory, no current-exe
/// path, spawn error — is swallowed: a refresh that cannot start this
/// render just means the next stale render tries again.
///
/// The lock is acquired the same way `log.rs`'s rotation lock is: an atomic
/// `create_new` first, and only on `AlreadyExists` do we check whether the
/// existing lock is stale enough to reclaim. A plain overwrite here (the
/// previous approach) let two renders that both observed a fresh lock as
/// "not stale yet" both write over it and spawn duplicate refreshes;
/// `create_new` means only one of two concurrent callers can ever hold the
/// lock at a time. Two renders can still both judge the *same stale* lock
/// reclaimable and both proceed — accepted the same way `log.rs` accepts it
/// (a dropped generation, not corruption) rather than engineered around.
pub fn spawn_refresh(data_dir: &Path) {
    if std::fs::create_dir_all(data_dir).is_err() {
        return;
    }
    let lock = lock_path(data_dir);
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&lock)
    {
        Ok(_) => {}
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return,
        Err(_) => {
            let stale = std::fs::metadata(&lock)
                .and_then(|m| m.modified())
                .map(|t| SystemTime::now().duration_since(t).unwrap_or(Duration::MAX))
                .is_ok_and(|age| age >= LOCK_STALE_AFTER);
            if !stale || std::fs::remove_file(&lock).is_err() {
                return; // a refresh is already in flight, or reclaiming it failed
            }
            if std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock)
                .is_err()
            {
                return; // another render reclaimed it first
            }
        }
    }

    let Ok(exe) = std::env::current_exe() else {
        release_lock(data_dir);
        return;
    };
    let mut cmd = Command::new(exe);
    cmd.arg("--internal-refresh-daily-cost")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Detaches the child from this process's session/console, so it keeps
    // running (and the render can exit) even if the terminal that invoked
    // the statusline closes immediately after.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }

    if cmd.spawn().is_err() {
        release_lock(data_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> CachePayload {
        CachePayload {
            date: "2026-08-10".to_string(),
            total_usd: 1.23,
            by_model: vec![("claude-sonnet-5".to_string(), 1.23)],
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        write_cache(dir.path(), &payload()).unwrap();

        let (read_back, age) = read_cache(dir.path()).unwrap();
        assert_eq!(read_back, payload());
        assert!(age < Duration::from_secs(5));
    }

    #[test]
    fn read_cache_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_cache(dir.path()).is_none());
    }

    #[test]
    fn read_cache_malformed_json_is_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(cache_path(dir.path()), b"not json").unwrap();
        assert!(read_cache(dir.path()).is_none());
    }

    #[test]
    fn write_cache_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        write_cache(&nested, &payload()).unwrap();
        assert!(nested.join("cost-cache.json").exists());
    }

    #[test]
    fn write_cache_never_leaves_a_tmp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        write_cache(dir.path(), &payload()).unwrap();
        assert!(!dir.path().join("cost-cache.json.tmp").exists());
    }

    #[test]
    fn release_lock_missing_file_does_not_error() {
        let dir = tempfile::tempdir().unwrap();
        release_lock(dir.path()); // must not panic
    }

    #[test]
    fn spawn_refresh_writes_a_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        spawn_refresh(dir.path());
        assert!(lock_path(dir.path()).exists());
        release_lock(dir.path()); // cleanup; the spawned child may also race this
    }

    #[test]
    fn spawn_refresh_skips_when_a_fresh_lock_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), b"").unwrap();
        let before = std::fs::metadata(lock_path(dir.path()))
            .unwrap()
            .modified()
            .unwrap();

        spawn_refresh(dir.path());

        let after = std::fs::metadata(lock_path(dir.path()))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after, "an in-flight lock must not be rewritten");
    }

    #[test]
    fn spawn_refresh_reclaims_a_stale_lock() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(lock_path(dir.path()), b"").unwrap();
        let stale =
            filetime::FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() - 120, 0);
        filetime::set_file_mtime(lock_path(dir.path()), stale).unwrap();

        // Captured *before* the call, not read back after it returns: the
        // reclaim itself (remove + create_new) happens up front, but
        // `spawn_refresh` also launches a detached child process before
        // returning, and how long process creation takes is both
        // platform-dependent and, on a loaded CI runner (Windows especially),
        // slow enough to blow past a few seconds. Comparing the reclaimed
        // lock's mtime to a "now" read after that wait flaked for exactly
        // this reason; comparing it to a timestamp from before the call does
        // not, because it no longer depends on how long spawning took.
        let before = SystemTime::now();

        spawn_refresh(dir.path());

        let mtime = std::fs::metadata(lock_path(dir.path()))
            .unwrap()
            .modified()
            .unwrap();
        assert!(
            mtime + Duration::from_secs(1) >= before,
            "reclaimed lock must not still carry the stale (120s-old) mtime"
        );
        release_lock(dir.path());
    }
}
