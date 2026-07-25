# Release Automation: release-please + hand-rolled cross-platform builds (v7, Project 3 of 3)

## Purpose

Automate `ferrisbar`'s release process end to end: version bumping and
changelog generation from Conventional Commits (release-please), and
cross-platform binary distribution via GitHub Releases, with
`cargo-auditable` wired into those binary builds so the compiled artifacts
embed their own dependency tree. This is Project 3 of 3 (Project 1:
manifest/local foundations, done; Project 2: CI pipeline, done).

**This spec supersedes an earlier draft** that used release-plz +
cargo-dist + a GitHub PAT for cross-workflow triggering. The user pointed
at `~/projects/rustywx`, a sibling Rust CLI project that already solved
this exact problem — a single workflow with chained jobs, no PAT — and
asked to reuse that pattern. Both the tool (release-please, not
release-plz) and the build approach (hand-rolled matrix build/package/
upload, not cargo-dist) were confirmed with the user as direct swaps.

## Background

`ferrisbar` has no git tags yet and has never been published to crates.io
(confirmed: `crates.io/api/v1/crates/ferrisbar` returns 404). Its commit
history already follows Conventional Commits throughout, which
release-please relies on to decide version bumps and changelog entries.

**The rustywx pattern, read directly from
`~/projects/rustywx/.github/workflows/release-please.yml`:** a single
workflow, triggered on `push: branches: [main]`, with three jobs chained
via `needs:`/`if:` rather than three separately-triggered workflows:

1. **`release-please`** — runs `googleapis/release-please-action`, which
   either opens/updates a release PR (version bump + changelog), or — when
   that PR was just merged — creates the tag and GitHub Release directly,
   in this same triggered run. Exposes `releases_created` (boolean) and
   `tag_name` as job outputs for the jobs below to consume.
2. **`build`** — `needs: release-please`, gated on
   `needs.release-please.outputs.releases_created == 'true'`. Matrix build
   across target platforms, packages each binary into an archive, uploads
   it to the release `release-please` already created via
   `softprops/action-gh-release`.
3. **`publish-crate`** — `needs: [release-please, build]`, same gate.
   Runs `cargo publish` using the `CARGO_REGISTRY_TOKEN` secret, after
   every target build has succeeded.

**Why this avoids the PAT entirely:** there is no second workflow being
triggered by a tag push at all. Everything — PR management, tagging,
building, publishing — happens inside jobs of the *one* workflow run that
the ordinary push-to-main event triggered, using the default
`GITHUB_TOKEN` throughout. GitHub's loop-prevention rule (a workflow's own
default token can't trigger *other* workflows) simply never comes into
play, because nothing here depends on triggering another workflow.

**Only one secret is needed**: `CARGO_REGISTRY_TOKEN` (a crates.io API
token, generated via the user's crates.io account — something only they
can do), stored as a GitHub secret. No PAT, no `RELEASE_PLZ_TOKEN`.

## Scope (v7)

### In scope

- **`release-please-config.json`** and **`.release-please-manifest.json`**
  at the repo root: `release-type: "rust"`, single root package (unlike
  rustywx's workspace-with-subpackage setup — `ferrisbar` is a plain
  single-crate repo, so the config is simpler: package path `"."`,
  `package-name: "ferrisbar"`), manifest seeded at `"0.1.0"` (the current
  `Cargo.toml` version — release-please needs the manifest to already
  reflect the last-known-released version, even for a first release).
