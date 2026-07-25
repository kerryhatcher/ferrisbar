# Release Automation: release-plz + cargo-dist + cargo-auditable (v7, Project 3 of 3)

## Purpose

Automate `ferrisbar`'s release process end to end: version bumping and
changelog generation from Conventional Commits (release-plz + its embedded
git-cliff), and cross-platform binary distribution via GitHub Releases
(cargo-dist), with `cargo-auditable` wired into those binary builds so the
compiled artifacts embed their own dependency tree. This is Project 3 of 3
(Project 1: manifest/local foundations, done; Project 2: CI pipeline, done).

## Background

`ferrisbar` has no git tags yet and has never been published to crates.io
(confirmed: `crates.io/api/v1/crates/ferrisbar` still returns 404). Its
entire commit history already follows Conventional Commits (`feat:`,
`fix:`, `docs:`, `chore:`, `ci:` prefixes throughout), which both release-plz
and git-cliff rely on to decide version bumps and generate changelog
entries — no history retrofitting needed.

During brainstorming, cargo-auditable's scope was expanded significantly
from the original ask: it only has meaning if `ferrisbar` distributes
binaries it built itself (a `cargo install ferrisbar` consumer compiles
their own binary and would need to run `cargo auditable install` themselves
— we can't force that on them). The user chose to go bigger: add real
cross-platform binary distribution via cargo-dist specifically so
cargo-auditable has something to attach to, rather than skip it or
half-wire it into nothing.

Two external tokens, which only the user can create, are required and are
called out explicitly rather than assumed:
- A crates.io API token (crates.io account settings), stored as the
  `CARGO_REGISTRY_TOKEN` GitHub secret — release-plz needs this to publish.
- A GitHub fine-grained PAT scoped to this repo (contents + workflows:
  write), stored as the `RELEASE_PLZ_TOKEN` GitHub secret — needed because
  GitHub does not let a workflow's own default `GITHUB_TOKEN` trigger other
  workflows when it pushes a tag (loop-prevention); without a real PAT,
  release-plz's tag push would never kick off cargo-dist's build.

## Scope (v7)

### In scope

- **release-plz**: `release-plz.toml` at repo root, plus
  `.github/workflows/release-plz.yml` with two jobs — `release-plz-pr`
  (opens/updates a release PR bumping `Cargo.toml`'s version and
  `CHANGELOG.md`, triggered on push to `main`, using the default
  `GITHUB_TOKEN` since it doesn't need to trigger downstream workflows) and
  `release-plz-release` (runs after that PR is merged, creates the git tag,
  publishes to crates.io, using the `RELEASE_PLZ_TOKEN` PAT so the tag push
  actually triggers cargo-dist). Uses `fetch-depth: 0` on checkout (full
  history needed for changelog generation).
- **git-cliff**: used via release-plz's embedded integration, not as a
  separate standalone tool/workflow step — release-plz calls it internally
  to generate `CHANGELOG.md` content from Conventional Commits. No separate
  `cliff.toml` unless release-plz's default changelog format turns out to
  need customization once seen against this repo's real commit history
  (determined during implementation, not pre-specified).
- **cargo-dist**: initialized via `cargo dist init`, generating
  `[workspace.metadata.dist]` in `Cargo.toml` and a self-generated
  `.github/workflows/release.yml` (via `cargo dist generate`), triggered on
  version tags. Target platforms: Linux (x86_64-gnu, x86_64-musl), macOS
  (x86_64, aarch64), Windows (x86_64) — covering the realistic set of
  workstations this tool is meant to install on.
- **cargo-auditable**: wired into cargo-dist's build step, so every
  distributed binary embeds its real dependency tree. The exact mechanism
  (a cargo-dist config option, or overriding its build command) is
  determined empirically during implementation by inspecting what
  `cargo dist init`/`generate` actually produce and cargo-dist's current
  documented options — not guessed here.
- **Release-object interop between the two tools**: cargo-dist's generated
  workflow may create/attach-to a GitHub Release for the tag differently
  depending on whether release-plz already created one with changelog notes
  — this integration detail is resolved empirically during implementation
  (checking each tool's actual current docs/config against the versions
  installed), not pre-specified, since guessing wrong here risks duplicate
  or conflicting releases.
- **SHA-pinning carries forward from Project 2**: every third-party Action
  in both new workflow files (`MarcoIeni/release-plz-action`, and whatever
  cargo-dist's generated workflow uses) gets pinned to a full commit SHA,
  not a mutable tag — including verifying whether cargo-dist's
  self-generated workflow already does this by default (some recent
  versions might) and only adding pins where it doesn't.
- **The two required secrets** (`CARGO_REGISTRY_TOKEN`, `RELEASE_PLZ_TOKEN`)
  — the user generates both via their own crates.io/GitHub account access;
  once they provide the values, they get stored via `gh secret set`.

### Out of scope

- Actually cutting the first real release (tagging v0.1.0 / publishing) —
  this project builds the automation; a human decision to trigger the
  first release through it is separate, made after this is built and
  verified, not bundled into "done."
- Graduating to 1.0.0, or any version-numbering policy decision — release
  version numbers are decided by release-plz reading Conventional Commits,
  not chosen here.
- Windows ARM64 or Linux ARM targets — not a stated requirement; the five
  targets above cover the realistic "many workstations" case from the
  original ask.
- A custom `cliff.toml` changelog format/template — only added if
  release-plz's default turns out to need it once seen against real output.
- Homebrew formula / other package-manager distribution beyond what
  cargo-dist generates by default (its own shell/PowerShell installer
  scripts) — not requested.

## Data flow / sequencing

1. Ask the user to generate the crates.io API token and the GitHub PAT
   (with the exact scopes/permissions needed), then store both as repo
   secrets via `gh secret set` once provided. This has to happen before the
   release-plz workflow can do anything meaningful, but doesn't block
   writing the workflow/config files themselves.
2. Install `cargo-dist`, run `cargo dist init` (target platforms above,
   GitHub CI backend), inspect what it generated (`Cargo.toml` metadata,
   `.github/workflows/release.yml`), and figure out the cargo-auditable
   wiring mechanism against the actual installed version's real options.
3. Install release-plz's local CLI (for local testing, e.g. `release-plz
   release-pr --dry-run`) if available for the installed version; write
   `release-plz.toml` and `.github/workflows/release-plz.yml`.
4. Verify both new workflow YAML files parse correctly and SHA-pin every
   third-party action (checking cargo-dist's generated file for any
   tag-based `uses:` lines needing conversion).
5. Push everything. Since no secrets exist until step 1 completes, the
   `release-plz-release` job would fail on its first real trigger without
   them — so this step's actual push either waits for the secrets to be in
   place, or ships with the understanding that the release job simply won't
   succeed until they are (surfaced clearly, not silently broken).
6. Verify: `release-plz-pr` actually opens a sane-looking release PR on a
   push to `main` (this alone can be verified even before the crates.io
   token exists, since opening a PR doesn't need it).

## Testing

- No new Rust tests — this is release infrastructure, not application code.
- Acceptance is empirical and external, same as Project 2: does
  `release-plz-pr` actually open a correct-looking PR (right version bump,
  right changelog content) on a real push to `main`? That's the
  verifiable milestone within this project's scope. Actually completing a
  full release (merge the PR, watch cargo-dist build and attach binaries,
  confirm crates.io publish) is explicitly a follow-on human decision (see
  Out of scope), not a requirement to close this project out.

## Risks / notes

- This project has more genuinely-undetermined-until-implemented details
  than Projects 1 or 2 (the cargo-auditable wiring mechanism, the
  release-plz/cargo-dist release-object interop, whether cargo-dist
  already SHA-pins) — this is disclosed explicitly rather than
  pre-guessed, matching how `deny.toml`'s license list and cargo-vet's
  exemption list were resolved by actually running the tools in Projects 1
  and 2, not assumed in advance.
- The two secrets are a hard external dependency with no workaround — if
  the user doesn't have a crates.io account with publish rights on
  `ferrisbar` (they do — they created it) or doesn't want to generate a
  PAT, the `release-plz-release` job and the cargo-dist trigger simply
  won't function; this is surfaced as a known limitation, not silently
  degraded.
- Because this repo has no releases yet, the very first run of this
  pipeline is untested territory for edge cases both tools generally
  handle well for established crates (first-ever release, no prior tag to
  diff against) but that are still worth watching closely rather than
  assuming will "just work" the same as a mature crate's 50th release.
