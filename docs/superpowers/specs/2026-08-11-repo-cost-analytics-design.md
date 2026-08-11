# ferrisbar — per-repo cost analytics datastore

## Purpose

Let a user track Claude Code spend, broken down by day and by model, per
git repository, so they can judge the ROI of using Claude Code on a given
repo over time. Add a `ferrisbar report` subcommand that exports that
history as JSON or CSV.

## Background

ferrisbar already estimates cost from Claude Code's own transcripts
(`src/cost.rs`) to render a "$X today" statusline chip. That aggregate is
global — it walks every transcript under `<claude config dir>/projects/`
and sums cost across every repo the user has ever worked in, cached for a
TTL and recomputed by a detached background refresh
(`src/cost_cache.rs`) so no render ever blocks on the file walk.

This spec adds a second, persistent aggregate: the same underlying data,
but bucketed by repository identity instead of collapsed into one global
number, kept over time instead of discarded each day. Each transcript
line already carries the absolute `cwd` Claude Code was run from, which
is enough to resolve which repository a given render belonged to.

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| Storage engine | Pure-Rust embedded DB (`redb`), not `rusqlite`/Diesel — both pull in the real SQLite C library and a C toolchain, which conflicts with `CLAUDE.md`'s existing no-C-toolchain stance (see the `flate2` `rust_backend` comment) |
| Scope of this spec | Datastore + ingestion + `ferrisbar report` in JSON/CSV only. HTML/PDF export is a separate fast-follow spec |
| Repo identity | Normalized `origin` remote URL (`remote:host/path`, lowercased, `.git` stripped, ssh/https forms unified); no remote → repo root folder name (`local:<name>`) |
| Backfill | None — forward-only from when the feature is enabled. Re-walking full transcript history on every refresh would defeat the reason the TTL/cache mechanism exists |
| Row content | Cost *and* raw token counts (input/output/cache read/write), not cost alone — enables re-pricing and token-volume reporting later |
| Ingestion trigger | Extend the existing background refresh (`cost_cache::spawn_refresh`) rather than a second job or on-demand-only ingestion — one file walk, reuses the existing lock/TTL machinery |
| Enablement | Off by default even when compiled in — a Cargo feature (`analytics`) makes recording *possible*; `[analytics] enabled = true` in the config file turns it on |
| DB location | ferrisbar's own `data_dir` (alongside `cost-cache.json` and `logs/`), not Claude's `~/.claude` config dir |
| `ferrisbar report` default scope | Current repo (resolved from cwd), full tracked history |
| `ferrisbar report` other scopes | `--repo <key>`, `--from`/`--to`, `--all` (cross-repo summary) |

## Scope

### In scope

- `analytics` Cargo feature gating a new optional `redb` dependency
- `[analytics] enabled` config key (default `false`)
- Repo identity resolution from a transcript record's `cwd`
- Extending the existing background refresh to upsert per-repo,
  per-model, per-day rows (today + yesterday, same pass)
- `ferrisbar report` subcommand: `--repo`, `--from`, `--to`, `--all`,
  `--format json|csv`
- Unit tests beside the code; CLI end-to-end tests in `tests/cli.rs`,
  covering both feature-on and feature-off builds

### Out of scope

- HTML/PDF export (separate spec)
- Backfilling history that predates enabling the feature
- Cross-machine merging of analytics databases
- Per-branch or per-session breakdown (repo+date+model granularity only)
- A `--output <path>` flag — output goes to stdout; users redirect

## Architecture

- New optional dependency `redb` (pure-Rust embedded table store — no C
  toolchain), gated behind a new Cargo feature `analytics`, off by
  default. Default builds are unaffected.
- New config section:
  ```toml
  [analytics]
  enabled = false
  ```
  Recording happens only when the binary was built with the `analytics`
  feature **and** this is `true`. `ferrisbar report` is compiled only
  under the same feature, and reads whatever the store already has
  regardless of the current `enabled` value — turning recording off
  does not hide previously collected history.
- New module `src/analytics.rs` (split into a submodule if it grows),
  entirely `#[cfg(feature = "analytics")]`. No behavior change to any
  existing code path when the feature is off.
- DB file: `<data_dir>/analytics.redb`.
- Row values are serialized as JSON bytes inside redb tables via the
  existing `serde_json` dependency — no additional serialization crate
  needed.

## Data model

One redb table, keyed by `(date, repo_key, model)`. Value holds:

| Field | Type | Notes |
|---|---|---|
| `repo_display` | `String` | Human-readable form of the identity |
| `cost_usd` | `f64` | Estimated cost, same pricing table as `cost.rs` |
| `input_tokens` | `u64` | |
| `output_tokens` | `u64` | |
| `cache_creation_tokens` | `u64` | |
| `cache_read_tokens` | `u64` | |

