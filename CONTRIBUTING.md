# Contributing to ferrisbar

Thanks for taking the time. ferrisbar is small on purpose, so almost any
change is reviewable in one sitting.

By participating you agree to abide by our
[Code of Conduct](CODE_OF_CONDUCT.md). Security problems have a separate,
private path — see [SECURITY.md](SECURITY.md), and please do not open a public
issue for them.

## Finding something to work on

- Browse [open issues](https://github.com/kerryhatcher/ferrisbar/issues),
  especially anything tagged
  [`good first issue`](https://github.com/kerryhatcher/ferrisbar/labels/good%20first%20issue).
- For anything that changes behaviour or adds a flag, open an issue first so we
  can agree on the shape before you write code.
- Typos, doc fixes, and test additions never need an issue. Just send the PR.

## Development setup

You need a [Rust](https://www.rust-lang.org) toolchain — install it with
[rustup](https://rustup.rs). The crate's minimum supported Rust version
(**1.85.1**) is declared as `rust-version` in
[`Cargo.toml`](Cargo.toml); CI enforces it, so do not raise it casually.

```bash
git clone https://github.com/kerryhatcher/ferrisbar.git
cd ferrisbar
cargo build
cargo test
```

That is enough to make and test a change. The full check suite below needs a
few more tools.

### The check suite

Everything CI runs is also a [`just`](https://github.com/casey/just) recipe, so
you can reproduce a red build locally instead of guessing from logs:

```bash
just          # list every recipe
just ci       # run all of them, failing fast
```

| Recipe | Command | What it protects |
| ------ | ------- | ---------------- |
| `just fmt` | `cargo fmt --check` | Formatting, unmodified |
| `just lint` | `cargo clippy --all-targets -- -D warnings` | Lints — this crate opts into clippy `pedantic` and `nursery` |
| `just test` | `cargo test` | The 62-test suite |
| `just audit` | [`cargo audit`](https://github.com/rustsec/rustsec) | Known advisories in the dependency tree |
| `just msrv` | [`cargo msrv verify`](https://github.com/foresterre/cargo-msrv) | MSRV drift from dependency bumps |
| `just deny` | [`cargo deny check`](https://github.com/EmbarkStudios/cargo-deny) | Licenses, banned crates, duplicate versions (see [`deny.toml`](deny.toml)) |
| `just trivy` | [`trivy fs`](https://github.com/aquasecurity/trivy) | Vulnerabilities and hardcoded secrets on disk |
| `just vet` | [`cargo vet check`](https://github.com/mozilla/cargo-vet) | Supply-chain trust policy (see [`supply-chain/`](supply-chain/)) |
| `just geiger` | [`cargo geiger`](https://github.com/geiger-rs/cargo-geiger) | `unsafe` usage — informational, never fails the build |

Install what you are missing:

```bash
cargo install just cargo-audit cargo-msrv cargo-deny cargo-vet cargo-geiger
# trivy: https://trivy.dev/latest/getting-started/installation/
```

If you would rather not install all of it, push the branch and let CI run the
rest — `just fmt lint test` catches nearly everything on its own.

## Code standards

- **Tests come with the change.** Unit tests live in a `mod tests` block beside
  the code; end-to-end tests that drive the real binary live in
  [`tests/cli.rs`](tests/cli.rs). A bug fix should come with the test that
  would have caught it.
- **Clippy is not advisory.** `pedantic` and `nursery` are on. If a lint is
  genuinely wrong for a piece of code, `#[allow(...)]` it *with a comment
  explaining why* — see the cast in
  [`src/context_bar.rs`](src/context_bar.rs) for the house style.
- **Keep the dependency tree tiny.** Two runtime crates is a feature, not an
  accident. A PR adding a third needs to argue for it, and will also need a
  `cargo vet` entry.
- **Never panic on input.** Bad, partial, or absent stdin must degrade to a
  shorter statusline. A panic here corrupts somebody's prompt on every render.

## Pull requests

1. Branch off `main`. Direct pushes to `main` are blocked by branch protection.
2. Write [Conventional Commits](https://www.conventionalcommits.org) —
   `feat:`, `fix:`, `docs:`, `chore:`, `ci:`, `refactor:`, `test:`.
   [release-please](https://github.com/googleapis/release-please) derives the
   version bump and the changelog from these, so the prefix decides whether
   your change ships as a minor or a patch. `feat!:` or a `BREAKING CHANGE:`
   footer triggers a major.
3. Run `just ci`, or at minimum `just fmt lint test`.
4. Open the PR. All CI checks must pass before merge — they run on Linux,
   macOS, and Windows.
5. Do not bump the version in `Cargo.toml` or edit `CHANGELOG.md` by hand.
   Release automation owns both files.

## Releases

Releases are automated. Merging to `main` updates a standing release PR; when
that PR is merged, the tag, the GitHub release, the cross-platform binaries,
and the crates.io publish all happen without anyone touching a keyboard. The
only manual step is deciding to merge it.
