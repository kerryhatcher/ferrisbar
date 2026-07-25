mod config;
mod config_dir;
mod context_bar;
mod layout;
mod log;
mod paths;
mod payload;
mod setup;
mod todo;

use payload::Payload;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};

fn resolve_todos_dir() -> PathBuf {
    config_dir::claude_config_dir(None).join("todos")
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [cmd] if cmd == "setup" => {
            if let Err(e) = setup::run(false, &config::Config::default()) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        [cmd, flag] if cmd == "setup" && flag == "--project" => {
            if let Err(e) = setup::run(true, &config::Config::default()) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        _ => {
            let program = env::args().next().unwrap_or_default();
            let program_name = Path::new(&program).file_name().map_or_else(
                || "ferrisbar".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            eprintln!("Usage: {program_name} [setup [--project]]");
            std::process::exit(1);
        }
    }

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
        .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());

    let output = layout::compose_statusline(&model, &ctx, task.as_deref(), &dirname);
    print!("{output}");
}
