mod context_bar;
mod layout;
mod payload;
mod todo;

use payload::Payload;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};

fn resolve_todos_dir() -> PathBuf {
    let claude_dir = match env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home = env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".claude")
        }
    };
    claude_dir.join("todos")
}

fn main() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }

    let Ok(payload) = serde_json::from_str::<Payload>(&input) else {
        return;
    };

    let process_cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cwd = payload.cwd(&process_cwd);
    let model = payload.model_name();
    let session_id = payload.session_id();

    let acw_env: f64 = env::var("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let ctx = context_bar::render(
        payload.remaining_percentage(),
        payload.total_tokens(),
        acw_env,
    );

    let todos_dir = resolve_todos_dir();
    let task = todo::active_task(&session_id, &todos_dir);

    let dirname = Path::new(&cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.clone());

    let output = layout::compose_statusline(&model, &ctx, task.as_deref(), &dirname);
    print!("{output}");
}
