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
ci: fmt lint test audit msrv deny trivy vet geiger
