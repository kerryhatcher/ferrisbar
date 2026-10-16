# ferrisbar — multi-app cost analytics (Claude Code + pi)

## Purpose

Let a user attribute cost/usage not just by day, repo, and model, but also
by the **app that produced it** — today Claude Code, and now
[pi](https://github.com/earendil-works/pi-coding-agent). A user driving both
agents against the same repositories should be able to answer "how much did
Claude Code cost me on repo X this week vs. pi?" from `ferrisbar report`.

## Background

ferrisbar already estimates cost from Claude Code's transcripts
(`src/cost.rs`) and, with the `analytics` Cargo feature, persists a per-repo,
per-day, per-model history to an embedded `redb` store
(`src/analytics/store.rs`), surfaced by `ferrisbar report`
(`src/analytics/report.rs`). See
`2026-08-11-repo-cost-analytics-design.md` for that datastore's design.

That pipeline is hardwired to one producer:

```
main.rs: resolve_transcripts_dir()  →  <claude config dir>/projects/   (Claude Code only)
      ↓
cost.rs: refresh_daily_cache(root, data_dir, analytics_enabled)
      ↓  discover_transcripts(root)   → every **/*.jsonl
      ↓  parse_line(line)             → ParsedRecord { usage, model, date, timestamp_unix, dedup_key, cwd }
      ↓  analytics.record(rec, cost)
store.rs: key = "date\0repo_key\0model"  →  Row { repo_display, cost_usd, tokens... }
```

Two assumptions block a second app: the transcript **shape** is
Claude-Code-specific, and the store **key** has no app dimension.

One property of that store matters throughout this spec: it is
**append-only and never pruned**. `Sink::record` drops any record whose date
is neither today nor yesterday (`store.rs:93`), and `flush` inserts only the
keys the current pass touched — it never deletes
(`store.rs:122-147`). Nothing anywhere revisits an older date; the prior
design states this outright ("Never walks further back than yesterday",
"Backfill: None"). There is no TTL on stored rows. Any row that becomes
unreachable is therefore lost permanently, which is why this spec does not
change the table name (§5).

## The format mismatch (Claude Code vs pi)

| | Claude Code (`~/.claude/projects/**/*.jsonl`) | pi (`~/.pi/agent/sessions/<encoded-cwd>/*.jsonl`) |
|---|---|---|
| record model | every line is a usage record | mixed lines; `{"type":"message"}` with `message.role == "assistant"` carry usage |
| usage keys | `input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens` | `input`, `output`, `cacheWrite`, `cacheWrite1h`, `cacheRead` (+ `reasoning`, `totalTokens`, and a pre-computed nested `cost`) |
| usage-bearing lines | every line | `{"type":"message"}` (assistant). A `{"type":"compaction"}` entry is believed to exist and to carry summary-generation usage, but no real example has been captured — see Known limitations |
| `cwd` | present on **every** record | present **only** on the `{"type":"session"}` header line |
| dedup id | `message.id` + `requestId` (globally unique) | per-entry `id` — only **8 hex chars** (~32 bits). Assumed unique within a file; cross-file uniqueness is **not** guaranteed by anything observable (see below) |
| session id | n/a | the `{"type":"session"}` header's `id` is a full UUID, also embedded in the filename |
| provider | implicit (Anthropic) | explicit `provider` inside `message` (`anthropic`, `ollama`, and any OpenAI-compatible endpoint via `api: "openai-completions"`) |
| model id | `claude-sonnet-5` | `claude-opus-4-8` for Anthropic — same scheme, `pricing.json` prefix match still resolves. Non-Anthropic ids (`gemma4:12b-mlx`, `gpt-…`) resolve to nothing |
| timestamp | top-level ISO8601 (`YYYY-MM-DDTHH:MM:SS…Z`) | top-level ISO8601 — **same**, `parse_iso8601_utc` works unchanged |

On cross-file uniqueness of the 8-hex entry `id`: a survey of the real pi
sessions on the author's machine (9 sessions, 456 entries, 447 of them
carrying an 8-hex `id`) found **no** duplicate id across files. That is
evidence of nothing — 447 draws from a 32-bit space would rarely collide by
chance regardless. Treat cross-file uniqueness as unproven, which is why the
dedup key is session-scoped (§4).

Representative pi lines (real shape, `cwd` and content elided):

