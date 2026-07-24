use crate::config_dir;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn resolve_settings_path(project_scope: bool) -> Result<PathBuf, String> {
    if project_scope {
        let cwd = env::current_dir()
            .map_err(|e| format!("failed to determine the current directory: {e}"))?;
        return Ok(cwd.join(".claude").join("settings.local.json"));
    }

    let has_config_dir = env::var("CLAUDE_CONFIG_DIR").is_ok_and(|v| !v.is_empty());
    let has_home = env::var("HOME").is_ok_and(|v| !v.is_empty());
    if !has_config_dir && !has_home {
        return Err(
            "Cannot resolve the Claude Code config directory: neither $CLAUDE_CONFIG_DIR nor $HOME is set."
                .to_string(),
        );
    }
    Ok(config_dir::claude_config_dir().join("settings.json"))
}

fn apply_statusline_update(
    settings_path: &Path,
    new_command: &str,
) -> Result<Option<String>, String> {
    let existing = match fs::read_to_string(settings_path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(e) => return Err(format!("failed to read {}: {e}", settings_path.display())),
    };

    let mut root: Value = serde_json::from_str(&existing)
        .map_err(|e| format!("{} contains invalid JSON: {e}", settings_path.display()))?;

    let map = root.as_object_mut().ok_or_else(|| {
        format!(
            "{} does not contain a JSON object at its root",
            settings_path.display()
        )
    })?;

    let previous = map
        .get("statusLine")
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    map.insert(
        "statusLine".to_string(),
        json!({ "type": "command", "command": new_command }),
    );

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    let serialized = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    fs::write(settings_path, serialized)
        .map_err(|e| format!("failed to write {}: {e}", settings_path.display()))?;

    Ok(previous)
}

pub fn run(project_scope: bool) -> Result<(), String> {
    let settings_path = resolve_settings_path(project_scope)?;
    let new_command = env::current_exe()
        .map_err(|e| format!("failed to resolve the current executable path: {e}"))?
        .to_string_lossy()
        .into_owned();

    let previous = apply_statusline_update(&settings_path, &new_command)?;

    println!("Updated statusLine in {}", settings_path.display());
    match previous {
        Some(before) => println!("  before: {before}"),
        None => println!("  before: (none)"),
    }
    println!("  after:  {new_command}");
    println!("Start a new Claude Code session for the change to take effect.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_settings_file_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let previous = apply_statusline_update(&path, "/usr/local/bin/mystatusline").unwrap();

        assert_eq!(previous, None);
        let contents = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            value["statusLine"],
            json!({"type": "command", "command": "/usr/local/bin/mystatusline"})
        );
    }

    #[test]
    fn preserves_unrelated_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"env":{"FOO":"bar"},"theme":"dark"}"#).unwrap();

        apply_statusline_update(&path, "/bin/mystatusline").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["env"]["FOO"], "bar");
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["statusLine"]["command"], "/bin/mystatusline");
    }

    #[test]
    fn captures_previous_statusline_command() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"statusLine":{"type":"command","command":"/old/path"}}"#,
        )
        .unwrap();

        let previous = apply_statusline_update(&path, "/new/path").unwrap();

        assert_eq!(previous, Some("/old/path".to_string()));
    }

    #[test]
    fn rejects_invalid_json_without_modifying_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = "{not valid json";
        fs::write(&path, original).unwrap();

        let result = apply_statusline_update(&path, "/bin/mystatusline");

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn rejects_non_object_root_without_modifying_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = "[1, 2, 3]";
        fs::write(&path, original).unwrap();

        let result = apply_statusline_update(&path, "/bin/mystatusline");

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn preserves_key_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"zeta":1,"alpha":2,"mid":3}"#).unwrap();

        apply_statusline_update(&path, "/bin/mystatusline").unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let zeta_pos = contents.find("zeta").unwrap();
        let alpha_pos = contents.find("alpha").unwrap();
        let mid_pos = contents.find("mid").unwrap();
        assert!(zeta_pos < alpha_pos);
        assert!(alpha_pos < mid_pos);
    }
}
