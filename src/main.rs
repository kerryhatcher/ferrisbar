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

fn resolve_todos_dir(cfg: &config::Config) -> PathBuf {
    config_dir::claude_config_dir(Some(&cfg.claude.config_dir)).join("todos")
}

fn main() {
    // Config and logging come up before anything else can fail, so the
    // failures below are reportable.
    let (mut cfg, warnings) = config::load(paths::config_file().as_deref());

    // Environment beats file: applied before the logger reads the config.
    if let Some(path) = env::var("FERRISBAR_LOG_PATH")
        .ok()
        .filter(|v| !v.is_empty())
    {
        cfg.log.path = path;
    }
    if let Some(level) = env::var("FERRISBAR_LOG_LEVEL")
        .ok()
        .filter(|v| !v.is_empty())
    {
        cfg.log.level = level;
    }

    let logger = log::Logger::new(&cfg, paths::data_dir().as_deref());
    for warning in &warnings {
        match warning {
            config::ParseWarning::Syntax(msg) => {
                logger.log(&log::warn("config_parse_failed", msg.clone()));
            }
            config::ParseWarning::Create(msg) => {
                logger.log(&log::warn("config_unavailable", msg.clone()));
            }
        }
    }

    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [cmd] if cmd == "setup" => {
            if let Err(e) = setup::run(false, &cfg) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        [cmd, flag] if cmd == "setup" && flag == "--project" => {
            if let Err(e) = setup::run(true, &cfg) {
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
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        logger.log(&log::warn("stdin_read_failed", e.to_string()));
        return;
    }

    let payload = match serde_json::from_str::<Payload>(&input) {
        Ok(payload) => payload,
        Err(e) => {
            // Empty stdin is the documented no-op, not a fault.
            if !input.trim().is_empty() {
                logger.log(&log::warn("stdin_parse_failed", e.to_string()));
            }
            return;
        }
    };

    let process_cwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cwd = payload.cwd(&process_cwd);
    let model = payload.model_name();
    let session_id = payload.session_id();

    let acw: f64 = env::var("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &f64| *v > 0.0)
        .unwrap_or(cfg.claude.auto_compact_window);
    let ctx = context_bar::render(payload.remaining_percentage(), payload.total_tokens(), acw);

    let todos_dir = resolve_todos_dir(&cfg);
    // `active_task` returns None both for "no task in progress" and for
    // "the directory isn't there", which are very different problems. The
    // existence check separates them without changing todo.rs's signature.
    //
    // Gated on the parent existing: a Claude config dir that has no todos/
    // is a genuine anomaly worth a warning, whereas no Claude dir at all
    // means the resolved path is simply wrong, and warning on every render
    // for a user who has never run Claude Code there would be pure churn.
    let claude_dir_present = todos_dir.parent().is_some_and(Path::is_dir);
    if !session_id.is_empty() && claude_dir_present && !todos_dir.is_dir() {
        let mut event = log::warn(
            "todo_file_unreadable",
            format!("todos directory not found: {}", todos_dir.display()),
        );
        event.session_id = Some(session_id.clone());
        logger.log(&event);
    }
    let task = todo::active_task(&session_id, &todos_dir);

    let dirname = Path::new(&cwd)
        .file_name()
        .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());

    let output = layout::compose_statusline(&model, &ctx, task.as_deref(), &dirname);

    let mut render = log::debug("render", format!("model={model} dir={dirname} acw={acw}"));
    render.session_id = Some(session_id);
    logger.log(&render);

    print!("{output}");
}