```jsonl
{"type":"session","version":3,"id":"019ff77b-d3c3-7dcd-a1d0-1f90cebf956a","timestamp":"2026-08-12T19:38:35.331Z","cwd":"/Users/dev/repo"}
{"type":"message","id":"9d0e1ca6","parentId":"8b417fdc","timestamp":"2026-08-12T19:40:03.469Z","message":{"role":"assistant","content":[…],"api":"openai-completions","provider":"anthropic","model":"claude-opus-4-8","usage":{"input":1373,"output":76,"cacheRead":0,"cacheWrite":0,"cacheWrite1h":0,"reasoning":0,"totalTokens":1449,"cost":{"input":0.006865,"output":0.0019,"cacheRead":0,"cacheWrite":0,"total":0.008765}}}}
```

Note `parentId`: pi entries form a branchable tree, so an entry being copied
into a second file (a resume, a branch, a rotation) is plausible — the same
situation Claude Code's own dedup exists to handle
(`cost.rs:1028`: "Same message resumed into a second transcript — must not
double-count").

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| New dimension | Add `app` as a first-class key component (`Source { ClaudeCode, Pi }`), not a repo-key prefix hack — keeps repo identity clean and lets `report` group/filter by app |
| Store key shape | Extend from `date\0repo_key\0model` to `date\0repo_key\0model\0app` |
| Legacy rows | **Keep the existing `cost_rows` table** and make `decode_key` accept both 3-part (legacy, app defaults to `claude-code`) and 4-part keys. Bumping the table name was considered and rejected: the store has no TTL and no pruning, so a new table would not "repopulate" — it would silently strand every row older than yesterday, permanently, on the first upgraded run. For a feature whose purpose is judging repo ROI *over time*, that is the worst possible upgrade outcome. Bloat is not a counter-argument: rows are ~150 bytes, so a year of history is well under a megabyte |
| Cost source | Price from `pricing.json` whenever `resolve_model` matches, ignoring pi's own `cost.total`. Cross-app comparison of the same model must be apples-to-apples, and raw tokens are stored either way so re-pricing stays possible. When `resolve_model` does **not** match, the record is stored **unpriced** (tokens kept, cost 0) via the existing unpriced path — the same treatment Claude gives an unknown model. This applies to *any* model absent from `pricing.json`, not just local/free ones; see Known limitations, which also records the rejected `cost.total` fallback |
| pi model ids | Rely on existing `resolve_model` prefix/alias matching. No `pricing.json` change in this spec. Provider is **not** part of the key (pricing is prefix-matched on model id); a provider dimension is a possible future addition, not modeled here |
| Compaction cost | **Out of scope for this spec.** pi is believed to emit `{"type":"compaction"}` entries carrying usage for the summary LLM call, but no real example has been captured, so neither the field path for its model nor the shape of its usage block is known. Shipping a fixture for a guessed shape would give false confidence while real records got dropped. Accepted as a documented undercount until a real example exists |
| Reader design | Introduce a `Source`-tagged reader step. Claude Code keeps its stateless per-line `parse_line`; pi gets a **stateful per-file** parser (carry `cwd` and session id from the header, normalize usage key names) built on the *same* resilient line-reading loop `aggregate_windows` already uses (§2) |
| Dedup across apps | Keep per-app dedup. pi's key is `pi\0<session-id>\0<entry-id>`, using the session **UUID from the header line**, not the file stem: it is exactly as unique (the stem embeds that same UUID), costs nothing (the parser reads that line for `cwd` anyway), and unlike the stem it survives a rename, rotation, or copy of the file — which is precisely the case where file-scoping would double-count real money (§4) |
| Enablement | pi ingestion is opt-in on top of the existing `analytics` feature + `[analytics] enabled`. New `[analytics] pi_enabled = false` default, plus `pi_sessions_dir` override |
| Feature gating | `Source`, `parse_pi_file`, and `aggregate_windows`'s dispatch on `Source` compile in **every** build (no `#[cfg]` in the hot loop), mirroring the existing no-op-`Sink` precedent. Only the *construction of the pi root* in `main.rs` is `#[cfg(feature = "analytics")]`, so a default build never walks the pi tree (§8) |
| Roots | `aggregate_windows` walks a `Vec<(PathBuf, Source)>` instead of one root, so adding a third app later is additive. `main.rs` owns building that vec (§3) |
| Backfill | None — forward-only from enablement, matching the existing analytics design |

## Scope

In scope:
- `app` dimension end-to-end: `ParsedRecord`, store key, `Row` grouping, `report`.
- A pi transcript reader (stateful cwd/session-id carry-forward, usage-key
  normalization, assistant-message usage).
- Multi-root ingestion in `aggregate_windows` / `refresh_daily_cache`, with the
  visible `WindowTotals` kept Claude-only (see §3).
- Backward-compatible key decoding plus legacy-key cleanup on write (§5).
- Config: pi opt-in + sessions-dir override; pi sessions root resolution.
- `ferrisbar report`: `app` column (JSON/CSV), validated `--app` filter,
  per-app summary rows.

Out of scope:
- The visible statusline cost chips (`daily_chip`, `budget_line`) stay
  Claude-Code-only for now: even when `pi_enabled`, pi records go to the
  persistent store but **not** into the `WindowTotals` that feed the chips, so
  enabling pi ingestion does not change the rendered statusline. `aggregate_windows`
  produces both outputs in one pass, so this requires an explicit `Source` guard
  on the window accumulation (§3) — not a no-op. A per-app / cross-app chip is a
  fast-follow.
- pi `{"type":"compaction"}` entries (see Known limitations).
- Trusting pi's precomputed `cost.total`, including as a fallback for unpriced
  models (rejected in Known limitations).
- Adding non-Anthropic model rates to `pricing.json`.
- A provider dimension in the store key (pricing is model-id prefix-matched).
- Any change to the `ferrisbar-pi` pi extension; this is entirely ferrisbar-side.

## Design

### 1. `Source` on `ParsedRecord` (`src/cost.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source { ClaudeCode, Pi }

impl Source {
    pub const fn key(self) -> &'static str {
        match self { Self::ClaudeCode => "claude-code", Self::Pi => "pi" }
    }
}

