# Manifest & Local Tooling Foundations (v5, Project 1 of 3)

## Purpose

Bring `ferrisbar`'s `Cargo.toml` and licensing up to current (2025-2026)
Rust OSS best practices, as researched and gap-analyzed against this repo:
dual MIT/Apache-2.0 licensing, a declared MSRV (`rust-version`), and a
`[lints.clippy]` configuration with stricter lint groups enabled. This is
Project 1 of a 3-project sequence (Project 2: CI pipeline — GitHub Actions,
Dependabot, cargo-semver-checks, cargo-vet; Project 3: release automation —
release-plz/git-cliff, cargo-auditable). Projects 2 and 3 depend on this one
being done first, since they reference the MSRV and run the lints this
project establishes.

## Background

`ferrisbar` (renamed from `mystatusline` earlier this session, not yet
published to crates.io) currently has: single MIT licensing (`LICENSE`,
`license = "MIT"` in `Cargo.toml`), no `rust-version` field, edition 2021,
and no `[lints.clippy]` section — clippy is only gated via `-D warnings` on
the command line in `justfile`'s `lint` recipe. A 6-agent research sweep
(3 Haiku + 3 Sonnet, covering repo hygiene, Cargo.toml conventions, code
quality tooling, CI/CD, supply-chain security, and release management)
flagged all three of these as current, cheap, worth-doing gaps — as opposed
to items explicitly deemed lower-priority or project-size-inappropriate
(SBOM generation, OpenSSF Scorecard), which are out of scope for all three
projects in this sequence.

Nothing has been published externally yet (crates.io returns 404 for
`ferrisbar`; the GitHub repo has been renamed but several recent commits are
still unpushed), so licensing and manifest changes now carry no
backward-compatibility risk.

## Scope (v5)

### In scope

- **Dual licensing**: rename `LICENSE` → `LICENSE-MIT` (unchanged text), add
  `LICENSE-APACHE` (standard Apache License 2.0 text, same copyright
  holder/year as the existing MIT file), and change `Cargo.toml`'s
  `license` field from `"MIT"` to `"MIT OR Apache-2.0"`.
- **MSRV declaration**: install `cargo-msrv` (not present on this machine),
  run it against the real crate to empirically determine the actual
  minimum supported Rust version (known floor: at least 1.82, since
  `Option::is_none_or` — stabilized in 1.82.0 — is used in `src/todo.rs`),
  and add the resulting `rust-version = "X.Y.Z"` to `Cargo.toml`. Add a
  `msrv` recipe to `justfile` running `cargo msrv verify`, and add it to
  the `ci` recipe's dependency chain, so MSRV drift from future dependency
  bumps is caught locally, not just eventually in Project 2's CI job.
- **`[lints.clippy]` in Cargo.toml**: add `all = "warn"`, `pedantic =
  "warn"`, `nursery = "warn"`. Then, empirically: run `cargo clippy
  --all-targets` against the real codebase, and for each lint that fires,
  either fix the flagged code (if the suggestion is a genuine improvement)
  or add a targeted allow (a `#[allow(clippy::lint_name)]` at the
  call-site, or a config-level allow under `[lints.clippy]` if a lint
  fires pervasively and is judged not worth fixing everywhere) — every
  allow gets a one-line comment stating why. No blind copy-pasted preset
  allow-list from elsewhere.
- Verify `just ci` (with the new `msrv` recipe added to its chain) still
  passes end-to-end after all changes.

### Out of scope (belongs to Project 2 or 3, or explicitly rejected)

- GitHub Actions CI workflow, Dependabot config, cargo-semver-checks,
  cargo-vet — Project 2.
- release-plz, git-cliff, cargo-auditable — Project 3 (cargo-auditable's
  value is embedding dependency info into *released* binaries, so it
  belongs where release builds happen, not here).
- SBOM generation, OpenSSF Scorecard — explicitly deemed not worth it for
  this project's current size, per the research sweep.
- Actually publishing to crates.io or pushing to GitHub — this project
  only prepares the manifest; publishing/pushing decisions stay separate,
  as they have been throughout this session.

## Data flow / sequencing

1. Licensing: create `LICENSE-APACHE`, rename `LICENSE` → `LICENSE-MIT`,
   update `Cargo.toml`'s `license` field. Verify `cargo build` still works
   (license field changes don't affect compilation, but confirms no typo
   broke the manifest).
2. MSRV: `cargo install cargo-msrv`, run `cargo msrv find` (or `verify`
   once a candidate version is known) against this crate specifically —
   not a guess — add the discovered `rust-version` to `Cargo.toml`, add the
   `justfile` `msrv` recipe, wire it into `ci`.
3. Clippy lints: add the `[lints.clippy]` section, run `cargo clippy
   --all-targets`, triage every finding (fix or targeted-allow-with-reason),
   iterate until clean.
4. Run `just ci` end-to-end to confirm everything (including the new `msrv`
   recipe) passes together.
5. Commit. Given the small, empirical, low-branching nature of this work,
   implementation happens directly in this session rather than via
   subagent dispatch.

## Testing

- No new automated tests are needed — this project changes manifest
  metadata and lint configuration, not application behavior. The existing
  62-test suite (`cargo test`, already part of `just ci`) is the regression
  check that nothing behavioral broke while triaging clippy findings (a
  clippy-suggested code change that altered behavior would be caught here).
- `just ci` passing end-to-end (including the new `msrv` recipe) is the
  acceptance criterion for this project as a whole.

## Risks / notes

- `cargo-msrv`'s empirical check can be slow (it may compile against
  several candidate toolchains via `rustup`) — this is a one-time local
  cost, not a recurring one once the `rust-version` field is set (the
  `msrv verify` recipe only checks against the *one* pinned version
  thereafter, not a search).
- Enabling `clippy::pedantic`/`clippy::nursery` may surface a nontrivial
  number of findings on first run, given the codebase was written across
  several prior sessions without these groups enabled. Triage is expected
  to take real iteration; the spec doesn't pre-commit to a specific final
  allow-list because it can't be known until clippy is actually run against
  this exact code.
- `nursery` lints are explicitly unstable/experimental in upstream clippy
  (the group's own documented caveat) and can change behavior across clippy
  versions; this is accepted as normal churn to react to later, not a
  reason to exclude the group now, per the research's endorsement of
  enabling it as "warn" (not "deny") specifically to keep it non-blocking
  when clippy itself changes its mind about a lint.
