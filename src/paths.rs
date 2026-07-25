use std::path::{Path, PathBuf};

const APP_DIR: &str = "ferrisbar";

/// Accepts a directory only when it is present, non-empty, and absolute.
///
/// The absolute check is what keeps a stray `./ferrisbar/` from being
/// created inside whatever repository Claude Code happens to be running in
/// — see the empty-HOME guard in the design spec.
fn usable_dir(raw: Option<&str>) -> Option<PathBuf> {
    let raw = raw?;
    if raw.is_empty() {
        return None;
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn platform_dir(base: Option<&str>, _xdg: Option<&str>, _unix_fallback: &str) -> Option<PathBuf> {
    Some(
        usable_dir(base)?
            .join("Library")
            .join("Application Support")
            .join(APP_DIR),
    )
}

#[cfg(windows)]
fn platform_dir(base: Option<&str>, _xdg: Option<&str>, _unix_fallback: &str) -> Option<PathBuf> {
    Some(usable_dir(base)?.join(APP_DIR))
}

#[cfg(all(not(target_os = "macos"), not(windows)))]
fn platform_dir(base: Option<&str>, xdg: Option<&str>, unix_fallback: &str) -> Option<PathBuf> {
    if let Some(dir) = usable_dir(xdg) {
        return Some(dir.join(APP_DIR));
    }
    let mut path = usable_dir(base)?;
    for part in unix_fallback.split('/') {
        path.push(part);
    }
    Some(path.join(APP_DIR))
}

/// `base` is `$HOME` on Unix and `%APPDATA%` on Windows. `xdg` is
/// `$XDG_CONFIG_HOME` and is consulted only on the XDG branch.
pub fn resolve_config_dir(base: Option<&str>, xdg: Option<&str>) -> Option<PathBuf> {
    platform_dir(base, xdg, ".config")
}

/// `base` is `$HOME` on Unix and `%LOCALAPPDATA%` on Windows. `xdg` is
/// `$XDG_DATA_HOME` and is consulted only on the XDG branch.
pub fn resolve_data_dir(base: Option<&str>, xdg: Option<&str>) -> Option<PathBuf> {
    platform_dir(base, xdg, ".local/share")
}

// Public API consumed by task 5 logging setup.
#[allow(dead_code)]
pub fn default_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("ferrisbar.jsonl")
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

#[cfg(windows)]
fn base_vars() -> (Option<String>, Option<String>) {
    (env_opt("APPDATA"), env_opt("LOCALAPPDATA"))
}

#[cfg(not(windows))]
fn base_vars() -> (Option<String>, Option<String>) {
    let home = env_opt("HOME");
    (home.clone(), home)
}

/// `<config dir>/config.toml`, or `None` when the platform base directory
/// is unavailable.
// Public API consumed by task 4 config loading.
#[allow(dead_code)]
pub fn config_file() -> Option<PathBuf> {
    let (config_base, _) = base_vars();
    resolve_config_dir(
        config_base.as_deref(),
        env_opt("XDG_CONFIG_HOME").as_deref(),
    )
    .map(|d| d.join("config.toml"))
}

/// The data directory, or `None` when the platform base is unavailable.
// Public API consumed by task 8 logging wire-up.
#[allow(dead_code)]
pub fn data_dir() -> Option<PathBuf> {
    let (_, data_base) = base_vars();
    resolve_data_dir(data_base.as_deref(), env_opt("XDG_DATA_HOME").as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_dir_rejects_none_empty_and_relative() {
        assert_eq!(usable_dir(None), None);
        assert_eq!(usable_dir(Some("")), None);
        assert_eq!(usable_dir(Some("relative/path")), None);
    }

    #[test]
    fn usable_dir_accepts_absolute() {
        assert!(usable_dir(Some("/home/someone")).is_some());
    }

    #[test]
    fn config_dir_none_when_base_missing_or_empty() {
        assert_eq!(resolve_config_dir(None, None), None);
        assert_eq!(resolve_config_dir(Some(""), None), None);
        assert_eq!(resolve_data_dir(None, None), None);
        assert_eq!(resolve_data_dir(Some(""), None), None);
    }

    #[test]
    fn dirs_end_in_app_name() {
        let cfg = resolve_config_dir(Some("/base"), None).unwrap();
        let data = resolve_data_dir(Some("/base"), None).unwrap();
        assert_eq!(cfg.file_name().unwrap(), APP_DIR);
        assert_eq!(data.file_name().unwrap(), APP_DIR);
    }

    #[test]
    fn dirs_are_absolute() {
        assert!(resolve_config_dir(Some("/base"), None)
            .unwrap()
            .is_absolute());
        assert!(resolve_data_dir(Some("/base"), None).unwrap().is_absolute());
    }

    #[test]
    fn default_log_path_is_under_logs() {
        let p = default_log_path(Path::new("/data/ferrisbar"));
        assert_eq!(p, PathBuf::from("/data/ferrisbar/logs/ferrisbar.jsonl"));
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    #[test]
    fn xdg_honored_when_absolute_and_ignored_when_relative() {
        assert_eq!(
            resolve_config_dir(Some("/home/u"), Some("/xdg/cfg")),
            Some(PathBuf::from("/xdg/cfg/ferrisbar"))
        );
        assert_eq!(
            resolve_config_dir(Some("/home/u"), Some("rel")),
            Some(PathBuf::from("/home/u/.config/ferrisbar"))
        );
        assert_eq!(
            resolve_config_dir(Some("/home/u"), Some("")),
            Some(PathBuf::from("/home/u/.config/ferrisbar"))
        );
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    #[test]
    fn linux_fallbacks() {
        assert_eq!(
            resolve_config_dir(Some("/home/u"), None),
            Some(PathBuf::from("/home/u/.config/ferrisbar"))
        );
        assert_eq!(
            resolve_data_dir(Some("/home/u"), None),
            Some(PathBuf::from("/home/u/.local/share/ferrisbar"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_application_support_for_both() {
        let expected = PathBuf::from("/Users/u/Library/Application Support/ferrisbar");
        assert_eq!(
            resolve_config_dir(Some("/Users/u"), None),
            Some(expected.clone())
        );
        assert_eq!(
            resolve_data_dir(Some("/Users/u"), Some("/xdg")),
            Some(expected)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_base_directly_and_ignores_xdg() {
        assert_eq!(
            resolve_config_dir(Some(r"C:\Users\u\AppData\Roaming"), Some(r"C:\xdg")),
            Some(PathBuf::from(r"C:\Users\u\AppData\Roaming\ferrisbar"))
        );
    }
}