// Only `report` parses a key back into a `Source`, so this follows
// `ParsedRecord::for_test`'s existing gating pattern rather than earning a
// dead-code warning in a default build.
#[cfg(feature = "analytics")]
impl Source {
    pub fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "claude-code" => Some(Self::ClaudeCode),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
}
```

Add `pub app: Source` to `ParsedRecord`. `parse_line` (Claude Code) sets
`Source::ClaudeCode` and prefixes its `dedup_key` with `Source::ClaudeCode.key()`
(see §4). `ParsedRecord::for_test` gains an `app` argument.

Note the no-op `Sink` in `src/analytics.rs` needs no signature change —
`app` rides inside `rec`, and `record(&mut self, rec, cost)` is unchanged.

### 2. pi reader (`src/cost.rs`)

`parse_line` stays as the Claude Code stateless parser. Add a pi parser that
is **per-file and stateful**, because `cwd` and the session id live only on
the header line.

First, extract `aggregate_windows`'s existing line-reading loop
(`cost.rs:466-491`) into a shared helper — something like
`for_each_line(path, |line| …)` — and have both readers use it. That loop is
deliberately hand-written: it survives a non-UTF-8 or unreadable line without
abandoning the rest of the file, and it bounds consecutive errors
(`MAX_CONSECUTIVE_LINE_ERRORS`, `cost.rs:34`) so a persistently-erroring
reader cannot spin forever. Both properties have named regression tests. A
fresh naive read for pi (`read_to_string`, or
`.lines().map_while(Result::ok)`) would silently reintroduce both bugs for pi
only, which is why this is an extraction, not a second implementation.

```rust
fn parse_pi_file(path: &Path) -> Vec<ParsedRecord> {
    // 1. Session header: the **first line whose `type` is `"session"`** —
    //    not "line 1" — read for `cwd` and `id`. No such line, or no
    //    `cwd` on it: skip the whole file (nothing to attribute).
    // 2. For each `{"type":"message"}` entry with message.role == "assistant"
    //    and a usage object, map:
    //      input                      → input_tokens
    //      output                     → output_tokens
    //      cacheWrite + cacheWrite1h  → cache_creation_tokens
    //      cacheRead                  → cache_read_tokens
    //      (reasoning / totalTokens are ignored: not priced fields)
    //    Sum the two cacheWrite variants with `saturating_add` — untrusted
    //    input, same reasoning as `Row::accumulate`. See Known limitations
    //    for why both land in one bucket.
    //    model  ← message.model
    //    date   ← top-level timestamp, sliced with `.get(..10)?`
    //    dedup  ← session-scoped key (see §4)
    //    cwd    ← header cwd
    //    app    ← Source::Pi
    // 3. Skip all-zero usage (reuse Usage::is_empty).
}
```

`.get(..10)`, never `timestamp[..10]`: byte offset 10 is not guaranteed to
land on a UTF-8 char boundary, and slicing off a boundary panics.
`parse_line` already uses `.get(..10)` for exactly this reason
(`cost.rs:246-251`) and has a named regression test for it
(`parse_line_does_not_panic_on_a_non_char_boundary_timestamp`,
`cost.rs:919`). The pi reader must not reintroduce the panicking form.

Same never-panic-on-input discipline as `parse_line`: unreadable file,
missing/`cwd`-less header, malformed lines → skip, don't raise.

The `Vec<ParsedRecord>` return is bounded by one session file, which is the
same working-set size the Claude path already holds transiently.

### 3. Multi-root aggregation (`src/cost.rs`, `src/main.rs`)

- Change `aggregate_windows(transcripts_root, …)` to
  `aggregate_windows(roots: &[(PathBuf, Source)], …)`.
- For a `ClaudeCode` root: `discover_transcripts` + per-line `parse_line`.
  For a `Pi` root: `discover_transcripts` + `parse_pi_file`.
- **Keep the visible `WindowTotals` Claude-only.** `aggregate_windows` emits
  both `analytics.record(&rec, cost)` (all apps → persistent store) *and* the
  `daily_total`/weekly/monthly/block5h sums that feed the statusline chips.
  Every record is passed to `analytics.record`; immediately after that call,
  alongside the existing `if cost <= 0.0 { continue; }` guard
  (`cost.rs:514-516`), add `if rec.app != Source::ClaudeCode { continue; }`.
  Enabling pi ingestion then never changes the rendered chip (per Scope).
  Drop this guard in the fast-follow that introduces a cross-app chip.
- **`main.rs` owns the roots vec**, not `refresh_daily_cache`. `main.rs`
  already resolves the Claude root today (`resolve_transcripts_dir`, called
  at `main.rs:92-96`) and `refresh_daily_cache(&Path, &Path, bool)` takes no
  config struct at all, so threading `Config` into it just to build a path
  list would be the larger change. `main.rs` gains
  `resolve_pi_sessions_dir(cfg)` (§7) and builds
  `vec![(claude_root, Source::ClaudeCode)]`, pushing
  `(pi_root, Source::Pi)` only under §8's gate.
- `refresh_daily_cache`'s public signature becomes
  `refresh_daily_cache(roots: &[(PathBuf, Source)], data_dir: &Path, analytics_enabled: bool)`.
  This ripples into its `main.rs` call site and into three existing tests
  (`cost.rs:1393`, `:1420`, `:1446`) plus the five `aggregate_windows` tests
  (`cost.rs:1047`, `:1082`, `:1108`, `:1144`, `:1180`).

### 4. Dedup across apps (`src/cost.rs`)

The `seen` set in `aggregate_windows` currently holds raw dedup keys. Claude's
key (`message.id:requestId`) is globally unique. pi's entry `id` is 8 hex
chars, so a bare app prefix is not enough.

There are two failure modes, and they pull in opposite directions:

- **Too coarse** (two distinct entries share a key) → one is dropped →
  *undercount*.
- **Too fine** (one logical entry appears under two keys) → both are counted
  → *double-count real money*. This is the worse of the two, and it is the
  one file-scoping causes: pi entries carry `parentId`, so the tree is
  branchable, and Claude Code's own dedup exists because a resume copies
  messages into a second transcript. If pi ever copies, rotates, or renames a
  session's entries into a second file, a file-stem-scoped key makes the
  copies look distinct.

Scope the key to the **session UUID from the header line**, which avoids
both. It is exactly as unique as the file stem (the stem embeds that same
UUID — `2026-08-12T19-38-35-331Z_019ff77b-….jsonl` vs. the header's
`"id":"019ff77b-…"`), it costs nothing extra since the parser reads that line
for `cwd` regardless, and it is stable across renames and rotation.

```rust
// pi: app + session UUID (from the header line) + entry id
rec.dedup_key = Some(format!("{}\0{}\0{}", Source::Pi.key(), session_id, entry_id));
```

Claude's `dedup_key` is likewise prefixed with `Source::ClaudeCode.key()` in
`parse_line`, so a Claude `msg:req` and a pi key can never collide or cancel.
The `seen` set then stores these already-qualified keys verbatim — one
construction site per app, no second prefixing step in `aggregate_windows`.

### 5. Store key + Row (`src/analytics/store.rs`)

- **`TABLE` keeps its existing name (`cost_rows`).** No table bump: the store
  is never pruned, so a new table would strand all history older than
  yesterday rather than "repopulating" (see Background and the Legacy rows
  decision).
- `encode_key(date, repo_key, model, app) -> "date\0repo_key\0model\0app"`.
- `decode_key` becomes a compatibility shim: it accepts **3 or 4** parts and
  rejects any other count.
  - 4 parts → `(date, repo_key, model, app)` as written.
  - 3 parts → `(date, repo_key, model, "claude-code")`. Every pre-existing row
    was written by the Claude-only pipeline, so that default is exact, not a
    guess.
  - Anything else → `None`, keeping today's defensiveness against a
    differently-shaped future store.
- `Sink::record` reads `rec.app.key()` for the 4th key component.
- `Sink::flush` additionally calls `table.remove()` on the legacy 3-part key
  `date\0repo_key\0model` for each 4-part key it writes **whose app component
  is `claude-code`**. Without this, today's and yesterday's Claude rows would
  exist under both shapes and double-count in `report --all`, since
  `decode_key` now reads both. The pass has just recomputed that day's full
  total, so the removal loses nothing.
  - Restricting removal to `claude-code` rows is deliberate: legacy rows are
    all Claude by definition, and removing on a `pi` write could delete a
    legacy Claude row that this pass did *not* regenerate (e.g. the Claude
    root was unreadable while the pi root was fine).
  - Dates older than yesterday only ever have the 3-part shape, so they
    cannot double-count.
- `Row` is unchanged (app lives in the key, not the value).

### 6. Report (`src/analytics/report.rs`)

- `decode_key` now yields `(date, repo_key, model, app)`; `ReportRow` gains
  `app`. Legacy rows arrive as `claude-code`, so no stored history disappears
  from `report`.
- `JsonRow` and the CSV header gain an `app` field, **appended as the last
  column/field** in both raw-row and `--all` output. Appending rather than
  inserting after `model` is a deliberate call: it shifts no existing CSV
  column index, so a consumer script reading `$5` for `cost_usd` keeps
  working. One rule for both formats keeps JSON and CSV field order aligned.
- `Options` gains `app: Option<Source>`; `parse_args` learns `--app` and
  **validates it**, erroring like `--format` already does
  (`report.rs:69-71`) rather than accepting an unvalidated string. `--app
  claude` (instead of the stored `claude-code`) must be an error, not a
  silently empty result set:

  ```rust
  "--app" => {
      let value = next_value(args, &mut i, "--app")?;
      opts.app = Some(Source::from_key(&value).ok_or_else(|| {
          format!("unknown --app {value} (expected claude-code or pi)")
      })?);
  }
  ```

- Raw-row scope filters on `app` when set (comparing `r.app` to
  `source.key()`).
- `--all` **groups by the `(repo_key, app)` tuple** — one summary row per
  repo per app, which is the cross-app comparison this whole spec exists to
  enable. (A separate per-app rollup is not added; `--all` output already
  carries `app`, so a consumer can roll up itself.) A `(String, String)`
  `HashMap` key preserves the existing deterministic cost-tie tie-break for
  free, since tuple `Ord` compares `repo_key` then `app`
  (`report.rs:238-243`).

### 7. Config (`src/config.rs`, `src/paths.rs`, sample `config.toml`)

New keys under `[analytics]`:

```toml
[analytics]
enabled = true          # existing
pi_enabled = false      # opt in to pi ingestion
pi_sessions_dir = ""    # optional absolute override; default ~/.pi/agent/sessions
```

Parsed in `parse_analytics` (`config.rs:406`). Three mechanical consequences
the implementer must handle:

- **`AnalyticsConfig` can no longer derive `Copy`.** It currently derives
  `Debug, Clone, Copy, PartialEq, Eq, Default` (`config.rs:125-128`); adding
  a `String` field makes `Copy` impossible. Drop `Copy`, keep the rest, and
  fix any by-value use sites the compiler flags.
- **`paths::usable_dir` is private** (`paths.rs:10`). The absolute-path
  validation for `pi_sessions_dir` should reuse it, so make it `pub` rather
  than duplicating the check.
- **Name the default-path resolver.** `~/.pi/agent/sessions` has no existing
  cross-platform resolution to lean on: `config_dir.rs` reads `HOME` only,
  and `paths.rs::base_vars` reads `APPDATA`/`LOCALAPPDATA` on Windows —
  nothing reads `USERPROFILE`. Add `paths::pi_sessions_dir() -> Option<PathBuf>`
  that resolves the home base from `HOME`, falling back to `USERPROFILE`, and
  returns `None` when neither is set. Returning `None` (pi ingestion stays
  off) is required: joining onto an empty base would yield the relative path
  `.pi/agent/sessions` inside whatever repo Claude Code is running in —
  exactly the failure `usable_dir`'s absolute check exists to prevent.

`main.rs::resolve_pi_sessions_dir(cfg)` then prefers
`usable_dir(Some(&cfg.analytics.pi_sessions_dir))`, falling back to
`paths::pi_sessions_dir()`, and yields `Option<PathBuf>`.

### 8. Feature gating and the default build

`just lint` runs `cargo clippy --all-targets -- -D warnings` on a
**default-feature** build, so a warning that only appears without
`analytics` still fails CI. Two naive approaches both break:

- Gate `Source::Pi` and `parse_pi_file` behind `#[cfg(feature = "analytics")]`
  → a default build never constructs `Source::Pi`, and rustc's dead-code lint
  flags a never-constructed variant in a binary crate (where `pub` grants no
  exemption).
