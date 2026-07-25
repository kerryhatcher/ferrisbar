# ferrisbar Config File and JSONL Logging — Phase 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Parse the `[display]` block from `config.toml`, thread it through `context_bar.rs` and `layout.rs`, and add it to the generated config template — all without changing stdout for users who haven't touched the file.

**Architecture:** Three changes, each self-contained: `config.rs` gains a `DisplayConfig` struct and parses the new section with clamping and threshold validation; `context_bar.rs` accepts configurable bar width and thresholds; `layout.rs` accepts a `show_task` flag. `main.rs` threads the new struct through.

**Tech Stack:** Rust 2021, `toml` (existing), no new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-25-config-and-logging-design.md` (Phase 2 detail section)

**Scope:** Phase 2 only. The `[display]` block is the last piece of the config-and-logging feature. No new environment variables — display settings are file-only.

## Global Constraints

Every task's requirements implicitly include this section.

- **MSRV is 1.85.1.** No stdlib API stabilized after it. `just msrv` is the gate.
- **Never panic.** No new `unwrap`, `expect`, `panic!`, slicing by range, or integer subtraction that can underflow on the render path. The central risk is `width - filled` in `context_bar.rs` — `filled` must be clamped to `width` first.
- **Nothing new on stdout, ever.** A stray `println!` corrupts the user's prompt on every render. Diagnostics go to the log file or stderr.
- **Every new failure mode degrades.** A malformed `[display]` block, out-of-order thresholds, or a `bar_width` of `0` each fall back to defaults and still print the statusline.
- **Clippy `pedantic` + `nursery`, CI runs `-D warnings`.** Run `just lint` at the end of every task, not just at the end of the plan.
- **Conventional Commits.** `release-please` derives the version bump from the prefix. Never hand-edit `version` in `Cargo.toml` or `CHANGELOG.md`.
- **Branch is `feat/config-and-logging`**, already pushed and holding Phase 1. Direct pushes to `main` are blocked.

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `src/config.rs` | Modify | Add `DisplayConfig`, parse `[display]`, add to template, update tests |
| `src/context_bar.rs` | Modify | Accept configurable width and thresholds |
| `src/layout.rs` | Modify | Accept `show_task` flag |
| `src/main.rs` | Modify | Thread `cfg.display` through to render and compose |

**Test invocations** (verified against this repo):
- Unit tests for one module: `cargo test --bin ferrisbar config::`
- One unit test: `cargo test --bin ferrisbar config::tests::name -- --exact`
- End-to-end: `cargo test --test cli`
- Everything: `just test`

---

### Task 1: `src/config.rs` — `DisplayConfig` struct, parsing, clamping, and template

Adds the `DisplayConfig` struct, parses the `[display]` section with clamping and threshold validation, and adds the block to the generated template. The existing `template_does_not_mention_display` test is updated to assert the opposite.

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Consumes: `toml::Table`, existing `section`/`get_bool`/`get_integer` helpers
- Produces:
  - `pub struct DisplayConfig { pub bar_width: u8, pub threshold_yellow: u8, pub threshold_orange: u8, pub threshold_critical: u8, pub show_task: bool }`
  - `impl Default for DisplayConfig`
  - `DisplayConfig` field added to `Config`
  - `[display]` block added to `TEMPLATE`

- [ ] **Step 1: Add `DisplayConfig` and update `Config`**

Insert after `ClaudeConfig` and before `impl Default for LogConfig`:

```rust
pub const MIN_BAR_WIDTH: u8 = 1;
pub const MAX_BAR_WIDTH: u8 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayConfig {
    pub bar_width: u8,
    pub threshold_yellow: u8,
    pub threshold_orange: u8,
    pub threshold_critical: u8,
    pub show_task: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            bar_width: 10,
            threshold_yellow: 50,
            threshold_orange: 65,
            threshold_critical: 80,
            show_task: true,
        }
    }
}
```

Add `pub display: DisplayConfig` to the `Config` struct and its `Default` impl.

- [ ] **Step 2: Add the `[display]` block to `TEMPLATE`**

Append to the `TEMPLATE` constant, after the `[claude]` block:

```toml
[display]
bar_width          = 10
threshold_yellow   = 50
threshold_orange   = 65
threshold_critical = 80
show_task          = true
```

- [ ] **Step 3: Parse `[display]` in `from_toml_str`**

Add after the `claude` section parsing in `from_toml_str`:

```rust
let display = section(&table, "display");
let defaults_display = DisplayConfig::default();

