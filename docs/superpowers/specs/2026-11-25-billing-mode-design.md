# ferrisbar — billing mode: subscription vs pay-for-usage budget line

## Purpose

Let the user say how they pay for Claude, and render the budget line to
match. A subscriber needs the plan's real usage percentages. A pay-for-usage
(Enterprise / API) user has no quota and needs self-imposed dollar budgets.
Today one implicit fallback serves both and misleads each.

## Background

`budget_line` (`src/cost.rs`) renders up to five bars: `sess`, `day`, `wk`,
`mo`, `5h`. For `wk` and `5h`, `plan_pct` returns the payload's
`rate_limits.*.used_percentage` when `[budget] plan_limits = true` and the
payload carries it. Otherwise the bar falls back to
`spent_usd / configured_limit_usd`.

Observed failure (2026-11-25, personal subscription): the `5h` bar read 205%.
It was `block5h_usd` $88.96 (API list-price estimate of the trailing 5h)
divided by a user-written `block5h_usd = 43.0`. Every bar on the line was a
dollar bar, so the payload evidently carried no `rate_limits` (to be
confirmed from a captured payload; see Open questions). The fallback painted
an invented dollar limit as a quota, in blinking red.

Two defects follow:

1. The mode is implicit. Presence of a payload field silently flips the
   meaning of a bar between "your plan's quota" and "a number you made up".
2. For a subscriber, a dollar-fallback percentage is not a quota at all.
   Anthropic publishes no dollar caps, and list-price estimates do not map
   to plan limits.

## Decisions

| Question | Decision |
|---|---|
| Setting name | `[budget] mode`, string |
| Values | `"auto"` (default), `"subscription"`, `"usage"` |
| `plan_limits` bool | Kept as a deprecated alias: `true` maps to `auto`, `false` maps to `usage`. `mode`, when present, wins. No warning needed; both parse silently |
| Dollar bars for a subscriber | Shown as plain spend, never as a percentage of a guessed limit |
| Weekly reset time | Use payload `resets_at` when present; `weekly_reset_*` config remains the anchor for `usage` mode and as fallback |

## Behavior by mode

### `subscription`

Plan percentages only; ferrisbar never divides spend by a configured dollar
limit.

- `5h`, `wk`: payload `used_percentage` rendered as today (`5h 41%`).
- Window missing from the payload (not yet sent, or already reset): the
  segment is omitted. It is never replaced by a dollar-derived percentage.
- `day`, `mo`, `sess`: hidden entirely. An earlier draft showed them as plain
  spend; that was dropped because the line is meant to show only what the
  plan reports. The cost cache is not read in this mode.
- `weekly_usd`, `session_usd`, `block5h_usd` are ignored.
- The 5h rolling-spend figure may be shown as `5h $89` only when the
  payload lacks `five_hour` and the user opts in with
  `show_block5h_spend = true` (default `false`).

### `usage`

Self-imposed dollar budgets, the current non-plan behavior.

- All five windows render as `spent / configured limit` bars, as today.
- `rate_limits` in the payload is ignored even when present.
- Label carries the unit so the line reads unambiguously: `wk $185/$390`
  at a width where it fits, else `wk 47%` (the bar is unchanged).
- Requires `enabled = true`, as today. `setup` seeds sensible numbers
  instead of the placeholder 100/5/15.

### `auto` (default)

Per window, independently:

- Payload has the window: behave as `subscription` for that bar.
- Payload lacks it, and the user has set a dollar limit explicitly in
  config: behave as `usage` for that bar.
- Payload lacks it, and the limit is the built-in placeholder (user never
  set it): omit the bar. The built-in defaults are not real limits and must
  not drive a percentage.

"User set it" requires distinguishing absent from default. `parse_budget`
will keep `Option<f64>` for the three limits internally and resolve to a
value only where a limit is actually needed.

## `resets_at`

`RateWindow` already parses `resets_at` (epoch seconds) in tests but
`plan_usage` discards it. Extend `PlanUsage` with
`five_hour_resets_at` / `seven_day_resets_at: Option<i64>`. In
`subscription`/`auto`, the `wk` window's cost-cache bucket boundary (used by
`weekly_usd` aggregation) and any "resets in Xh" text derive from
`seven_day.resets_at` minus 7 days when present, else the configured
`WeeklyAnchor`. Not-JSON-number or negative values degrade to `None`,
following the never-panic invariant.

## Config surface

```toml
[budget]
mode = "auto"   # "auto" | "subscription" | "usage"
                # subscription: plan % from Claude Code, never invented $ limits
                # usage:        $ budgets below (Enterprise / API billing)
                # auto:         plan % where Claude Code reports it, else
                #               your explicit $ limits, else the bar is hidden
```

`ferrisbar setup` asks one question, "Do you pay via a Claude subscription
(Pro/Max) or per usage (Enterprise/API)?", and writes `mode` accordingly,
with usage-mode dollar defaults only in the second case. Invalid `mode`
values fall back to `auto` and log `config_parse_failed`.

## Code changes

- `src/config.rs`: `BillingMode` enum; `BudgetConfig.mode`;
  `Option<f64>` limits resolved at render time; `plan_limits` alias in
  `parse_budget`; template + defaults test updated.
- `src/payload.rs`: `PlanUsage` gains the two `resets_at` fields; reuse
  `lenient_option` so wrong-typed values degrade.
- `src/cost.rs`: replace `plan_pct`'s bool gate with a per-window
  resolution function returning `Plan(pct)`, `Spend(usd)`, `Budget{..}`, or
  `Hidden`; add a plain-spend renderer without bar or color.
- `src/setup.rs`: the mode question.
- `tests/cli.rs`: end-to-end cases listed below.

## Tests

Unit (beside the code):

- Mode resolution matrix: 3 modes x payload has/lacks window x limit
  set/unset, asserting Plan / Spend / Budget / Hidden.
- The 2026-11-25 regression: `auto`, payload without `rate_limits`,
  `block5h_usd` unset, cache `block5h_usd = 88.96` renders no `5h` bar and
  never a value above 100% in red.
- `plan_limits` alias maps `true`/`false` and loses to `mode`.
- Invalid `mode` string falls back to `auto`.
- `resets_at` wrong-typed, negative, and missing all yield `None`.

End to end (`tests/cli.rs`): payload with and without `rate_limits` under
each mode; empty and non-JSON stdin still print nothing and exit 0.

## Out of scope

- Fetching quota or credits from Anthropic, OpenAI or OpenRouter APIs (see
  the earlier research: only OpenRouter exposes remaining credit, and it
  would need an HTTP dependency).
- Per-repo or per-team budgets for Enterprise.
- Showing plan percentage and dollar spend side by side in `subscription`
  mode beyond the opt-in `5h` spend figure.

## Open questions

1. Whether Claude Code 2.1.332 sends `rate_limits` for this account at all.
   Resolve by capturing a real payload with `FERRISBAR_CAPTURE_PAYLOAD`
   before implementing. If it never does for subscribers, `subscription`
   mode would show only plain spend, and the right answer may be to read
   the percentages from another source.
2. Whether `spend_limit` (listed in Claude Code's statusline docs alongside
   `five_hour` and `seven_day`) is the quota an Enterprise seat sees. If so,
   `usage` mode could prefer it over the self-imposed budget. Needs a real
   Enterprise payload.