- Gate nothing at all → a default build walks and JSON-parses the entire pi
  session tree on every refresh, only to feed the results into the no-op
  `Sink` (`analytics.rs:50`). Pure waste for users without the feature.

The approach this spec takes, following the no-op-`Sink` precedent already in
`analytics.rs`:

- `Source` (both variants), `parse_pi_file`, and `aggregate_windows`'s
  dispatch arm on `Source::Pi` are compiled in **every** build. The dispatch
  arm calls `parse_pi_file`, and `parse_pi_file` constructs `Source::Pi`, so
  neither is dead code in a default build and the hot loop needs no `#[cfg]`.
- Only `main.rs`'s **push of the pi root** onto the roots vec is
  `#[cfg(feature = "analytics")]`, and it additionally requires
  `cfg.analytics.enabled && cfg.analytics.pi_enabled` plus a resolvable
  directory. A default build therefore has no pi root in the vec, walks
  nothing extra, and pays nothing — even if a user sets `pi_enabled = true`
  in the config file.
- `Source::from_key` is the one exception: it is only ever called by
  `report`, so it carries `#[cfg(feature = "analytics")]`, matching how
  `ParsedRecord::for_test` is already gated (`cost.rs:206`).

## Known limitations

Each of these is a deliberate, documented gap rather than an oversight.

