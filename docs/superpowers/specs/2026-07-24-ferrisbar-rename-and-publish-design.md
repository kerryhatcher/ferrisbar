# Rename to `ferrisbar` and publish to crates.io (v4)

## Purpose

Rename the `mystatusline` crate/binary to `ferrisbar` and publish it to
crates.io, so it can be installed on any workstation with `cargo install
ferrisbar` instead of requiring a local clone and `cargo install --path .`.

## Background

`mystatusline` has been a single-user, single-machine tool: installed via
`cargo install --path .` from a local checkout, wired into
`~/.claude/settings.json` (not yet actually wired — that setting currently
still points at a Cognee statusline script, so this rename has zero impact
on any live config). The user now intends to share it and install it on
many workstations, which means:

- It needs a name that reads as a shared tool rather than a personal one.
  `mystatusline` was confirmed available on crates.io, but ten candidate
  names (mixing Rust-mascot/crab puns, Claude wordplay, and status-bar
  cheekiness) were generated and checked for availability via
  `crates.io/api/v1/crates/<name>` (note: this endpoint returns 403 without
  a descriptive `User-Agent` header, not 404 — a real 404 indicates the name
  is genuinely available). `ferrisbar` (Ferris, the Rust mascot + "bar" as
  in status bar) was chosen from that list.
- It needs to be installable without a local clone, i.e. published to
  crates.io.

The GitHub repository (`kerryhatcher/mystatusline`, public, MIT-licensed)
and local checkout (`~/projects/mystatusline`) both currently use the old
name.

## Scope (v4)

### In scope

- **Cargo.toml rename**: `[package] name` and `[[bin]] name` both become
  `"ferrisbar"`.
- **Cargo.toml publish metadata** (crates.io requires `description` and
  `license` at minimum to accept a publish; adding the full recommended
  set for discoverability):
  ```toml
  description = "A Claude Code statusline renderer, written in Rust"
  license = "MIT"
  repository = "https://github.com/kerryhatcher/ferrisbar"
  readme = "README.md"
  keywords = ["claude-code", "statusline", "cli"]
  categories = ["command-line-utilities"]
  ```
  (`command-line-utilities` confirmed as a valid crates.io category slug via
  `crates.io/api/v1/categories/command-line-utilities`.)
- **`tests/cli.rs` mechanical rename**: every `env!("CARGO_BIN_EXE_mystatusline")`
  becomes `env!("CARGO_BIN_EXE_ferrisbar")` — **required**, not cosmetic:
  Cargo generates this env var name from the `[[bin]]` target's name at
  compile time, so the old form will fail to compile once the bin is
  renamed. Matching `.expect("failed to spawn mystatusline")` panic-message
  text is updated alongside for consistency, since those lines are already
  being touched.
- **`src/main.rs`'s usage string**: instead of hardcoding the program name
  (`"Usage: mystatusline [setup [--project]]"` → `"Usage: ferrisbar
  [setup [--project]]"`), derive it from `argv[0]` at runtime via
  `env::args().next()`, falling back to `"ferrisbar"` if `argv[0]` is
  somehow absent. This is a small, idiomatic improvement (common in CLI
  tools) that makes the usage message self-correcting if the binary is ever
  renamed, symlinked, or invoked under a different name again.
- **`README.md`**: title, every command example (`mystatusline` →
  `ferrisbar`), and a new primary install method:
  ```bash
  cargo install ferrisbar
  ```
  ahead of the existing `cargo install --path .` instructions, which stay
  documented as the contributor/dev-build route (building from a local
  checkout of the repo rather than from crates.io).
- **GitHub repository rename**: `gh repo rename ferrisbar` (run from inside
  the local checkout, so the `origin` remote URL updates automatically;
  GitHub auto-redirects the old `kerryhatcher/mystatusline` URL, so
  existing external links/clones keep working).
