use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Deserialize)]
struct TodoItem {
    status: Option<String>,
    #[serde(rename = "activeForm")]
    active_form: Option<String>,
    content: Option<String>,
}

/// Returns the active task's display text, if any, alongside a diagnostic
/// message for the caller to log.
///
/// The diagnostic is `Some` only when a todo file was matched for this
/// session but could not be read or parsed — a genuine, once-per-fault
/// condition. A missing `todos/` directory or no matching file is normal
/// (no session has run yet, or none is in progress) and produces `(None,
/// None)` silently.
pub fn active_task(session_id: &str, todos_dir: &Path) -> (Option<String>, Option<String>) {
    if session_id.is_empty() {
        return (None, None);
    }
    let Ok(entries) = fs::read_dir(todos_dir) else {
        return (None, None);
    };

    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(session_id) || !name.contains("-agent-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(mtime) = metadata.modified() else {
            continue;
        };
        if latest.as_ref().is_none_or(|(t, _)| mtime > *t) {
            latest = Some((mtime, entry.path()));
        }
    }

    let Some((_, path)) = latest else {
        return (None, None);
    };

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) => {
            return (
                None,
                Some(format!("could not read {}: {e}", path.display())),
            )
        }
    };
    let todos: Vec<TodoItem> = match serde_json::from_str(&content) {
        Ok(todos) => todos,
        Err(e) => {
            return (
                None,
                Some(format!("could not parse {}: {e}", path.display())),
            )
        }
    };
    let Some(in_progress) = todos
        .into_iter()
        .find(|t| t.status.as_deref() == Some("in_progress"))
    else {
        return (None, None);
    };

    let task = in_progress
        .active_form
        .filter(|s| !s.is_empty())
        .or(in_progress.content)
        .filter(|s| !s.is_empty());
    (task, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{set_file_mtime, FileTime};
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_todo_file(dir: &Path, name: &str, contents: &str, mtime_secs: i64) {
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        set_file_mtime(&path, FileTime::from_unix_time(mtime_secs, 0)).unwrap();
    }

    #[test]
    fn none_when_dir_missing() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(active_task("abc", &missing), (None, None));
    }

    #[test]
    fn none_when_session_id_empty() {
        let dir = tempdir().unwrap();
        assert_eq!(active_task("", dir.path()), (None, None));
    }

    #[test]
    fn none_when_no_matching_files() {
        let dir = tempdir().unwrap();
        write_todo_file(dir.path(), "other-session-agent-1.json", "[]", 1000);
        assert_eq!(active_task("abc", dir.path()), (None, None));
    }

    #[test]
    fn ignores_files_missing_agent_marker() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-notit.json",
            r#"[{"status":"in_progress","content":"x"}]"#,
            1000,
        );
        assert_eq!(active_task("abc", dir.path()), (None, None));
    }

    #[test]
    fn picks_newest_file_by_mtime() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-old.json",
            r#"[{"status":"in_progress","content":"old task"}]"#,
            1000,
        );
        write_todo_file(
            dir.path(),
            "abc-agent-new.json",
            r#"[{"status":"in_progress","content":"new task"}]"#,
            2000,
        );
        assert_eq!(
            active_task("abc", dir.path()),
            (Some("new task".to_string()), None)
        );
    }

    #[test]
    fn none_when_no_in_progress_entry() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.json",
            r#"[{"status":"completed","content":"done"}]"#,
            1000,
        );
        assert_eq!(active_task("abc", dir.path()), (None, None));
    }

    #[test]
    fn prefers_active_form_over_content() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.json",
            r#"[{"status":"in_progress","activeForm":"Doing thing","content":"do thing"}]"#,
            1000,
        );
        assert_eq!(
            active_task("abc", dir.path()),
            (Some("Doing thing".to_string()), None)
        );
    }

    #[test]
    fn falls_back_to_content_when_active_form_empty() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.json",
            r#"[{"status":"in_progress","activeForm":"","content":"do thing"}]"#,
            1000,
        );
        assert_eq!(
            active_task("abc", dir.path()),
            (Some("do thing".to_string()), None)
        );
    }

    #[test]
    fn malformed_json_in_a_matched_file_produces_a_diagnostic() {
        let dir = tempdir().unwrap();
        write_todo_file(dir.path(), "abc-agent-1.json", "not json", 1000);

        let (task, diagnostic) = active_task("abc", dir.path());

        assert_eq!(task, None);
        assert!(
            diagnostic.is_some(),
            "a matched-but-malformed file is a genuine fault, not silence"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_matched_file_produces_a_diagnostic() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.json",
            r#"[{"status":"in_progress","content":"x"}]"#,
            1000,
        );
        let path = dir.path().join("abc-agent-1.json");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let (task, diagnostic) = active_task("abc", dir.path());

        // Restore so the tempdir can be cleaned up.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(task, None);
        assert!(
            diagnostic.is_some(),
            "a matched file that cannot be read is a genuine fault, not silence"
        );
    }

    #[test]
    fn ignores_files_missing_json_suffix() {
        let dir = tempdir().unwrap();
        write_todo_file(
            dir.path(),
            "abc-agent-1.txt",
            r#"[{"status":"in_progress","content":"x"}]"#,
            1000,
        );
        assert_eq!(active_task("abc", dir.path()), (None, None));
    }
}