- **pi `{"type":"compaction"}` entries are not read.** pi is understood to
  charge for a summary LLM call during compaction and to count it in session
  totals, so omitting it undercounts long pi sessions. But a survey of the
  real pi sessions available (9 sessions, 456 entries; types present were
  only `session`, `model_change`, `thinking_level_change`, and `message`)
  turned up no compaction entry at all, so the entry's usage shape and the
  field path for its model are both unknown. **Before implementing compaction
  support, capture a real compaction entry from a real pi session and write
  the fixture from that.** A fixture built from a guessed shape would pass
  while real records were silently dropped.
- **Non-Anthropic pi usage is recorded unpriced (cost 0).** `pricing.json`
  holds Anthropic model rates only, and `cost_for` returns `0.0` for any
  model `resolve_model` cannot match (`cost.rs:121-124`). This is not limited
  to local/free providers: pi can drive any OpenAI-compatible endpoint, and a
  user running pi against a paid non-Anthropic model will see tokens with
  cost 0 in `report`. Tokens are stored, so re-pricing later is possible —
  that is why the row holds raw counts. The fix direction is adding those
  rates to `pricing.json` (out of scope here), not changing the cost source.
  - A fallback to pi's own `cost.total` when `resolve_model` fails was
    considered and **rejected for this spec**. It would need a new
    `ParsedRecord` field (pi's cost is per-entry data that `cost_for`, which
    sees only model + usage, cannot reach) and an extra branch in the shared
    hot loop, for a case with no observed instance yet. It is also less
    urgent than it looks: back-solving pi's embedded `cost` object from real
    usage shows pi's per-token rates currently **match `pricing.json`
    exactly** for the models both apps share (`claude-opus-4-8` at
    5/25/6.25/0.5 per Mtok, `claude-sonnet-5` at 2/10/2.5/0.2), so the two
    sources disagree on nothing today. And cost 0 for an unpriced model is
    the codebase's established convention, visible as tokens-without-cost
    rather than a silently plausible wrong number. Revisit if `pricing.json`
    gains non-Anthropic rates or a user reports paid non-Anthropic pi usage.
