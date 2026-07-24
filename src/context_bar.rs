use crate::layout::{BLINK_RED, GREEN, ORANGE, RESET, YELLOW};

pub fn compute_used(remaining_percentage: f64, total_tokens: f64, acw_env: f64) -> u8 {
    let buffer_pct = if acw_env > 0.0 {
        ((1.0 - acw_env / total_tokens) * 100.0).clamp(0.0, 100.0)
    } else {
        16.5
    };
    let usable_remaining =
        ((remaining_percentage - buffer_pct) / (100.0 - buffer_pct) * 100.0).max(0.0);
    let used = (100.0 - usable_remaining).round();
    used.clamp(0.0, 100.0) as u8
}

fn render_bar(used: u8) -> String {
    let filled = (used / 10) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
}

pub fn render(remaining_percentage: Option<f64>, total_tokens: f64, acw_env: f64) -> String {
    let Some(remaining) = remaining_percentage else {
        return String::new();
    };
    let used = compute_used(remaining, total_tokens, acw_env);
    let bar = render_bar(used);
    if used < 50 {
        format!(" {GREEN}{bar} {used}%{RESET}")
    } else if used < 65 {
        format!(" {YELLOW}{bar} {used}%{RESET}")
    } else if used < 80 {
        format!(" {ORANGE}{bar} {used}%{RESET}")
    } else {
        format!(" {BLINK_RED}💀 {bar} {used}%{RESET}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_empty_when_no_remaining_percentage() {
        assert_eq!(render(None, 1_000_000.0, 0.0), "");
    }

    #[test]
    fn compute_used_full_remaining_is_zero_used() {
        assert_eq!(compute_used(100.0, 1_000_000.0, 0.0), 0);
    }

    #[test]
    fn compute_used_at_buffer_boundary_is_full() {
        assert_eq!(compute_used(16.5, 1_000_000.0, 0.0), 100);
    }

    #[test]
    fn compute_used_below_buffer_clamps_to_100() {
        assert_eq!(compute_used(0.0, 1_000_000.0, 0.0), 100);
    }

    #[test]
    fn compute_used_honors_acw_env_override() {
        // total=1000, acw=500 -> buffer_pct = (1 - 500/1000) * 100 = 50
        // remaining=75 -> usable=(75-50)/(100-50)*100=50 -> used=50
        assert_eq!(compute_used(75.0, 1000.0, 500.0), 50);
    }

    #[test]
    fn render_green_below_50() {
        let out = render(Some(100.0), 1_000_000.0, 0.0); // used=0
        assert!(out.contains(GREEN));
        assert!(out.contains("░░░░░░░░░░ 0%"));
    }

    #[test]
    fn render_yellow_between_50_and_65() {
        // used=50 -> remaining = 16.5 + 50*0.835 = 58.25
        let out = render(Some(58.25), 1_000_000.0, 0.0);
        assert!(out.contains(YELLOW));
        assert!(out.contains("50%"));
    }

    #[test]
    fn render_orange_between_65_and_80() {
        // used=70 -> remaining = 16.5 + 30*0.835 = 41.55
        let out = render(Some(41.55), 1_000_000.0, 0.0);
        assert!(out.contains(ORANGE));
        assert!(out.contains("70%"));
    }

    #[test]
    fn render_blink_red_at_80_and_above() {
        let out = render(Some(0.0), 1_000_000.0, 0.0); // used=100
        assert!(out.contains(BLINK_RED));
        assert!(out.contains('💀'));
    }

    #[test]
    fn render_ends_with_reset() {
        let out = render(Some(100.0), 1_000_000.0, 0.0);
        assert!(out.ends_with(RESET));
    }
}
