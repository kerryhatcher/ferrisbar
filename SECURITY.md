# Security Policy

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub
issues.**

Report privately through either channel:

- [GitHub's private vulnerability reporting](https://github.com/kerryhatcher/ferrisbar/security/advisories/new)
  — preferred, since it keeps the report, the fix, and the advisory together.
- Email **kerry@kerryhatcher.com** with `ferrisbar security` in the subject.

A useful report includes the version or commit affected, the platform, what an
attacker gains, and the smallest input or configuration that demonstrates it.

## What to expect

| Stage | Target |
| ----- | ------ |
| Acknowledgement of your report | within 3 business days |
| Initial assessment and severity | within 7 business days |
| Fix released, or a plan with dates | within 30 days |

ferrisbar is maintained by one person, so these are honest targets rather than
a contractual SLA. If you have not heard back within the acknowledgement
window, please send a follow-up — it means the first message went astray.

## Supported versions

| Version | Supported |
| ------- | --------- |
| Latest release | ✅ |
| Older releases | ❌ — upgrade to the latest |

Fixes land on `main` and ship in the next release. There are no long-term
support branches.

## Disclosure

We follow coordinated disclosure. Once a fix is available we publish a
[GitHub Security Advisory](https://github.com/kerryhatcher/ferrisbar/security/advisories),
which also feeds the [RustSec](https://rustsec.org) advisory database, and
credit you by name unless you would rather stay anonymous. Please give us a
chance to ship the fix before disclosing publicly.

## Scope notes

ferrisbar reads JSON on stdin, reads files under `$CLAUDE_CONFIG_DIR`, writes
ANSI text to stdout, and — only when you run `ferrisbar setup` — edits a Claude
Code settings file. Findings that are in scope include anything that escapes
those boundaries, corrupts a settings file, or turns hostile stdin into
something worse than a wrong-looking statusline.

Advisories in third-party crates are tracked separately by
[`cargo audit`](https://github.com/rustsec/rustsec) and
[`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) in CI. If you spot
one we have missed, an ordinary issue is fine — those are already public.
