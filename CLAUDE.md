# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

ferrisbar is a Claude Code statusline renderer: it reads a JSON payload on stdin
and prints one line to stdout. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
full check-suite table and tool install commands.

## Checks

Run `just ci` before claiming a change is done — it chains fmt, lint, test,
audit, msrv, deny, trivy, vet, and geiger, failing fast. `just fmt` is
`cargo fmt --check` (verify only); use `just fmt-fix` to actually reformat.

## Invariants

- **Never panic on input.** Partial or wrong-typed JSON must degrade to a
  shorter statusline; stdin that is not JSON, or is empty, must print nothing
  and exit `0`. A panic here corrupts someone's prompt on every render.
- **MSRV is 1.85.1** (`rust-version` in Cargo.toml, pinned in CI). Do not use
  stdlib APIs stabilized after it, and do not raise it casually.
- **Four runtime dependencies is deliberate.** `serde` and `serde_json` for
  the payload, `toml` for the config file, `flate2` for log rotation. A
  fifth needs a justification and a `cargo vet` entry in `supply-chain/`.
  `toml` is version-pinned below 1.2 because it sits one patch under our
  MSRV floor.

## Code standards

- Clippy `pedantic` and `nursery` are enabled and CI runs `-D warnings`. When a
  lint is genuinely wrong, `#[allow(...)]` it *with a comment explaining why* —
  see `src/context_bar.rs:12` for the house style.
- Unit tests live in a `mod tests` block beside the code; end-to-end tests that
  drive the real binary live in `tests/cli.rs`. A bug fix ships with the test
  that would have caught it.

## Repo etiquette

- Branch off `main` — direct pushes are blocked by branch protection.
- Use [Conventional Commits](https://www.conventionalcommits.org); release-please
  derives the version bump and changelog from the prefix.
- **Never hand-edit the `version` field in `Cargo.toml` or `CHANGELOG.md`.**
  Release automation owns both files.