- **`.github/workflows/release-please.yml`** — the three-job pattern above,
  adapted for `ferrisbar`:
  - `release-please` job: same shape as rustywx's, referencing this repo's
    config/manifest file names.
  - `build` job: matrix matching rustywx's exact target list for
    consistency — `x86_64-unknown-linux-gnu` (ubuntu-22.04),
    `aarch64-unknown-linux-gnu` (ubuntu-22.04-arm), `aarch64-apple-darwin`
    (macos-14), `x86_64-pc-windows-msvc` (windows-latest). No GUI/audio
    system build dependencies are needed (that step in rustywx is specific
    to it being a GUI app; `ferrisbar` is a plain CLI with only `serde`/
    `serde_json` as runtime deps). Uses `cargo auditable build --release
    --locked --target <target>` instead of plain `cargo build` — this is
    where cargo-auditable actually gets wired in, installed via
    `taiki-e/install-action` in this job. Packages `README.md`,
    `LICENSE-MIT`, and `LICENSE-APACHE` (both license files, unlike
    rustywx's single `LICENSE`) alongside the binary into a `tar.gz`
    (unix) or `.zip` (Windows) archive, uploaded via
    `softprops/action-gh-release`.
  - `publish-crate` job: same shape as rustywx's, `cargo publish --locked`
    using `CARGO_REGISTRY_TOKEN`.
  - Actions reused at their already-verified pins from Project 2's
    `ci.yml` where the same action is used there too (`actions/checkout`,
    `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, `taiki-e/install-action`
    — kept consistent within this repo rather than copying rustywx's
    possibly-older pins for those same actions). Two actions new to this
    repo (`googleapis/release-please-action`, `softprops/action-gh-release`)
    get freshly-resolved SHA pins, verified against their real current tags
    the same way Project 2's pins were (`git ls-remote`, not guessed).
- **The `CARGO_REGISTRY_TOKEN` secret**: the user generates it via
  crates.io account settings; stored via `gh secret set` once provided.

### Out of scope

- release-plz, cargo-dist, and any GitHub PAT — all dropped per the
  rustywx-pattern decision above.
- Actually cutting the first real release — this project builds the
  automation; triggering the first release through it is a separate human
  decision made after this is built and verified (matches the prior draft's
  stance, unchanged by the tool swap).
- Graduating to 1.0.0 or any version-numbering policy — release-please
  decides version numbers from Conventional Commits, not chosen here.
- Windows ARM64, or any target beyond the four listed — matches rustywx's
  own scope, itself already a considered "realistic workstation" list.
- A custom release-please changelog template/config beyond the basics
  (`release-type`, package name/path) — only added if the default output
  needs it once seen against this repo's real commit history.

## Data flow / sequencing

1. Write `release-please-config.json` and `.release-please-manifest.json`.
2. Write `.github/workflows/release-please.yml` (all three jobs), reusing
   already-verified action pins from `ci.yml` where applicable, freshly
   resolving the two new ones.
3. Validate the YAML parses (same check used in Project 2) and confirm no
   SHA was copied without independent verification.
4. Ask the user to generate the crates.io API token; store it as
   `CARGO_REGISTRY_TOKEN` via `gh secret set` once provided.
5. Push. Confirm the `release-please` job runs and opens a sane release PR
   on this ordinary push (this alone is verifiable without the crates.io
   token existing yet, same as the prior draft's stance — opening a PR
   doesn't need it).

## Testing

- No new Rust tests — release infrastructure, not application code.
- Acceptance is the same empirical milestone as the superseded draft: does
  the `release-please` job open a correct-looking PR (right version bump,
  right changelog content) on a real push to `main`? Completing a full
  release (merging that PR, watching `build`/`publish-crate` actually run)
  is a follow-on human decision, not required to close this project out.

## Risks / notes

- This is a straight adaptation of a pattern already proven working in a
  sibling repo, which meaningfully de-risks this project compared to the
  superseded draft (which had multiple genuinely-undetermined integration
  details between two separate tools). The main remaining unknown is
  release-please's behavior on a repo with zero prior releases/tags —
  worth watching on the first real run rather than assumed identical to
  rustywx's now-8-releases-in history.
- `cargo-auditable` is the one piece with no precedent in rustywx (it
  doesn't use it) — wiring it in is a one-word change (`cargo auditable
  build` instead of `cargo build`) per cargo-auditable's own documented
  design as a drop-in wrapper, but is still worth confirming empirically
  (inspect the built binary) once implemented, not just assumed to work
  from the command substitution alone.