let bar_width = get_integer(display, "bar_width")
    .and_then(|v| u8::try_from(v).ok())
    .map_or(defaults_display.bar_width, |v| v.clamp(MIN_BAR_WIDTH, MAX_BAR_WIDTH));

let threshold_yellow = get_integer(display, "threshold_yellow")
    .and_then(|v| u8::try_from(v).ok())
    .unwrap_or(defaults_display.threshold_yellow);

let threshold_orange = get_integer(display, "threshold_orange")
    .and_then(|v| u8::try_from(v).ok())
    .unwrap_or(defaults_display.threshold_orange);

let threshold_critical = get_integer(display, "threshold_critical")
    .and_then(|v| u8::try_from(v).ok())
    .unwrap_or(defaults_display.threshold_critical);

// Thresholds must be monotonically increasing. When they are not, all
// three fall back to defaults — a single out-of-order value is a
// configuration error, not a reason to guess which one the user meant.
let (threshold_yellow, threshold_orange, threshold_critical) =
    if threshold_yellow < threshold_orange && threshold_orange < threshold_critical {
        (threshold_yellow, threshold_orange, threshold_critical)
    } else {
        (
            defaults_display.threshold_yellow,
            defaults_display.threshold_orange,
            defaults_display.threshold_critical,
        )
    };

let show_task = get_bool(display, "show_task").unwrap_or(defaults_display.show_task);
```

And add `display: DisplayConfig { ... }` to the `Config` construction at the end.

- [ ] **Step 4: Update the `template_does_not_mention_display` test**

The test currently asserts the template does *not* mention `[display]`. Flip it:

```rust
#[test]
fn template_includes_display_block() {
    assert!(TEMPLATE.contains("[display]"));
    assert!(TEMPLATE.contains("bar_width"));
    assert!(TEMPLATE.contains("threshold_yellow"));
    assert!(TEMPLATE.contains("show_task"));
}
```

- [ ] **Step 5: Add parsing tests**

Append to `mod tests`:

```rust
#[test]
fn display_defaults_match_the_documented_values() {
    let d = DisplayConfig::default();
    assert_eq!(d.bar_width, 10);
    assert_eq!(d.threshold_yellow, 50);
    assert_eq!(d.threshold_orange, 65);
    assert_eq!(d.threshold_critical, 80);
    assert!(d.show_task);
}

#[test]
fn display_values_are_read_from_toml() {
    let (c, _) = from_toml_str(
        "[display]\nbar_width = 20\nthreshold_yellow = 40\n\
         threshold_orange = 60\nthreshold_critical = 90\nshow_task = false\n",
    );
    assert_eq!(c.display.bar_width, 20);
    assert_eq!(c.display.threshold_yellow, 40);
    assert_eq!(c.display.threshold_orange, 60);
    assert_eq!(c.display.threshold_critical, 90);
    assert!(!c.display.show_task);
}

#[test]
fn bar_width_clamps_to_range() {
    let (zero, _) = from_toml_str("[display]\nbar_width = 0\n");
    assert_eq!(zero.display.bar_width, MIN_BAR_WIDTH);
    let (huge, _) = from_toml_str("[display]\nbar_width = 200\n");
    assert_eq!(huge.display.bar_width, MAX_BAR_WIDTH);
}