- **`cacheWrite1h` is priced at the 5-minute cache-write rate.**
  `cacheWrite1h` appears in most real pi usage objects (157 of 198 sampled,
  all currently zero-valued — it carries real tokens only when a user enables
  the 1-hour cache TTL). It is folded into `cache_creation_tokens` (§2)
  because `pricing.json` has a single `cache_write` rate per model, so a 1-hour
  write is priced as a 5-minute one. That underestimates those tokens, but by
  far less than dropping them entirely would, and the token count itself is
  stored accurately. A per-TTL cache-write rate is a `pricing.json` change,
  out of scope here.

## Testing

New unit tests:

- `parse_pi_file`: header `cwd`/session-id carry-forward, including a header
  that is **not** line 1; usage-key normalization
  (`input`/`cacheRead`/… → `*_tokens`); `cacheWrite` + `cacheWrite1h` summed
  into `cache_creation_tokens`; non-assistant and all-zero usage lines
  skipped; missing header, and header without `cwd` → empty; malformed line →
  skip and keep reading.
- `parse_pi_file` resilience, mirroring the Claude-side regression tests now
  that both share one line loop: a non-UTF-8 line mid-file does not abandon
  the rest of the file, and a timestamp whose byte 10 is not a char boundary
  does not panic.
- pi dedup: two files reusing the same 8-hex entry `id` under **different**
  session ids do not collide; the same entry appearing in two files under the
  **same** session id *is* deduped (the property file-scoping would lose); a
  genuine intra-file duplicate is still deduped.
