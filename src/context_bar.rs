use crate::config::DisplayConfig;
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
    // Safe: clamp(0.0, 100.0) guarantees the value fits in u8 with no
    // truncation or sign loss.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        used.clamp(0.0, 100.0) as u8
    }
}

fn render_bar(used: u8, width: usize) -> String {
    let filled = ((used as usize) * width / 100).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

pub fn render(
    remaining_percentage: Option<f64>,
    total_tokens: f64,
    acw_env: f64,
    display: &DisplayConfig,
) -> String {
    let Some(remaining) = remaining_percentage else {
        return String::new();
    };
    let used = compute_used(remaining, total_tokens, acw_env);
    let bar = render_bar(used, display.bar_width as usize);
    if used < display.threshold_yellow {
        format!(" {GREEN}{bar} {used}%{RESET}")
    } else if used < display.threshold_orange {
        format!(" {YELLOW}{bar} {used}%{RESET}")
    } else if used < display.threshold_critical {
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
        assert_eq!(render(None, 1_000_000.0, 0.0, &DisplayConfig::default()), "");
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
        let out = render(Some(100.0), 1_000_000.0, 0.0, &DisplayConfig::default()); // used=0
        assert!(out.contains(GREEN));
        assert!(out.contains("░░░░░░░░░░ 0%"));
    }

    #[test]
    fn render_yellow_between_50_and_65() {
        // used=50 -> remaining = 16.5 + 50*0.835 = 58.25
        let out = render(Some(58.25), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.contains(YELLOW));
        assert!(out.contains("50%"));
    }

    #[test]
    fn render_orange_between_65_and_80() {
        // used=70 -> remaining = 16.5 + 30*0.835 = 41.55
        let out = render(Some(41.55), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.contains(ORANGE));
        assert!(out.contains("70%"));
    }

    #[test]
    fn render_blink_red_at_80_and_above() {
        let out = render(Some(0.0), 1_000_000.0, 0.0, &DisplayConfig::default()); // used=100
        assert!(out.contains(BLINK_RED));
        assert!(out.contains('💀'));
    }

    #[test]
    fn render_ends_with_reset() {
        let out = render(Some(100.0), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.ends_with(RESET));
    }

    #[test]
    fn compute_used_49_is_green() {
        assert_eq!(compute_used(59.085, 1_000_000.0, 0.0), 49);
        let out = render(Some(59.085), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.contains(GREEN));
    }

    #[test]
    fn compute_used_64_is_yellow() {
        assert_eq!(compute_used(46.56, 1_000_000.0, 0.0), 64);
        let out = render(Some(46.56), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.contains(YELLOW));
    }

    #[test]
    fn compute_used_65_is_orange() {
        assert_eq!(compute_used(45.725, 1_000_000.0, 0.0), 65);
        let out = render(Some(45.725), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.contains(ORANGE));
    }

    #[test]
    fn compute_used_79_is_orange() {
        assert_eq!(compute_used(34.035, 1_000_000.0, 0.0), 79);
        let out = render(Some(34.035), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.contains(ORANGE));
    }

    #[test]
    fn compute_used_80_is_blink_red_with_skull() {
        assert_eq!(compute_used(33.2, 1_000_000.0, 0.0), 80);
        let out = render(Some(33.2), 1_000_000.0, 0.0, &DisplayConfig::default());
        assert!(out.contains(BLINK_RED));
        assert!(out.contains('💀'));
    }

    #[test]
    fn bar_width_1_renders_single_character() {
        let d = DisplayConfig { bar_width: 1, ..DisplayConfig::default() };
        let out = render(Some(100.0), 1_000_000.0, 0.0, &d); // used=0
        assert!(out.contains("░ 0%"));
        assert!(!out.contains("░░")); // only one character
    }

    #[test]
    fn bar_width_1_at_full_usage_renders_single_block() {
        let d = DisplayConfig { bar_width: 1, ..DisplayConfig::default() };
        let out = render(Some(0.0), 1_000_000.0, 0.0, &d); // used=100
        assert!(out.contains("█ 100%"));
        assert!(!out.contains("██"));
    }

    #[test]
    fn bar_width_20_renders_twenty_characters() {
        let d = DisplayConfig { bar_width: 20, ..DisplayConfig::default() };
        let out = render(Some(100.0), 1_000_000.0, 0.0, &d); // used=0
        assert!(out.contains(&"░".repeat(20)));
    }

    #[test]
    fn custom_thresholds_are_honored() {
        let d = DisplayConfig {
            threshold_yellow: 30,
            threshold_orange: 60,
            threshold_critical: 90,
            ..DisplayConfig::default()
        };
        // used=25 -> green (below 30)
        let out = render(Some(79.125), 1_000_000.0, 0.0, &d);
        assert!(out.contains(GREEN));
        // used=50 -> yellow (between 30 and 60)
        let out = render(Some(58.25), 1_000_000.0, 0.0, &d);
        assert!(out.contains(YELLOW));
        // used=80 -> orange (between 60 and 90)
        let out = render(Some(33.2), 1_000_000.0, 0.0, &d);
        assert!(out.contains(ORANGE));
        // used=95 -> blink red (above 90)
        let out = render(Some(20.675), 1_000_000.0, 0.0, &d);
        assert!(out.contains(BLINK_RED));
    }

    #[test]
    fn width_does_not_panic_at_edge_cases() {
        // width=1, used=100 — the underflow risk from the spec
        let d = DisplayConfig { bar_width: 1, ..DisplayConfig::default() };
        let out = render(Some(0.0), 1_000_000.0, 0.0, &d);
        assert!(out.contains("█ 100%"));
        // width=100, used=0
        let d = DisplayConfig { bar_width: 100, ..DisplayConfig::default() };
        let out = render(Some(100.0), 1_000_000.0, 0.0, &d);
        assert!(out.contains(&"░".repeat(100)));
    }
}