#[test]
fn bar_width_negative_falls_back() {
    let (c, _) = from_toml_str("[display]\nbar_width = -5\n");
    assert_eq!(c.display.bar_width, DisplayConfig::default().bar_width);
}

#[test]
fn out_of_order_thresholds_fall_back_to_defaults() {
    let (c, _) = from_toml_str(
        "[display]\nthreshold_yellow = 80\nthreshold_orange = 50\nthreshold_critical = 90\n",
    );
    // yellow >= orange, so all three fall back.
    assert_eq!(c.display.threshold_yellow, 50);
    assert_eq!(c.display.threshold_orange, 65);
    assert_eq!(c.display.threshold_critical, 80);
}

#[test]
fn equal_thresholds_fall_back() {
    let (c, _) = from_toml_str(
        "[display]\nthreshold_yellow = 50\nthreshold_orange = 50\nthreshold_critical = 80\n",
    );
    assert_eq!(c.display.threshold_yellow, 50);
    assert_eq!(c.display.threshold_orange, 65);
    assert_eq!(c.display.threshold_critical, 80);
}

#[test]
fn critical_below_orange_falls_back() {
    let (c, _) = from_toml_str(
        "[display]\nthreshold_yellow = 30\nthreshold_orange = 60\nthreshold_critical = 40\n",
    );
    assert_eq!(c.display.threshold_yellow, 50);
    assert_eq!(c.display.threshold_orange, 65);
    assert_eq!(c.display.threshold_critical, 80);
}

#[test]
fn partial_display_block_fills_in_defaults() {
    let (c, _) = from_toml_str("[display]\nbar_width = 5\n");
    assert_eq!(c.display.bar_width, 5);
    assert_eq!(c.display.threshold_yellow, 50);
    assert!(c.display.show_task);
}

#[test]
fn template_round_trips_including_display() {
    let (c, warnings) = from_toml_str(TEMPLATE);
    assert!(warnings.is_empty());
    assert_eq!(c, Config::default());
}
```

- [ ] **Step 6: Run tests and lint**

```bash
cargo test --bin ferrisbar config:: && just lint
```

Expected: both PASS. The `template_round_trips_including_display` test will catch any mismatch between the template and the defaults.

- [ ] **Step 7: Commit**

```bash
git add src/config.rs
git commit -m "feat: parse [display] config block with clamping and threshold validation"
```

---

### Task 2: `src/context_bar.rs` — configurable bar width and thresholds

The central risk from the spec: `10 - filled` becomes `width - filled`, a `usize` subtraction that underflows and panics unless `filled` is clamped to `width` first. The fix is `filled.min(width)`.

**Files:**
- Modify: `src/context_bar.rs`

**Interfaces:**
- Consumes: `config::DisplayConfig`
- Produces:
  - `pub fn render(remaining_percentage: Option<f64>, total_tokens: f64, acw_env: f64, display: &DisplayConfig) -> String`
  - `render_bar` gains a `width: usize` parameter

- [ ] **Step 1: Update `render_bar` to accept a configurable width**

Replace the hardcoded `10`:

```rust
fn render_bar(used: u8, width: usize) -> String {
    let filled = ((used as usize) * width / 100).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}