- `encode_key`/`decode_key`: 4-part round trip; a **3-part legacy key decodes
  with `app == "claude-code"`** (not `None`); a 5-part key → `None`.
- `Sink::flush` legacy cleanup: writing a `claude-code` row removes the
  3-part twin key for the same `(date, repo_key, model)`; writing a `pi` row
  leaves any 3-part key untouched; a store holding only pre-existing 3-part
  rows for older dates is unchanged by a flush and still readable by
  `report`.
- `aggregate_windows`: two roots (one Claude, one pi) attributing to distinct
  `app` keys for the same repo/day/model; cross-app dedup does not cancel.
- `aggregate_windows` window guard: pi records reach `analytics.record` but are
  **excluded** from the returned `WindowTotals` (the chip is Claude-only) —
  assert a pi-only day yields a stored row but a zero daily total.
- `report`: `app` present as the last JSON field and last CSV column;
  `--app pi` filters; `--app claude` (an invalid value) is an **error**, not
  an empty result; `--all` keeps per-`(repo, app)` rows distinct.
- Re-pricing: a pi record with a known Anthropic model prices from
  `pricing.json`, not from its embedded `cost.total`. The fixture must carry
  a **deliberately wrong** `cost.total` (e.g. 10x) for the assertion to mean
  anything — real pi and ferrisbar rates agree for Anthropic models (see
  Known limitations), so a realistic fixture would pass either way.
- Unpriced model: a pi record on a model absent from `pricing.json` (e.g.
  `provider: "ollama"`, `model: "gemma4:12b-mlx"`, `cost.total` 0) stores
  tokens with cost 0 and does not panic — mirrors the existing
  `aggregate_windows_still_records_an_unpriced_models_usage_for_analytics`
  test (`cost.rs:1162`).

Existing tests that must be updated (not new coverage — churn the change
forces):

