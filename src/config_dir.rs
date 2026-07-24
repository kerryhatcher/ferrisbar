use std::env;
use std::path::PathBuf;

/// Resolves Claude Code's config directory: `$CLAUDE_CONFIG_DIR` if set and
/// non-empty, else `$HOME/.claude`.
pub fn claude_config_dir() -> PathBuf {
    match env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home = env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".claude")
        }
    }
}