```

The `filled.min(width)` is the underflow guard: when `used` is 100 and `width` is 1, `filled` computes to 1, and `width - filled` is 0 — safe. Without the `.min(width)`, rounding could push `filled` past `width` and the subtraction would panic.

- [ ] **Step 2: Update `render` to accept `&DisplayConfig`**

Change the signature and body:

```rust
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
```

Add `use crate::config::DisplayConfig;` at the top.

- [ ] **Step 3: Update all existing tests**

The existing tests call `render` with three arguments. Add `&DisplayConfig::default()` as the fourth argument everywhere. The tests should still pass because the defaults match the old hardcoded values.

- [ ] **Step 4: Add new tests for configurable width and thresholds**

Append to `mod tests`:

```rust
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
```

- [ ] **Step 5: Run tests and lint**

```bash
cargo test --bin ferrisbar context_bar:: && just lint
```

Expected: all existing tests pass with the new signature, all new tests pass, lint clean.

- [ ] **Step 6: Commit**

```bash
git add src/context_bar.rs
git commit -m "feat: make context bar width and thresholds configurable"
```

---

### Task 3: `src/layout.rs` — `show_task` flag

Adds a `show_task` parameter to `compose_statusline`. When `false`, the task segment is suppressed even when a task is present.

**Files:**
- Modify: `src/layout.rs`

**Interfaces:**
- Consumes: nothing new
- Produces: `pub fn compose_statusline(model: &str, ctx: &str, task: Option<&str>, dirname: &str, show_task: bool) -> String`

- [ ] **Step 1: Update `compose_statusline`**

Add the `show_task: bool` parameter and gate the task branch:

```rust
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
```

- [ ] **Step 2: Update existing tests**

Add `true` as the fifth argument to every existing `compose_statusline` call. All should pass unchanged.

- [ ] **Step 3: Add a test for `show_task = false`**

```rust
#[test]
fn show_task_false_suppresses_task() {
    let out = compose_statusline("Claude", "", Some("Fix bug"), "myproject", false);
    assert_eq!(
        out,
        format!("{DIM}Claude{RESET} │ {DIM}myproject{RESET}")
    );
    // The task text must not appear anywhere.
    assert!(!out.contains("Fix bug"));
}
```

- [ ] **Step 4: Run tests and lint**

```bash
cargo test --bin ferrisbar layout:: && just lint
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/layout.rs
git commit -m "feat: add show_task flag to suppress the task segment"
```

---

### Task 4: `src/main.rs` — wire `DisplayConfig` through the render path

Threads `cfg.display` into `context_bar::render` and `layout::compose_statusline`. This is the smallest task — the three modules already accept the new parameters; this just connects them.

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Pass `&cfg.display` to `context_bar::render`**

Change the call at the `context_bar::render` line:

```rust
let ctx = context_bar::render(
    payload.remaining_percentage(),
    payload.total_tokens(),
    acw,
    &cfg.display,
);
```

- [ ] **Step 2: Pass `cfg.display.show_task` to `layout::compose_statusline`**

Change the call:

```rust
let output = layout::compose_statusline(
    &model,
    &ctx,
    task.as_deref(),
    &dirname,
    cfg.display.show_task,
);
```

- [ ] **Step 3: Build, lint, and run the full suite**

```bash
just ci
```

Expected: EXIT 0. All unit tests, e2e tests, fmt, clippy, audit, msrv, deny, trivy, vet, and geiger pass.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire display config through context bar and layout"
```

---

## Post-implementation verification

After all four tasks are committed:

- [ ] **Byte-identical stdout for default config.** Build the binary, run it against the same payloads used in the Phase 1 fix-wave verification, and confirm stdout is byte-identical to the pre-Phase-2 binary. The defaults match the old hardcoded values exactly, so this should hold.
- [ ] **Custom `[display]` values take effect.** Write a `config.toml` with `bar_width = 5`, `threshold_yellow = 30`, `show_task = false`, and confirm the statusline reflects all three.
- [ ] **`just ci` is green.** The full check suite passes.
- [ ] **No new `#[allow(dead_code)]` annotations.** Every new item has a real caller.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| `width - filled` underflows when `filled > width` | `filled.min(width)` in `render_bar`, tested at `width = 1, used = 100` |
| Out-of-order thresholds produce nonsensical colors | Monotonicity check in `from_toml_str` falls back to all three defaults |
| `bar_width = 0` produces an empty bar or division by zero | Clamped to `1..=100` on parse |
| Existing tests break on signature changes | Every test updated in the same commit as the signature change |
| Template round-trip fails because defaults and template diverge | `template_round_trips_including_display` test catches this |
