# CI Pipeline: GitHub Actions, Dependabot, cargo-vet (v6, Project 2 of 3)

## Purpose

Automate the checks Project 1 made possible locally (`just ci`) so they run
on every push/PR via GitHub Actions, add automated dependency-update PRs via
Dependabot, and establish a supply-chain trust policy via cargo-vet — per
the research sweep's CI/CD and security findings, and per the user's
explicit item list. This is Project 2 of 3 (Project 1: manifest/local
foundations, done; Project 3: release automation, not started). Depends on
Project 1's `rust-version` and `[lints.clippy]` being in place, since CI
jobs reference both.

## Background

`ferrisbar` currently has no `.github/` directory at all — no CI workflow,
no Dependabot config. Every check (`just ci`: fmt, clippy, test, audit,
msrv, deny, trivy, geiger) only runs when a human remembers to run it
locally. The repo is public, renamed to `kerryhatcher/ferrisbar`, not yet
published to crates.io, currently worked on by a single maintainer pushing
directly to `main` (no PR workflow in use yet).

Two scope decisions were resolved during brainstorming:
- **cargo-semver-checks dropped from scope entirely.** It diffs a crate's
  public Rust API (`[lib]` target) between versions; `ferrisbar` is
  bin-only with no `[lib]` section (one was added and reverted earlier in
  this project specifically because it caused duplicate-compilation
  warnings), so there's no API surface for the tool to check. The crate's
  real "interface" is its CLI behavior, which this tool doesn't inspect.
- **cargo-vet is in scope**, per the user's explicit reasoning: they expect
  large enterprise users and a growing dependency footprint, where the
  research's "skip for small trees" framing no longer applies.

## Scope (v6)

### In scope

- **All third-party Actions pinned to a full commit SHA, never a mutable
  tag or branch ref** (`uses: owner/repo@<40-char-sha> # vX.Y.Z` — the
  version in a trailing comment for human readability, the SHA is what
  actually gets executed). This applies to every third-party action used
  below (`actions/checkout`, `dtolnay/rust-toolchain`,
  `Swatinem/rust-cache`, `aquasecurity/trivy-action`,
  `taiki-e/install-action`) — a tag like `@v4` or `@stable` can be
  silently re-pointed at different code later (by the action's own repo
  being compromised, or a tag being moved), which a SHA pin prevents. The
  exact SHAs are resolved at implementation time (each action's current
  release commit), not guessed here.