- `cost.rs:895` asserts `rec.dedup_key == Some("msg_1:req_1")`; the
  app-prefix from §4 changes that expected string.
- `ParsedRecord::for_test` call sites gain the `app` argument
  (`store.rs:173`).
- `refresh_daily_cache` and `aggregate_windows` call sites take a roots slice
  (see §3 for the line list).
- `store.rs` tests calling `encode_key` (`:188`, `:250`, `:281`, `:314`) gain
  the `app` argument. `report.rs:305`'s hand-built 3-part key can stay as-is —
  it now exercises the legacy-decode path, which is a bonus, not a break.
  That reduced churn is one more argument for the shim over a table bump.

End-to-end (`tests/cli.rs`), which the prior design mandated for `report` and
which already exists there for the Claude path (`tests/cli.rs:383-519`):

- The existing `report` e2e tests keep asserting what they assert today
  against a store written by the new code — proving the `app` addition didn't
  break the Claude path — with one of them extended to also assert the new
  `app` field reads `claude-code`.
- A pi-ingestion e2e test: a pi sessions tree + a config with
  `pi_enabled = true`, driven through `--internal-refresh-daily-cost`, then
  read back with `report --app pi`.
- **Sequencing note:** that pi e2e test needs a way to point the binary at a
  fixture pi sessions directory. Claude's path has `CLAUDE_CONFIG_DIR` as a
  test hook (`tests/cli.rs:437`); pi has no analogous environment variable
  today, so the hook is `[analytics] pi_sessions_dir` in the config file. pi
  e2e coverage therefore cannot land before the config work (§7) — see
  Sequencing.

New fixtures: a pi-shaped `.jsonl` (session header + assistant messages,
including one with a non-zero `cacheWrite1h`), one with a deliberately wrong
`cost.total`, and one on an unpriced local model — all escaped for Windows
paths via `cost.rs`'s existing `escape_path_for_json`.

## Migration / compatibility

- **No table rename, so no history is lost.** The existing `cost_rows` table
  is read and written in place. Pre-existing 3-part keys keep decoding, now
  reported as `app = "claude-code"`, which is what they always were.
- On the first upgraded run, today's and yesterday's Claude rows are rewritten
  under 4-part keys and their 3-part twins removed (§5), so no key is
  double-counted while both shapes coexist. Older dates keep their 3-part
  shape indefinitely and read fine.
- **Downgrade is lossy, and knowingly so.** An older binary's `decode_key`
  rejects 4-part keys, so a downgrade hides every row written after the
  upgrade until the user upgrades again. Nothing is deleted, and this spec
  accepts that asymmetry rather than adding a dual-write.
- Default build unaffected: pi ingestion requires `--features analytics` **and**
  `[analytics] enabled = true` **and** `[analytics] pi_enabled = true`, and
  §8 keeps the pi walk out of a default build entirely.
- No change to the statusline output or the `ferrisbar-pi` extension.

## Sequencing

1. **`app` dimension, Claude-only.** `Source` on `ParsedRecord`, the
   `Source::ClaudeCode` prefix on `parse_line`'s `dedup_key` (§4), 4-part
   `encode_key`, the 3-or-4-part `decode_key` shim, `flush`'s legacy-key
   removal, `report`'s `app` column and validated `--app` filter. Every row
   is still tagged `claude-code`. Safe to ship alone: no table rename, so no
   history is stranded, and the only user-visible change is a new column
   reading `claude-code`.
2. **pi reader.** Extract the shared resilient line loop (§2), then
   `parse_pi_file` plus the session-scoped dedup key, unit-tested in
   isolation. No wiring yet.
3. **Multi-root `aggregate_windows`** (with the Claude-only `WindowTotals`
   guard) + `main.rs` roots-vec wiring, including §8's gate. Signature changes
   and their test updates land here.
4. **Config** (`pi_enabled`, `pi_sessions_dir`, the `Copy` derive drop,
   `pub usable_dir`, `paths::pi_sessions_dir`) + sample `config.toml` +
   README. This is also what unblocks pi e2e coverage, so the pi
   `tests/cli.rs` test lands with this step, not with step 2 or 3.
5. **(Deferred, needs data)** pi compaction entries — only once a real
   compaction entry has been captured from a real session (see Known
   limitations).
6. **(Fast-follow, separate spec)** per-app statusline chip.