- **Verification before publishing**: `cargo build`, `cargo test` (full
  suite, confirming the rename didn't break anything), then `cargo publish
  --dry-run` to validate the package without actually publishing.
- **The actual `cargo publish`**: gated on an explicit go-ahead from the
  user at that exact step, separate from every other step in this plan
  being "done." Publishing a crate name is effectively permanent — even a
  later `cargo yank` never frees the name for reuse — so this is not
  bundled into an automated task sequence the way the rest of this plan is.

### Out of scope

- Renaming the local directory (`~/projects/mystatusline` stays as-is) —
  purely cosmetic, no functional effect on the build, install, or publish
  process. Can be done separately later if desired.
- Rewriting historical spec/plan documents under `docs/superpowers/` that
  reference the old name — they document what was built at the time under
  that name; rewriting them would falsify the historical record.
- Renaming the test-fixture path strings in `src/setup.rs`
  (`"/bin/mystatusline"`, `"/usr/local/bin/mystatusline"`, etc.) — these are
  arbitrary example absolute paths used to test that `apply_statusline_update`
  round-trips JSON correctly; they carry no relationship to this crate's own
  name and renaming them serves no purpose.
- Any change to `LICENSE` (copyright holder/year are unaffected by a rename).
- CI/automated publish workflows (e.g. a GitHub Action that publishes on
  tag push) — first publish is manual; automating future releases is a
  separate concern for a later spec if wanted.

## Data flow / sequencing

1. Rename in `Cargo.toml` (package + bin name, plus new metadata fields).
2. Update `tests/cli.rs` (env var name) and `src/main.rs` (usage string,
   made `argv[0]`-driven).
3. Run `cargo build` and `cargo test` — confirm everything still compiles
   and passes under the new name before touching anything external.
4. Update `README.md`.
5. Rename the GitHub repo (`gh repo rename ferrisbar`, from inside the repo).
6. Commit the rename (Cargo.toml, tests/cli.rs, src/main.rs, README.md
   together — this is one coherent change, not several).
7. `cargo publish --dry-run` — validate packaging.
8. **Stop and confirm with the user** before proceeding.
9. `cargo publish` (only after explicit go-ahead).

## Testing

- The existing test suite (62 tests: unit + integration) is the correctness
  check for the rename itself — if the rename is done right, every existing
  test keeps passing unmodified in behavior (only the `CARGO_BIN_EXE_*` env
  var name and a couple of message strings change literally; no test
  assertions about program behavior change).
- No new test coverage is needed for `cargo publish` itself — `--dry-run`
  is the verification step, run before the real publish.
- Manual verification after publish: from a machine/directory without a
  local checkout, `cargo install ferrisbar` followed by `ferrisbar setup`
  should behave identically to today's `cargo install --path .` +
  `mystatusline setup` flow. (This spec doesn't mandate spinning up a
  separate clean environment to test this — the existing test suite plus
  `--dry-run`'s packaging validation is considered sufficient confidence
  for a first publish of a small, already-well-tested crate.)

## Risks / notes

- `cargo login` credentials already exist in this environment
  (`~/.cargo/credentials.toml`, present from a prior, unrelated `cargo
  login`), so `cargo publish` is technically executable from here — this
  spec does not change that; it only adds the explicit-confirmation gate
  described above around actually invoking it.
- The crates.io name-check method (`curl -A "<UA>"
  https://crates.io/api/v1/crates/<name>`, 404 = available) is not
  atomic — a name could theoretically be claimed by someone else between
  now and the actual `cargo publish`. This is treated as an acceptable,
  vanishingly unlikely race for a private tool name like `ferrisbar`, not
  worth adding retry/re-check logic for.
- Once the GitHub repo is renamed, any external bookmark to the old
  `kerryhatcher/mystatusline` URL will redirect (GitHub's standard
  behavior) rather than break — but `git clone` of the *old* SSH/HTTPS URL
  from a machine that has it cached differently, or any hardcoded reference
  to the old URL outside this repo (e.g. in unrelated notes), won't be
  found and updated by this plan, since that's outside this repo's control.
