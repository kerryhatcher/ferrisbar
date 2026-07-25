pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const ORANGE: &str = "\x1b[38;5;208m";
pub const BLINK_RED: &str = "\x1b[5;31m";

pub fn compose_statusline(
    model: &str,
    ctx: &str,
    task: Option<&str>,
    dirname: &str,
    show_task: bool,
) -> String {
    let model_seg = format!("{DIM}{model}{RESET}");
    let dir_seg = format!("{DIM}{dirname}{RESET}");
    let ctx_seg = if ctx.is_empty() {
        String::new()
    } else {
        format!(" {DIM}│{RESET}{ctx}")
    };
    match task {
        Some(t) if !t.is_empty() && show_task => {
            format!("{model_seg} │ {BOLD}{t}{RESET} │ {dir_seg}{ctx_seg}")
        }
        _ => format!("{model_seg} │ {dir_seg}{ctx_seg}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_without_task_or_ctx() {
        let out = compose_statusline("Claude", "", None, "myproject", true);
        assert_eq!(out, format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET}"));
    }

    #[test]
    fn composes_with_task_no_ctx() {
        let out = compose_statusline("Claude", "", Some("Fix bug"), "myproject", true);
        assert_eq!(
            out,
            format!("{DIM}Claude{RESET} │ {BOLD}Fix bug{RESET} │ {DIM}myproject{RESET}")
        );
    }

    #[test]
    fn composes_with_ctx_no_task() {
        let ctx = format!(" {GREEN}████░░░░░░ 42%{RESET}");
        let out = compose_statusline("Claude", &ctx, None, "myproject", true);
        assert_eq!(
            out,
            format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET} {DIM}│{RESET}{ctx}")
        );
    }

    #[test]
    fn composes_with_task_and_ctx() {
        let ctx = format!(" {GREEN}████░░░░░░ 42%{RESET}");
        let out = compose_statusline("Claude", &ctx, Some("Fix bug"), "myproject", true);
        assert_eq!(
            out,
            format!(
                "{DIM}Claude{RESET} │ {BOLD}Fix bug{RESET} │ {DIM}myproject{RESET} {DIM}│{RESET}{ctx}"
            )
        );
    }

    #[test]
    fn empty_task_treated_as_no_task() {
        let out = compose_statusline("Claude", "", Some(""), "myproject", true);
        assert_eq!(out, format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET}"));
    }

    #[test]
    fn show_task_false_suppresses_task() {
        let out = compose_statusline("Claude", "", Some("Fix bug"), "myproject", false);
        assert_eq!(out, format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET}"));
        // The task text must not appear anywhere.
        assert!(!out.contains("Fix bug"));
    }
}