- **`.github/workflows/ci.yml`** — triggers on `push: branches: [main]` and
  `pull_request:`. Jobs, each installing tools via `taiki-e/install-action`
  (prebuilt binaries, not slow from-source `cargo install`) and caching via
  `Swatinem/rust-cache`:
  - `test`: matrix `os: [ubuntu-latest, macos-latest, windows-latest]` ×
    `stable`, via SHA-pinned `dtolnay/rust-toolchain` with `toolchain:
    stable`, runs `just test`.
  - `lint`: `ubuntu-latest` only, runs `just fmt` then `just lint`.
  - `msrv`: `ubuntu-latest` only, installs the exact toolchain named by
    `Cargo.toml`'s `rust-version` (currently 1.85.1) via the same
    SHA-pinned `dtolnay/rust-toolchain` action with an explicit `toolchain:
    1.85.1` input (not a live `cargo-msrv find` search — CI pins and
    verifies, it doesn't rediscover), runs `cargo check --all-targets` and
    `cargo test`.
  - `security`: `ubuntu-latest` only, runs `just audit`, `just deny`, a
    trivy scan via `aquasecurity/trivy-action` (scanners: vuln, secret;
    same flags as the local `trivy` recipe), and `just geiger`
    (non-blocking, matching local behavior — the job step does not fail on
    geiger's exit code).
  - `vet`: `ubuntu-latest` only, runs `cargo vet check`.
  - Every job invokes the *existing* `just` recipes rather than duplicating
    raw `cargo`/tool commands in YAML — the justfile stays the single
    source of truth for what each check does, locally and in CI.
- **`.github/dependabot.yml`** — weekly schedule, two `package-ecosystem`
  entries: `cargo` (root directory) and `github-actions` (root directory,
  so pinned Action versions in `ci.yml` get bumped too).
- **cargo-vet setup**:
  - Install `cargo-vet`, run `cargo vet init` (creates `supply-chain/`:
    `config.toml`, `audits.toml`, `imports.lock`).
  - Add Mozilla's and Google's published audit sets as trusted imports in
    `supply-chain/config.toml`.
  - Run `cargo vet check` (or `cargo vet regenerate exemptions` /
    equivalent) against the real ~14-crate dependency tree to see what the
    imports don't already cover. For whatever remains: `cargo vet exempt
    <crate> <version>` (an honest, explicitly-tracked "not yet reviewed"
    marker) — never `cargo vet certify`, since that command asserts a real
    human security review took place, which fabricating would be a false
    attestation. The exact list of exemptions needed isn't knowable until
    this is actually run — not pre-specified here.
- **Branch protection on `main`**: via `gh api`, require the status checks
  above (all matrix legs of `test`, plus `lint`, `msrv`, `security`, `vet`)
  to pass before a PR can merge. This does not restrict direct pushes to
  `main` (GitHub cannot retroactively block a push that already happened by
  the time CI runs on it) — it only gates PR merges, which matters once
  PRs exist (Dependabot bumps, future contributors) rather than changing
  the user's current direct-push workflow.

### Out of scope

- cargo-semver-checks — explicitly dropped (see Background).
- Release automation (release-plz, git-cliff, cargo-auditable, cargo-dist)
  — Project 3.
- SBOM generation, OpenSSF Scorecard — explicitly rejected in the original
  research gap-analysis for a project this size (still true even with the
  enterprise-growth framing that brought cargo-vet into scope — those two
  weren't reconsidered).
- Requiring PR reviews/approvals, or restricting direct pushes to `main` —
  not asked for; only "require status checks" was requested.
- A nightly/beta Rust channel CI job — the research mentioned this as
  common but optional ("allow-failure, for early warning"); not in the
  user's item list and adds CI cost without a concrete need this crate has
  today (no nightly-only features in use). Can be added later if desired.

## Data flow / sequencing

1. Install `cargo-vet`, run `cargo vet init`, add the Mozilla/Google import
   config, run `cargo vet check` (or the regenerate-exemptions equivalent)
   against the real dependency tree, resolve whatever's left with
   `cargo vet exempt`, confirm `cargo vet check` passes clean locally.
2. Write `.github/workflows/ci.yml` (including the `vet` job, now that
   `supply-chain/` exists and passes) and `.github/dependabot.yml`.
3. Push all of the above to GitHub in one push (required — Actions/
   Dependabot only activate once the files exist on the default branch)
   and confirm the workflow runs and every job is visible/green in the
   Actions tab.
4. Once CI has run at least once (so the exact job names exist for GitHub
   to reference), configure branch protection on `main` requiring those
   checks.
5. Verify end-to-end: `just ci` still passes locally (nothing about this
   project should change local-check behavior, only add CI automation
   around the same checks), and the GitHub Actions tab shows a clean run.

## Testing

- No new Rust unit/integration tests — this project adds infrastructure
  (YAML workflow config, Dependabot config, supply-chain trust config), not
  application behavior.
- The acceptance test is empirical and external: pushing the workflow files
  and observing an actual green CI run on GitHub, and confirming branch
  protection shows the expected required checks in the repo's Settings ->
  Branches page (or via `gh api repos/.../branches/main/protection`).

## Risks / notes

- This is the first time this repo's checks run somewhere other than the
  user's own machine — a job could fail for environment reasons (e.g. a
  tool version pinned differently in CI than locally) that never surfaced
  locally. Expect a possible iteration cycle on first real CI run, not a
  one-shot success.
- `cargo vet check`'s CI job will fail on every future dependency addition
  until that new crate is exempted or certified — this is the intended
  behavior (per the user's own stated reason for wanting cargo-vet), but
  it does mean dependency bumps (including from Dependabot) will need a
  `cargo vet` step as part of accepting them, not just a version bump.
- Branch protection is configured via a direct GitHub API call (`gh api`),
  which is an external, shared-state-affecting action — this will be
  confirmed with the user again at the moment of execution, not assumed
  from this spec's approval alone.
