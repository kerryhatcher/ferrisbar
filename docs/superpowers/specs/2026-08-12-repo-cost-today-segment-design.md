# ferrisbar — per-repo "today" cost segment on the statusline

## Purpose

Show how much of today's Claude Code spend happened in the *current*
repository, right on the statusline's existing daily-cost line, using the
per-repo analytics store the previous feature already builds and
maintains.

## Background

`src/cost.rs`'s `daily_chip` already renders the statusline's second
line — today's total cost across every session, everywhere, plus a
per-model breakdown:

```
$204.10 today (Opus $135 · Sonnet $69)
```

Separately, the optional `analytics` feature (opt-in via `[analytics]
enabled = true` plus a build with `--features analytics`) already
records exactly this data broken out by repository: the same background
refresh that recomputes the chip above also buckets cost into a redb
table keyed by `(date, repo_key, model)`, and a `ferrisbar report`
subcommand reads it back out for historical/scripted use.

This spec wires the two together: read today's total for whichever repo
the current render's `cwd` resolves to, and append it to the existing
daily-cost line when available.

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| Time scope | Today only — matches the line's existing "today" framing, not an all-time total |
| Label | `repo $X.XX` — generic label, doesn't repeat the repo's display name (already shown as the folder name on line 1) |
| Config toggle | None — follows `[analytics] enabled` with no separate switch |
| Data source | Read the redb store directly, live, on the render path (not folded into the existing `cost-cache.json`) |

The data-source decision is a deliberate departure from the rest of this
project's pattern of only ever reading pre-computed caches on the render
path (redb has otherwise only ever been touched by the detached
background-refresh process and the standalone `report` command). It
was chosen explicitly over extending `cost-cache.json` with a
per-repo map, which would have kept that invariant intact at the cost of
a slightly more roundabout data flow. Section "Error handling" below
covers why this is still safe for the render path.

## Scope

### In scope

- `analytics::store::today_repo_cost(enabled, data_dir, repo_key) ->
  Option<f64>` — dual-build (real redb query / no-op stub), matching
  `Sink`'s existing pattern
- `cost::daily_chip` gains `cwd: &str` and `analytics_enabled: bool`
  parameters; internally resolves repo identity, queries
  `today_repo_cost`, and appends the segment when positive
- `main.rs`'s call site updated with the two new arguments
- Unit tests for the query function and the new formatting branch;
  one end-to-end test in `tests/cli.rs`

### Out of scope

- Any change to `ferrisbar report`'s existing behavior or output
- An all-time/lifetime repo total anywhere on the statusline
- A dedicated config toggle for this segment
- Any change to how the background refresh writes to redb (this spec
  only adds a *reader*)

## Architecture

`src/analytics/store.rs` gains:

```rust
pub fn today_repo_cost(enabled: bool, data_dir: &Path, repo_key: &str) -> Option<f64>
```

under `#[cfg(feature = "analytics")]`, with an identical-signature stub
(`enabled`/`data_dir`/`repo_key` all unused, always `None`) under
`#[cfg(not(feature = "analytics"))]` — the same shape `Sink` already
uses, so `cost.rs` never needs its own `#[cfg]` to call it.

`enabled = false` short-circuits to `None` before any I/O, so a
disabled/not-compiled-in build pays nothing extra on the render path.
The real implementation:

1. Computes today's UTC date string (reusing `cost.rs`'s existing
   `today_utc_date`/`now_unix_secs`, exposed as `pub` for this purpose —
   `store.rs` already depends on `cost.rs` for `ParsedRecord`, so this
   doesn't introduce a new dependency direction).
2. Opens the redb file read-only (`Database::open`, never `create`).
3. Sums `cost_usd` across every row whose decoded key matches today's
   date and the given `repo_key`, across any model.
4. Degrades to `None` at every fallible step (missing file, corrupt
   file, unreadable table, undecodable row) — matching the existing
   `let Ok(...) else { return None }` pattern in `store.rs`/`report.rs`.

`cost::daily_chip`'s signature becomes:

```rust
pub fn daily_chip(cfg: &CostConfig, data_dir: Option<&Path>, cwd: &str, analytics_enabled: bool) -> Option<String>
```

When `analytics_enabled` is true and `data_dir` is `Some`, it resolves
`repo_identity::resolve(cwd)` (cheap — no I/O beyond `.git/config`,
already the same cost `git::branch_name` pays on every render) and
calls `today_repo_cost`. `main.rs`'s single call site passes the two new
arguments straight through; no other render-path code changes.

## Formatting

Appended to the existing chip string, separated by `" │ "`:

```
$204.10 today (Opus $135 · Sonnet $69) │ repo $12
```

- Only appended when `today_repo_cost` returns `Some(cost)` and
  `cost > 0.0` — a `$0.00` segment (possible if the repo's only activity
  today was an unpriced model) is treated as "nothing to show," the same
  as `None`.
- Uses the chip's existing cents/whole-dollar rule: sub-dollar amounts
  keep cents, `$1` and above round to whole dollars.
- No separate toggle: shown whenever `[analytics] enabled = true`, the
  binary was built with the `analytics` feature, and today's repo total
  is positive.

## Error handling

- Every fallible step in `today_repo_cost` degrades to `None`, never a
  panic or a render failure — consistent with the project's
  never-panic-on-input invariant.
- `repo_identity::resolve` already never fails by design (falls back to
  a `local:` identity), so no new failure mode there.
- A slow or failing redb open cannot block or corrupt the rest of the
  statusline: worst case, this one segment is silently absent for that
  render while everything else prints normally.
- No new background job and no new lock file, but this is not entirely
  lock-free: `redb::Database::open` takes an exclusive advisory file
  lock (this version of redb has no read-only open mode), so it does
  contend with the existing refresh's writer, and if a refresh process
  were ever killed mid-commit, the next `open` — including this one —
  can run synchronous repair before returning. Both are non-blocking in
  the sense that matters (no wait/retry loop here, and a lost lock race
  or mid-repair open just yields `None` for that render) and
  self-healing (the next refresh fully recomputes and overwrites), so
  this stays a pure reader that never mutates the store, just not one
  that is fully isolated from the writer's lock.

## Testing

- `store.rs`: `today_repo_cost` sums correctly across multiple models
  for the same repo/day; returns `None` for a different repo or date;
  returns `None` for a missing file; returns `None` (with no directory
  created) when `enabled = false`.
- `cost.rs`: the new formatting branch appends the segment when cost is
  positive, stays absent when `None` or exactly `0.0`, and formats
  cents/whole-dollars the same way the existing breakdown does.
- `tests/cli.rs`: extends the existing ingestion fixture (real repo,
  real transcript, real `--internal-refresh-daily-cost`) to assert the
  rendered statusline's second line contains `repo $X.XX` when rendered
  with `cwd` set to that repo.
