mod analytics;
mod config;
mod config_dir;
mod context_bar;
mod cost;
mod cost_cache;
mod git;
mod layout;
mod log;
mod paths;
mod payload;
mod repo_identity;
mod setup;
mod todo;

use payload::Payload;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};

fn resolve_todos_dir(cfg: &config::Config) -> PathBuf {
    config_dir::claude_config_dir(Some(&cfg.claude.config_dir)).join("todos")
}

fn resolve_transcripts_dir(cfg: &config::Config) -> PathBuf {
    config_dir::claude_config_dir(Some(&cfg.claude.config_dir)).join("projects")
}

/// Environment beats file, applied before the logger reads the config.
///
/// An explicit, non-`"off"` `FERRISBAR_LOG_LEVEL` also forces
/// `enabled = true`: it is the documented escape hatch for turning logging
/// back on for one session without editing the file, and that must work
/// even when the file says `enabled = false`. `FERRISBAR_LOG_LEVEL=off`
/// still disables, via `Logger::new`'s existing `level != Off` check.
fn apply_env_overrides(cfg: &mut config::Config) {
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
        if !level.trim().eq_ignore_ascii_case("off") {
            cfg.log.enabled = true;
        }
        cfg.log.level = level;
    }
    // A quick, file-free way to silence the daily-cost background refresh —
    // handy in CI/tests, where spawning a detached process per render is
    // pure noise, and for a one-off session where the feature is unwanted.
    if let Some(ttl) = env::var("FERRISBAR_COST_TTL_SECONDS")
        .ok()
        .filter(|v| !v.is_empty())
        .and_then(|v| v.parse().ok())
    {
        cfg.cost.ttl_seconds = ttl;
    }
}

fn flush_config_warnings(logger: &log::Logger, warnings: &[config::ParseWarning]) {
    for warning in warnings {
        match warning {
            config::ParseWarning::Syntax(msg) => {
                logger.log(&log::warn("config_parse_failed", msg.clone()));
            }
            config::ParseWarning::Create(msg) => {
                logger.log(&log::warn("config_unavailable", msg.clone()));
            }
        }
    }
}

/// Returns `true` when a subcommand consumed this invocation, meaning `main`
/// should return without reading stdin. `setup` runs to completion (or
/// exits the process on failure) before returning `true`; an unknown
/// subcommand exits the process directly.
///
/// `--internal-refresh-daily-cost` is undocumented on purpose: it exists
/// only so `cost_cache::spawn_refresh` can re-invoke this same binary as a
/// detached background process (see that module's docs), never something a
/// user types themselves.
fn dispatch_subcommand(cfg: &config::Config, data_dir: Option<&Path>) -> bool {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => false,
        [cmd] if cmd == "--internal-refresh-daily-cost" => {
            if let Some(data_dir) = data_dir {
                cost::refresh_daily_cache(&resolve_transcripts_dir(cfg), data_dir);
            }
            true
        }
        [cmd] if cmd == "setup" => {
            if let Err(e) = setup::run(false, cfg) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            true
        }
        [cmd, flag] if cmd == "setup" && flag == "--project" => {
            if let Err(e) = setup::run(true, cfg) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            true
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
}

/// Logs the one `debug`-level `render` event per invocation. `used_pct` is
/// `None` when the payload carried no `context_window.remaining_percentage`,
/// in which case the gauge itself is also omitted from the statusline.
fn log_render_event(
    logger: &log::Logger,
    session_id: String,
    model: &str,
    dirname: &str,
    acw: f64,
    used_pct: Option<u8>,
    elapsed: std::time::Duration,
) {
    let used_pct_str = used_pct.map_or_else(|| "none".to_string(), |v| v.to_string());
    let elapsed_micros = elapsed.as_micros();
    let mut render = log::debug(
        "render",
        format!(
            "model={model} dir={dirname} acw={acw} used_pct={used_pct_str} \
             elapsed_micros={elapsed_micros}"
        ),
    );
    render.session_id = Some(session_id);
    logger.log(&render);
}

fn main() {
    let start = std::time::Instant::now();

    // Config and logging come up before anything else can fail, so the
    // failures below are reportable.
    let (mut cfg, warnings) = config::load(paths::config_file().as_deref());
    apply_env_overrides(&mut cfg);

    let data_dir = paths::data_dir();
    let logger = log::Logger::new(&cfg, data_dir.as_deref());
    flush_config_warnings(&logger, &warnings);

    if dispatch_subcommand(&cfg, data_dir.as_deref()) {
        return;
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
    let ctx = context_bar::render(
        payload.remaining_percentage(),
        payload.total_tokens(),
        acw,
        &cfg.display,
    );

    let todos_dir = resolve_todos_dir(&cfg);
    let (task, todo_diagnostic) = todo::active_task(&session_id, &todos_dir);
    if let Some(msg) = todo_diagnostic {
        let mut event = log::warn("todo_file_unreadable", msg);
        event.session_id = Some(session_id.clone());
        logger.log(&event);
    }

    let dirname = Path::new(&cwd)
        .file_name()
        .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());
    let branch = git::branch_name(&cwd);

    let session_cost = cfg
        .cost
        .show_session
        .then(|| payload.session_cost_usd())
        .flatten();
    let mut output = layout::compose_statusline(
        &model,
        &ctx,
        task.as_deref(),
        &dirname,
        cfg.display.show_task,
        session_cost,
        branch.as_deref(),
    );
    if let Some(daily) = cost::daily_chip(&cfg.cost, data_dir.as_deref()) {
        output.push('\n');
        output.push_str(&daily);
    }
    if let Some(budget) = cost::budget_line(
        &cfg.budget,
        &cfg.cost,
        &cfg.display,
        data_dir.as_deref(),
        session_cost,
    ) {
        output.push('\n');
        output.push_str(&budget);
    }

    let used_pct = payload
        .remaining_percentage()
        .map(|remaining| context_bar::compute_used(remaining, payload.total_tokens(), acw));
    log_render_event(
        &logger,
        session_id,
        &model,
        &dirname,
        acw,
        used_pct,
        start.elapsed(),
    );

    print!("{output}");
}