`date` is the UTC calendar date (`YYYY-MM-DD`), matching `cost.rs`'s
existing convention.

## Repo identity resolution

Given a transcript record's `cwd`:

1. Walk up from `cwd` looking for a `.git` entry (reusing `git.rs`'s
   existing directory walk-up logic).
2. Read `.git/config`; look for `[remote "origin"]`'s `url`. Normalize:
   - `git@host:path.git`, `ssh://git@host/path.git`, and
     `https://host/path.git` all collapse to `host/path`
   - host is lowercased, a trailing `.git` is stripped
   - `repo_key = "remote:{host/path}"`, `repo_display` = the same string
3. No `.git` found, or no `origin` remote configured:
   - `repo_key = "local:{root-folder-name}"`, `repo_display` = that
     folder name
   - the `remote:`/`local:` prefix keeps the two namespaces from
     colliding (a local folder that happens to be named like a
     `host/path` string still can't collide with a real remote key)

Resolution is cached per unique `cwd` string for the duration of one
ingestion pass — many transcript lines share the same `cwd`.

## Ingestion mechanics

Extends the existing background refresh
(`cost_cache::spawn_refresh` → hidden `--internal-refresh-daily-cost`)
rather than adding a second job:

- The transcript walk that already computes the global "today" chip
  totals (`aggregate_windows` in `src/cost.rs`) also, when analytics is
  enabled, resolves each record's repo identity and upserts into the
  analytics table in the same pass.
- Buckets into **both today's and yesterday's** UTC date, not just
  today — closes the day-boundary gap where the last refresh before
  midnight might miss the final minutes of "yesterday," since nothing
  else ever revisits a date once it's neither today nor yesterday.
- Gated behind `#[cfg(feature = "analytics")]` so the non-analytics
  build's hot path is untouched.
- Never walks further back than yesterday — consistent with the
  forward-only decision, and avoids reintroducing the expensive
  full-history walk the TTL/cache design exists to prevent.

## CLI reporting

New `ferrisbar report` subcommand, compiled only under the `analytics`
feature:

- No args → resolve repo identity from `cwd` (same logic as ingestion),
  report that repo's full tracked history as raw rows (one per
  date/model), sorted by date.
- `--repo <key>` → same raw-row report for an explicit `repo_key`,
  runnable from anywhere.
- `--from <date> --to <date>` → bounds the date range on either the
  default or `--repo`-selected report.
- `--all` → one summary row per tracked repo (total cost, total tokens
  across the date range), for side-by-side ROI comparison, instead of
  raw per-date-per-model rows.
- `--format json|csv` (default `json`), printed to stdout.

## Error handling

Follows the existing never-panic invariant:

- A missing/corrupt `.git`, unreadable `.git/config`, or unparseable
  remote URL degrades to the `local:` fallback identity — never a
  failure that aborts the refresh.
- A redb open/write/schema error during ingestion is swallowed (matching
  how `cost_cache::write_cache` failures are already swallowed) — a
  failed analytics write must never break the cost-chip refresh it
  piggybacks on.
- `ferrisbar report` against a missing/unreadable/corrupt DB file, or a
  `--repo` key with no rows, prints an empty result set (`[]` for JSON,
  header-only for CSV) and exits `0` — "no data yet" is a normal state
  for a freshly enabled feature, not an error.
- Malformed transcript records already degrade to "not counted" per
  `parse_line`'s existing behavior; analytics ingestion inherits that
  for free, since it reads the same parsed records.

## Testing

- Unit tests beside the code (`mod tests`):
  - Repo identity normalization: ssh/https/bare URL forms resolving to
    the same key; no-remote fallback; nested-cwd walk-up.
  - Ingestion upsert: today+yesterday bucketing, idempotent re-upsert
    (running the refresh twice the same day doesn't double-count),
    multiple repos/models in one pass.
  - Report filtering: `--repo`, `--from`/`--to`, `--all` summarization,
    JSON/CSV output shape.
- End-to-end coverage in `tests/cli.rs`: `ferrisbar report` against a
  fixture DB, for both feature-on and feature-off builds (the
  feature-off build must not expose the subcommand at all).

## Dependency and supply-chain impact

- Adds `redb` as an optional runtime dependency, gated behind the
  `analytics` feature. Needs a `cargo vet` entry in `supply-chain/` per
  `CLAUDE.md`'s "a fifth dependency needs a justification" rule, scoped
  to when the feature is built.
- No C toolchain requirement introduced — `redb` is pure Rust.
