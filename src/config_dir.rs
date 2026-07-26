use std::env;
use std::path::PathBuf;

fn non_empty(raw: Option<&str>) -> Option<&str> {
    raw.filter(|v| !v.is_empty())
}

/// Precedence: `$CLAUDE_CONFIG_DIR`, then the config file's
/// `claude.config_dir`, then `$HOME/.claude`.
///
/// Environment beats file deliberately — inverting it would silently stop
/// an exported `CLAUDE_CONFIG_DIR` from working for existing users.
fn resolve(env_value: Option<&str>, file_value: Option<&str>, home: Option<&str>) -> PathBuf {
    if let Some(dir) = non_empty(env_value) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = non_empty(file_value) {
        return PathBuf::from(dir);
    }
    PathBuf::from(home.unwrap_or_default()).join(".claude")
}

/// Resolves Claude Code's config directory.
pub fn claude_config_dir(file_override: Option<&str>) -> PathBuf {
    resolve(
        env::var("CLAUDE_CONFIG_DIR").ok().as_deref(),
        file_override,
        env::var("HOME").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_override_used_when_env_is_absent() {
        assert_eq!(
            resolve(None, Some("/from/config"), Some("/home/u")),
            PathBuf::from("/from/config")
        );
    }

    #[test]
    fn env_beats_the_file() {
        assert_eq!(
            resolve(Some("/from/env"), Some("/from/config"), Some("/home/u")),
            PathBuf::from("/from/env"),
            "an exported CLAUDE_CONFIG_DIR must keep winning for existing users"
        );
    }

    #[test]
    fn empty_values_are_treated_as_unset_at_every_layer() {
        assert_eq!(
            resolve(Some(""), Some("/from/config"), Some("/home/u")),
            PathBuf::from("/from/config")
        );
        assert_eq!(
            resolve(Some(""), Some(""), Some("/home/u")),
            PathBuf::from("/home/u/.claude")
        );
    }

    #[test]
    fn falls_back_to_home_dot_claude() {
        assert_eq!(
            resolve(None, None, Some("/home/u")),
            PathBuf::from("/home/u/.claude")
        );
    }
}
