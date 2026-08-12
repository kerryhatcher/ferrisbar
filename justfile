# Run `just` with no args to list recipes, or `just ci` to run all checks.

# Check formatting without modifying files.
fmt:
    cargo fmt --check

# Reformat in place. `fmt` only verifies; this is the one that writes.
fmt-fix:
    cargo fmt

# Lint, treating warnings as errors.
lint:
    cargo clippy --all-targets -- -D warnings

# Run the test suite.
test:
    cargo test

# Run the test suite with the analytics feature enabled — the default
# `test` recipe never compiles src/analytics/store.rs or report.rs at
# all, since the feature is optional and off by default.
test-analytics:
    cargo test --features analytics

# Lint the analytics feature's code the same way `lint` covers the
# default build.
lint-analytics:
    cargo clippy --all-targets --features analytics -- -D warnings

# Check dependencies against the RustSec advisory database.
audit:
    cargo audit

# Verify the crate still builds on its declared MSRV (Cargo.toml's
# rust-version), catching drift from dependency bumps.
msrv:
    cargo msrv verify

# Check licenses, banned crates, duplicate versions, and dependency sources
# (see deny.toml).
deny:
    cargo deny check

# Check licenses/bans for redb (and any of its transitive deps) too —
# deny.toml's [graph] all-features = false means the default `deny`
# recipe skips anything gated behind an inactive feature. Note:
# --all-features is a global cargo-deny flag, so it must precede the
# `check` subcommand — `cargo deny check --all-features` errors out.
deny-analytics:
    cargo deny --all-features check

# Scan the filesystem for known vulnerabilities and hardcoded secrets.
trivy:
    trivy fs --scanners vuln,secret --exit-code 1 --skip-dirs target .

# Check the dependency tree against our supply-chain trust policy
# (see supply-chain/config.toml).
vet:
    cargo vet check

# Report unsafe-code usage in this crate and its dependency tree.
# Informational only — not a pass/fail gate (the leading `-` ignores
# cargo-geiger's exit code, which can be nonzero on internal warnings
# unrelated to actual unsafe-code findings).
geiger:
    -cargo geiger

# Run every check. Fails fast on the first failing recipe.
ci: fmt lint lint-analytics test test-analytics audit msrv deny deny-analytics trivy vet geiger
