# ferrisbar — multi-app cost analytics (Claude Code + pi)

## Purpose

Let a user attribute cost/usage not just by day, repo, and model, but also
by the **app that produced it** — today Claude Code, and now
pi ([`@earendil-works/pi-coding-agent`](https://www.npmjs.com/package/@earendil-works/pi-coding-agent);
session format: <https://pi.dev/docs/latest/session-format>). A user driving both
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

```text
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
| usage keys | `input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens` | `input`, `output`, `cacheWrite`, `cacheRead`, plus the optional `cacheWrite1h` (a **subset** of `cacheWrite`, not an addition) and `reasoning` (a subset of `output`), `totalTokens`, and a pre-computed nested `cost` |
| usage-bearing lines | every line | top-level `{"type":"message"}` (assistant). pi's session-format doc also gives `{"type":"compaction"}` and `{"type":"branch_summary"}` entries an optional `usage` for the summary LLM call, with no model recorded — see Known limitations |
| `cwd` | present on **every** record | present **only** on the `{"type":"session"}` header line |
| dedup id | `message.id` + `requestId` (globally unique) | per-entry `id` — normally **8 hex chars** (~32 bits), falling back to a full UUID if pi hits 100 collisions in a row (`generateId`). Unique within a file; **preserved verbatim** when pi copies entries into a new file (see below). Absent on old v1 files |
| session id | n/a | the `{"type":"session"}` header's `id` is a full UUID, also embedded in the filename. A fork/clone/branch gets a **new** one (see below), so it does not identify an entry across files |
| provider | implicit (Anthropic) | explicit `provider` inside `message` (`anthropic`, `ollama`, and any OpenAI-compatible endpoint via `api: "openai-completions"`) |
| model id | `claude-sonnet-5` | `claude-opus-4-8` for Anthropic — same scheme, `pricing.json` prefix match still resolves. Non-Anthropic ids (`gemma4:12b-mlx`, `gpt-…`) resolve to nothing |
| timestamp | top-level ISO8601 (`YYYY-MM-DDTHH:MM:SS…Z`) | top-level ISO8601 — **same**, `parse_iso8601_utc` works unchanged |

On cross-file uniqueness of the 8-hex entry `id`: a survey of the real pi
sessions on the author's machine (9 sessions, 456 entries, 447 of them
carrying an 8-hex `id`) found **no** duplicate id across files. That is
evidence of nothing — 447 draws from a 32-bit space would rarely collide by
chance regardless. Treat cross-file uniqueness of a bare `id` as unproven.

What *is* established, from pi's own source (`pi-coding-agent@0.81.1`,
`dist/core/session-manager.js`), is that pi deliberately copies entries
between files and keeps their `id` and `timestamp` while changing the
session id:

- `SessionManager.forkFrom` (`/fork`, `/clone`, `newSession({ parentSession })`)
  writes a new header with a fresh `id`, `parentSession` pointing at the
  source file, and possibly a **different `cwd`**, then appends every source
  entry verbatim (`JSON.stringify(entry)`).
- `createBranchedSession` copies one branch path as `{ ...entry, parentId }`
  under a new session id — `id` and `timestamp` survive, `parentId` may not.

So the same billed assistant message routinely appears in two files under
two session ids. §4's dedup key is built from the fields those copies
preserve.

Representative pi lines (real shape, `cwd` and content elided):

```jsonl
{"type":"session","version":3,"id":"019ff77b-d3c3-7dcd-a1d0-1f90cebf956a","timestamp":"2026-08-12T19:38:35.331Z","cwd":"/Users/dev/repo"}
{"type":"message","id":"9d0e1ca6","parentId":"8b417fdc","timestamp":"2026-08-12T19:40:03.469Z","message":{"role":"assistant","content":[…],"api":"openai-completions","provider":"anthropic","model":"claude-opus-4-8","usage":{"input":1373,"output":76,"cacheRead":0,"cacheWrite":0,"cacheWrite1h":0,"reasoning":0,"totalTokens":1449,"cost":{"input":0.006865,"output":0.0019,"cacheRead":0,"cacheWrite":0,"total":0.008765}}}}
```

Note `parentId`: pi entries form a branchable tree, and as shown above, pi
copies entries into new files on fork, clone and branch — the same situation
Claude Code's own dedup exists to handle
(`cost.rs:1063`: "Same message resumed into a second transcript — must not
double-count").

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| New dimension | Add `app` as a first-class key component (`Source { ClaudeCode, Pi }`), not a repo-key prefix hack — keeps repo identity clean and lets `report` group/filter by app |
| Store key shape | Extend from `date\0repo_key\0model` to `date\0repo_key\0model\0app` |
| Legacy rows | **Keep the existing `cost_rows` table** and make `decode_key` accept both 3-part (legacy, app defaults to `claude-code`) and 4-part keys. Bumping the table name was considered and rejected: the store has no TTL and no pruning, so a new table would not "repopulate" — it would silently strand every row older than yesterday, permanently, on the first upgraded run. For a feature whose purpose is judging repo ROI *over time*, that is the worst possible upgrade outcome. Bloat is not a counter-argument: rows are ~150 bytes, so a year of history is well under a megabyte |
| Cost source | Price from `pricing.json` whenever `resolve_model` matches, ignoring pi's own `cost.total`. Cross-app comparison of the same model must be apples-to-apples, and raw tokens are stored either way so re-pricing stays possible. When `resolve_model` does **not** match, the record is stored **unpriced** (tokens kept, cost 0) via the existing unpriced path — the same treatment Claude gives an unknown model. This applies to *any* model absent from `pricing.json`, not just local/free ones; see Known limitations, which also records the rejected `cost.total` fallback |
| pi model ids | Rely on existing `resolve_model` prefix/alias matching. No `pricing.json` change in this spec. Provider is **not** part of the key (pricing is prefix-matched on model id); a provider dimension is a possible future addition, not modeled here |
| Compaction / branch-summary cost | **Out of scope for this spec.** pi's session-format doc gives `{"type":"compaction"}` and `{"type":"branch_summary"}` entries an optional `usage` (the same `Usage` type) for the summary LLM call, but records **no model** on either entry, so there is nothing to price it against without guessing which model ran the summary. No real example has been captured either. Accepted as a documented undercount |
| Reader design | Introduce a `Source`-tagged reader step. Claude Code keeps its stateless per-line `parse_line`; pi gets a **stateful per-file** parser (carry `cwd` and session id from the header, normalize usage key names) built on the *same* resilient line-reading loop `aggregate_windows` already uses (§2) |
| Dedup across apps | Keep per-app dedup. pi's key is `pi\0<entry-id>\0<entry-timestamp>` — the two fields pi preserves when it forks, clones or branches a session into a new file. Neither the session id nor the file stem works: pi assigns a fresh session id to every copy, so either would count each copied message twice (§4). Which copy *attributes* the cost is made deterministic by reading pi files in filename order (§3) |
| Enablement | pi ingestion is opt-in on top of the existing `analytics` feature + `[analytics] enabled`. New `[analytics] pi_enabled = false` default, plus `pi_sessions_dir` override. Directory resolution honors pi's own `PI_CODING_AGENT_SESSION_DIR` / `PI_CODING_AGENT_DIR` env vars first (§7) |
| Feature gating | `Source`, `parse_pi_file`, and `aggregate_windows`'s dispatch on `Source` compile in **every** build (no `#[cfg]` in the hot loop), mirroring the existing no-op-`Sink` precedent. The pi-root resolution and push in `main.rs` are `#[cfg(feature = "analytics")]`, so a default build never walks the pi tree (§8) |
| Roots | `aggregate_windows` walks a `Vec<(PathBuf, Source)>` instead of one root, so adding a third app later is additive. `main.rs` owns building that vec (§3) |
| Backfill | None — forward-only from enablement, matching the existing analytics design |

## Scope

In scope:
- `app` dimension end-to-end: `ParsedRecord`, store key, `Row` grouping, `report`.
- A pi transcript reader (stateful header-`cwd` carry-forward, usage-key
  normalization, assistant-message usage).
- Multi-root ingestion in `aggregate_windows` / `refresh_daily_cache`, with the
  visible `WindowTotals` kept Claude-only (see §3).
- Backward-compatible key decoding plus legacy-key cleanup on write (§5).
- Config: pi opt-in + sessions-dir override; pi sessions root resolution.
- `ferrisbar report`: `app` column (JSON/CSV), validated `--app` filter,
  per-`(repo, app)` summary rows.

Out of scope:
- The visible statusline cost chips (`daily_chip`, `budget_line`) stay
  Claude-Code-only for now: even when `pi_enabled`, pi records go to the
  persistent store but **not** into the `WindowTotals` that feed the chips,
  and **not** into the 0.5.0 `repo $X` segment, so enabling pi ingestion does
  not change the rendered statusline. Both need explicit guards — an `app`
  check on the window accumulation (§3) and in `today_repo_cost` (§5) — not a
  no-op. A per-app / cross-app chip is a fast-follow.
- pi `{"type":"compaction"}` and `{"type":"branch_summary"}` usage (see Known
  limitations).
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
and a `\0` separator, so `msg_1:req_1` becomes `claude-code\0msg_1:req_1`
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
fn parse_pi_file(path: &Path, mut emit: impl FnMut(ParsedRecord)) {
    // 1. Session header: the **first line whose `type` is `"session"`** —
    //    not "line 1" — read for `cwd`. No such line, or no `cwd` on it:
    //    skip the whole file (nothing to attribute).
    // 2. For each **top-level** `{"type":"message"}` entry with
    //    message.role == "assistant" and a usage object, map:
    //      input       → input_tokens
    //      output      → output_tokens
    //      cacheWrite  → cache_creation_tokens
    //      cacheRead   → cache_read_tokens
    //      (cacheWrite1h is a subset of cacheWrite and reasoning a subset of
    //       output, so adding either would double-count; totalTokens is
    //       derived. All three are ignored.)
    //    Every usage field is `#[serde(default)]`: `cacheWrite1h` and
    //    `reasoning` are optional in pi's `Usage` type, and older files may
    //    omit others.
    //    model  ← message.model
    //    date   ← top-level timestamp, sliced with `.get(..10)?`
    //    dedup  ← `pi\0<id>\0<timestamp>` (§4); `None` when the entry has no
    //             `id` (old v1 files), same as a Claude line without ids
    //    cwd    ← header cwd
    //    app    ← Source::Pi
    // 3. Skip all-zero usage (reuse Usage::is_empty).
    // 4. Hand each record to `emit` as it is parsed.
}
```

Only top-level `message` lines are read. A `compaction` entry's
`retainedTail` embeds copies of earlier assistant messages, `usage` included;
descending into it would count those messages a second time.

`.get(..10)`, never `timestamp[..10]`: byte offset 10 is not guaranteed to
land on a UTF-8 char boundary, and slicing off a boundary panics.
`parse_line` already uses `.get(..10)` for exactly this reason
(`cost.rs:246-251`) and has a named regression test for it
(`parse_line_does_not_panic_on_a_non_char_boundary_timestamp`,
`cost.rs:954`). The pi reader must not reintroduce the panicking form.

Same never-panic-on-input discipline as `parse_line`: unreadable file,
missing/`cwd`-less header, malformed lines → skip, don't raise.

The callback keeps the pi path streaming one record at a time, like the
Claude path (`cost.rs:475-536`), instead of buffering a whole session file.
The only per-file state is the header's `cwd`.

### 3. Multi-root aggregation (`src/cost.rs`, `src/main.rs`)

- Change `aggregate_windows(transcripts_root, …)` to
  `aggregate_windows(roots: &[(PathBuf, Source)], …)`.
- For a `ClaudeCode` root: `discover_transcripts` + per-line `parse_line`.
  For a `Pi` root: `discover_transcripts`, then two pi-only steps, then
  `parse_pi_file`:
  - **Sort by file name.** pi names files `<creation-timestamp>_<uuid>.jsonl`,
    so name order is creation order and an original is read before its forks.
    That fixes which copy of a forked entry is seen first, and therefore which
    `cwd` (repo) it is attributed to: `forkFrom` can give the copy a different
    `cwd`. Without this, `read_dir` order could flip attribution between
    refreshes, and since `flush` overwrites but never deletes, the repo that
    lost the entry would keep a stale row and the cost would be counted twice.
  - **Skip files whose mtime is before the start of yesterday (UTC).** Only
    records dated today or yesterday reach the store (`store.rs:93`), and pi
    records never reach the chip, so such a file contributes nothing. Each
    refresh then reads only recently active sessions instead of pi's whole
    history, which matters against the refresh lock's `LOCK_STALE_AFTER = 75s`
    (`cost_cache.rs:38`). It is safe: an entry dated today or yesterday was
    written today or yesterday, so its file's mtime is at least that recent.
    A file whose mtime can't be read is not skipped.
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
  (`cost.rs:1592`, `:1619`, `:1645`) plus the five `aggregate_windows` tests
  (`cost.rs:1082`, `:1117`, `:1143`, `:1179`, `:1215`).

### 4. Dedup across apps (`src/cost.rs`)

The `seen` set in `aggregate_windows` currently holds raw dedup keys. Claude's
key (`message.id:requestId`) is globally unique. pi's entry `id` is 8 hex
chars, so a bare app prefix is not enough.

There are two failure modes, and they pull in opposite directions:

- **Too coarse** (two distinct entries share a key) → one is dropped →
  *undercount*.
- **Too fine** (one logical entry appears under two keys) → both are counted
  → *double-count real money*. This is the worse of the two, and it is the
  one any session- or file-scoped key causes. pi's `forkFrom` and
  `createBranchedSession` (see "The format mismatch") copy entries into a new
  file under a **new session id**, keeping each entry's `id` and
  `timestamp`. A key containing the session id or the file stem therefore
  gives the original and the copy different keys, and every forked, cloned or
  branched message is counted twice.

Key on what the copies preserve: the entry `id` plus the entry's top-level
`timestamp`.

```rust
// pi: app + entry id + entry timestamp (both preserved verbatim by fork/clone/branch)
rec.dedup_key = Some(format!("{}\0{}\0{}", Source::Pi.key(), entry_id, timestamp));
```

That is not too coarse either. For two *distinct* entries to collide they
would need the same 32-bit random `id` **and** the same millisecond
timestamp, which is negligible. The timestamp is used verbatim as pi wrote
it, not re-parsed, so the key never depends on date-parsing behavior.

Claude's `dedup_key` is likewise prefixed with `Source::ClaudeCode.key()` in
`parse_line` (`claude-code\0msg:req`, §1), so a Claude key and a pi key can
never collide or cancel.
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
  - Dates older than yesterday normally have only the 3-part shape, so they
    cannot double-count. The exception is a downgrade followed by an upgrade
    (see the next bullet).
- **Readers also merge on the read side.** In `report.rs::read_all`
  (`report.rs:97`) and `today_repo_cost`, when a 3-part key and a 4-part `claude-code` key both
  exist for the same `(date, repo_key, model)`, the 4-part row wins and the
  3-part row is ignored. Write-side removal alone is not enough: a downgraded
  binary writes fresh 3-part keys for today and yesterday next to the 4-part
  ones, and if those dates leave the two-day window before the user upgrades
  again, `flush` never revisits them, so both shapes would be counted forever.
  The write-side removal stays as cleanup.
- **`today_repo_cost` (`store.rs:165-213`, the 0.5.0 `repo $X` segment) skips
  rows whose app is not `claude-code`**, keeping the segment Claude-only per
  Scope and consistent with `2026-08-12-repo-cost-today-segment-design.md`,
  which defines it as today's Claude Code spend. Without this, pi spend would
  appear in the repo segment while the `$X today` total beside it stays
  Claude-only, so the repo figure could exceed the day total. Its 3-tuple
  destructure of `decode_key` (`store.rs:198`) becomes a 4-tuple, and its
  comment about the `date\0repo_key\0model` key shape needs updating (the
  `"{today}\0" .. "{today}\u{1}"` range bound still holds for 4-part keys).
- `Row` is unchanged (app lives in the key, not the value).

### 6. Report (`src/analytics/report.rs`)

- `decode_key` now yields `(date, repo_key, model, app)`; `ReportRow` gains
  `app`. Legacy rows arrive as `claude-code`, so no stored history disappears
  from `report`.
- Raw-row output (`JsonRow`, CSV header at `report.rs:184`) and `--all`
  output (`SummaryRow`, `report.rs:248-256`, with its own CSV header at
  `report.rs:273`) all gain an `app` field, **appended as the last
  column/field**. Appending rather than
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
  `source.key()`). Its sort (`report.rs:146`, `date` then `model`) gains
  `.then_with(|| a.app.cmp(&b.app))` so a Claude row and a pi row for the
  same date and model come out in a fixed order.
- `--all` **groups by the `(repo_key, app)` tuple** — one summary row per
  repo per app, which is the cross-app comparison this whole spec exists to
  enable. (A separate per-app rollup is not added; `--all` output already
  carries `app`, so a consumer can roll up itself.) A `(String, String)`
  `HashMap` key preserves the existing deterministic cost-tie tie-break for
  free, since tuple `Ord` compares `repo_key` then `app`
  (`report.rs:238-243`).
- `--app` combined with `--all` filters rows by app **before** grouping, so
  `report --all --app pi` gives one row per repo with pi usage.
- **Compatibility change:** grouping by `(repo, app)` means a repo used from
  both apps gets two `--all` rows. README.md:359 promises "one summary row per
  tracked repo"; update it to "one summary row per tracked repo per app". In
  practice nothing changes until pi ingestion is enabled, because every row
  reads `claude-code` until then.
- The usage string at `main.rs:134` lists the report flags by hand; add
  `[--app claude-code|pi]`.

### 7. Config (`src/config.rs`, `src/paths.rs`, sample `config.toml`)

New keys under `[analytics]`:

```toml
[analytics]
enabled = true          # existing
pi_enabled = false      # opt in to pi ingestion
pi_sessions_dir = ""    # optional absolute override; see resolution order below
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

**Resolution order.** pi has its own overrides (`pi-coding-agent@0.81.1`,
`dist/config.js` and `dist/main.js`): `PI_CODING_AGENT_SESSION_DIR` sets the
session directory outright, and `PI_CODING_AGENT_DIR` moves the whole agent
directory, with sessions under `<agent-dir>/sessions`. A user who has set
either would get nothing ingested if ferrisbar only looked in
`~/.pi/agent/sessions`. `main.rs::resolve_pi_sessions_dir(cfg)` returns the
first usable (non-empty, absolute) candidate as an `Option<PathBuf>`:

1. `$PI_CODING_AGENT_SESSION_DIR`
2. `$PI_CODING_AGENT_DIR/sessions`
3. `[analytics] pi_sessions_dir`
4. `paths::pi_sessions_dir()` (`$HOME`, falling back to `$USERPROFILE`, then
   `.pi/agent/sessions`)

Environment beats file, matching `config_dir.rs:8-21` for
`CLAUDE_CONFIG_DIR`: these are the same variables pi itself obeys, so they
describe where pi is actually writing. The file key covers what ferrisbar
can't see, namely pi's `--session-dir` flag and a `sessionDir` in pi's
settings. `discover_transcripts` already walks recursively, so it works both
on the default per-cwd layout and on a flat custom session directory.

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
  `Sink` (`analytics.rs:39-62`). Pure waste for users without the feature.

The approach this spec takes, following the no-op-`Sink` precedent already in
`analytics.rs`:

- `Source` (both variants), `parse_pi_file`, and `aggregate_windows`'s
  dispatch arm on `Source::Pi` are compiled in **every** build. The dispatch
  arm calls `parse_pi_file`, and `parse_pi_file` constructs `Source::Pi`, so
  neither is dead code in a default build and the hot loop needs no `#[cfg]`.
- `main.rs`'s **push of the pi root** onto the roots vec is
  `#[cfg(feature = "analytics")]`, and it additionally requires
  `cfg.analytics.enabled && cfg.analytics.pi_enabled` plus a resolvable
  directory. A default build therefore has no pi root in the vec, walks
  nothing extra, and pays nothing — even if a user sets `pi_enabled = true`
  in the config file.
- **The resolvers are gated too.** `main.rs::resolve_pi_sessions_dir` and
  `paths::pi_sessions_dir` are only called from that gated push, so in a
  default build they would be dead code and fail `just lint`. A prototype of
  the push-only gate confirmed it: `cargo clippy --all-targets -- -D warnings`
  reported `function 'resolve_pi_sessions_dir' is never used`. Put
  `#[cfg(feature = "analytics")]` on both functions, or on one helper that
  holds both the resolution and the push. The new `AnalyticsConfig` fields do
  not warn, because `parse_analytics` reads them in every build.
- `Source::from_key` is only ever called by `report`, so it is gated as
  well. Use `#[cfg(feature = "analytics")]`, not the
  `#[cfg(any(test, feature = "analytics"))]` that `ParsedRecord::for_test`
  carries (`cost.rs:206`). The difference means `from_key`'s unit tests
  belong in `report.rs`'s test module, which only compiles with the feature,
  rather than in `cost.rs`'s.

## Known limitations

Each of these is a deliberate, documented gap rather than an oversight.

- **pi `compaction` and `branch_summary` usage is not read.** pi's
  session-format doc gives both entries an optional `usage` ("LLM usage from
  generating the summary; included in session token and cost totals"), with
  the same `Usage` shape as assistant messages. So omitting them undercounts
  long pi sessions (compaction) and sessions that switch branches with `/tree`
  (branch summaries). The blocker is the model, not the shape: neither entry
  records which model produced the summary, so it can't be priced without
  guessing. A survey of the real pi sessions available (9 sessions, 456
  entries; types present were only `session`, `model_change`,
  `thinking_level_change`, and `message`) turned up neither entry. **Before
  implementing support, capture real examples from real pi sessions, decide
  how to attribute the model (e.g. the most recent `model_change` or
  assistant model), and write the fixtures from those.**
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
- **1-hour cache writes are priced at the single `cache_write` rate.**
  `cacheWrite1h` appears in most real pi usage objects (157 of 198 sampled,
  all currently zero-valued — it carries real tokens only when a user enables
  the 1-hour cache TTL). pi-ai defines it as the "subset of `cacheWrite`
  written with 1h retention" (`dist/types.d.ts`; `dist/api/anthropic-messages.js`
  sets `cacheWrite` from `cache_creation_input_tokens` and `cacheWrite1h` from
  `cache_creation.ephemeral_1h_input_tokens`), so those tokens are already in
  `cacheWrite` and are **not** added again (§2). `pricing.json` has one
  `cache_write` rate per model, so the 1-hour portion is priced at that rate,
  which underestimates it. This is the same treatment Claude Code's own
  `cache_creation_input_tokens` gets today. A per-TTL cache-write rate is a
  `pricing.json` change, out of scope here.

## Testing

New unit tests:

- `parse_pi_file`: header `cwd` carry-forward, including a header that is
  **not** line 1; usage-key normalization
  (`input`/`cacheRead`/… → `*_tokens`); a non-zero `cacheWrite1h` is **not**
  added to `cache_creation_tokens` (which equals `cacheWrite` alone); usage
  objects missing `cacheWrite1h`/`reasoning` still parse; non-assistant and
  all-zero usage lines skipped; missing header, and header without `cwd` →
  nothing emitted; malformed line → skip and keep reading; an entry without an
  `id` gets `dedup_key == None`; a `compaction` entry whose `retainedTail`
  holds an assistant message with usage emits nothing.
- `parse_pi_file` resilience, mirroring the Claude-side regression tests now
  that both share one line loop: a non-UTF-8 line mid-file does not abandon
  the rest of the file, and a timestamp whose byte 10 is not a char boundary
  does not panic.
- pi dedup, using a fixture shaped like pi's real `forkFrom` output: a second
  file with a new header `id`, `parentSession`, and the source entries copied
  verbatim counts each copied entry **once**. Two distinct entries that share
  an 8-hex `id` but have different timestamps both count. A genuine intra-file
  duplicate is still deduped.
- pi attribution: when the fork's header has a **different `cwd`** and a file
  name that sorts after the original, the copied entries are attributed to the
  original's repo, and the result does not depend on the order files come back
  from discovery.
- pi mtime prefilter: a pi file whose mtime is before the start of yesterday
  is not read; a recent file is.
- `encode_key`/`decode_key`: 4-part round trip; a **3-part legacy key decodes
  with `app == "claude-code"`** (not `None`); a 5-part key → `None`.
- `Sink::flush` legacy cleanup: writing a `claude-code` row removes the
  3-part twin key for the same `(date, repo_key, model)`; writing a `pi` row
  leaves any 3-part key untouched; a store holding only pre-existing 3-part
  rows for older dates is unchanged by a flush and still readable by
  `report`.
- Read-side merge: a store holding both a 3-part and a 4-part `claude-code`
  key for the same `(date, repo_key, model)` on a date **older than
  yesterday** (the downgrade-then-upgrade case) is counted once, using the
  4-part row, by both `read_all` and `today_repo_cost`.
- `today_repo_cost`: a `pi` row for today's repo does not change the result;
  4-part `claude-code` rows and legacy 3-part rows both still count.
- `aggregate_windows`: two roots (one Claude, one pi) attributing to distinct
  `app` keys for the same repo/day/model; cross-app dedup does not cancel.
- `aggregate_windows` window guard: pi records reach `analytics.record` but are
  **excluded** from the returned `WindowTotals` (the chip is Claude-only) —
  assert a pi-only day yields a stored row but a zero daily total.
- `report`: `app` present as the last JSON field and last CSV column, in both
  raw-row and `--all` output; `--app pi` filters; `--app claude` (an invalid
  value) is an **error**, not an empty result; `--all` keeps per-`(repo, app)`
  rows distinct; `--all --app pi` filters before grouping; raw rows with the
  same date and model sort by `app`.
- Re-pricing: a pi record with a known Anthropic model prices from
  `pricing.json`, not from its embedded `cost.total`. The fixture must carry
  a **deliberately wrong** `cost.total` (e.g. 10x) for the assertion to mean
  anything — real pi and ferrisbar rates agree for Anthropic models (see
  Known limitations), so a realistic fixture would pass either way.
- Unpriced model: a pi record on a model absent from `pricing.json` (e.g.
  `provider: "ollama"`, `model: "gemma4:12b-mlx"`, `cost.total` 0) stores
  tokens with cost 0 and does not panic — mirrors the existing
  `aggregate_windows_still_records_an_unpriced_models_usage_for_analytics`
  test (`cost.rs:1197`).
- `resolve_pi_sessions_dir`: each step of the §7 resolution order wins over
  the ones below it; an empty or relative value at any step falls through.

Existing tests that must be updated (not new coverage — churn the change
forces):

- `cost.rs:930` asserts `rec.dedup_key == Some("msg_1:req_1")`; the
  app prefix from §1/§4 changes that expected string to
  `"claude-code\0msg_1:req_1"`.
- `ParsedRecord::for_test` call sites gain the `app` argument
  (`store.rs:238`, `cost.rs:1388`, `cost.rs:1435`).
- `refresh_daily_cache` and `aggregate_windows` call sites take a roots slice
  (see §3 for the line list).
- `store.rs` tests calling `encode_key` (`:253`, `:315`, `:346`, `:379`,
  `:544`) gain the `app` argument. `report.rs:305`'s hand-built 3-part key can stay as-is —
  it now exercises the legacy-decode path, which is a bonus, not a break.
  That reduced churn is one more argument for the shim over a table bump.

End-to-end (`tests/cli.rs`), which the prior design mandated for `report` and
which already exists there for the Claude path (`tests/cli.rs:382-578`):

- The existing `report` e2e tests keep asserting what they assert today
  against a store written by the new code — proving the `app` addition didn't
  break the Claude path — with one of them extended to also assert the new
  `app` field reads `claude-code`.
- A pi-ingestion e2e test: a pi sessions tree + a config with
  `pi_enabled = true`, driven through `--internal-refresh-daily-cost`, then
  read back with `report --app pi`.
- A repo-segment e2e test next to
  `daily_chip_shows_a_repo_segment_for_the_current_repo` (`tests/cli.rs:505`):
  with pi ingestion enabled and pi usage for the current repo today, the
  rendered `repo $X` segment still shows only the Claude amount.
- **Test hook:** the pi e2e tests point the binary at a fixture pi sessions
  directory with `PI_CODING_AGENT_SESSION_DIR`, the same way Claude's path
  uses `CLAUDE_CONFIG_DIR` (`tests/cli.rs:437`). The env var is honored by §7's
  resolution order, so pi e2e coverage lands with the config work (§7) — see
  Sequencing.

New fixtures: a pi-shaped `.jsonl` (session header + assistant messages,
including one with a non-zero `cacheWrite1h` that is a subset of its
`cacheWrite`), a `forkFrom`-shaped copy of it with a new header and a different
`cwd`, one with a deliberately wrong `cost.total`, and one on an unpriced local
model — all escaped for Windows paths via `cost.rs`'s existing
`escape_path_for_json`.

## Migration / compatibility

- **No table rename, so no history is lost.** The existing `cost_rows` table
  is read and written in place. Pre-existing 3-part keys keep decoding, now
  reported as `app = "claude-code"`, which is what they always were.
- On the first upgraded run, today's and yesterday's Claude rows are rewritten
  under 4-part keys and their 3-part twins removed (§5), so no key is
  double-counted while both shapes coexist. Older dates keep their 3-part
  shape indefinitely and read fine.
- **Downgrade is lossy, and knowingly so.** An older binary's `decode_key`
  rejects 4-part keys. `report` skips them, so it hides every row written
  after the upgrade until the user upgrades again. The 0.5.0
  `today_repo_cost` is stricter: it aborts on the first undecodable key
  (`decode_key(...)?`, `store.rs:198`), so the `repo $X` segment disappears
  entirely until the day's rows are 3-part again. Nothing is deleted, and this
  spec accepts that asymmetry rather than adding a dual-write. The 3-part rows
  a downgraded binary writes in the meantime are reconciled by §5's read-side
  merge after the re-upgrade, so they are not double-counted.
- Default build unaffected: pi ingestion requires `--features analytics` **and**
  `[analytics] enabled = true` **and** `[analytics] pi_enabled = true`, and
  §8 keeps the pi walk out of a default build entirely.
- No change to the statusline output or the `ferrisbar-pi` extension.

## Sequencing

1. **`app` dimension, Claude-only.** `Source` on `ParsedRecord`, the
   `Source::ClaudeCode` prefix on `parse_line`'s `dedup_key` (§4), 4-part
   `encode_key`, the 3-or-4-part `decode_key` shim, `flush`'s legacy-key
   removal, the read-side merge and `today_repo_cost`'s app filter (§5),
   `report`'s `app` column, sort tie-break and validated `--app` filter, and
   the usage string. Every row is still tagged `claude-code`. Safe to ship
   alone: no table rename, so no history is stranded, and the only
   user-visible change is a new column reading `claude-code`.
2. **pi reader.** Extract the shared resilient line loop (§2), then
   `parse_pi_file` plus the `id` + `timestamp` dedup key, unit-tested in
   isolation. No wiring yet.
3. **Multi-root `aggregate_windows`** (with the Claude-only `WindowTotals`
   guard, pi file-name ordering and the mtime prefilter) + `main.rs`
   roots-vec wiring, including §8's gate on the push and both resolvers.
   Signature changes and their test updates land here.
4. **Config** (`pi_enabled`, `pi_sessions_dir`, the `Copy` derive drop,
   `pub usable_dir`, `paths::pi_sessions_dir`, the §7 resolution order) +
   sample `config.toml` + README, including the "per tracked repo per app"
   wording. The pi `tests/cli.rs` tests land here, because the
   `PI_CODING_AGENT_SESSION_DIR` hook they use is part of this step.
5. **(Deferred, needs data)** pi compaction and branch-summary usage — only
   once real entries have been captured from real sessions and a model
   attribution rule is chosen (see Known limitations).
6. **(Fast-follow, separate spec)** per-app statusline chip.
