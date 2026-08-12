//! Persistent per-repo, per-day, per-model cost history, feeding
//! `ferrisbar report`. Piggybacks on `cost::refresh_daily_cache`'s existing
//! transcript walk rather than adding a second one. See
//! docs/superpowers/specs/2026-08-11-repo-cost-analytics-design.md.
//!
//! `Sink` exists in every build, feature or not: `cost.rs` calls it
//! unconditionally so its hot loop never needs an `#[cfg]` of its own. When
//! the `analytics` feature is off, `Sink` below is a zero-cost no-op; the
//! real implementation lives in `store.rs`, compiled only with the feature.

#[cfg(feature = "analytics")]
pub mod store;

#[cfg(feature = "analytics")]
mod report;

// `render`/`parse_report_args`/`ReportOptions` and everything they touch in
// `report` are unreachable from `main` until Task 8 wires this re-export
// into the CLI's `report` subcommand, so a `--features analytics` build
// without that wiring yet flags the re-export (and, transitively, `report`'s
// own items) as dead. Task 7 only builds the report engine; Task 8 is what
// makes it live.
#[cfg(feature = "analytics")]
#[allow(unused_imports)]
pub use report::{parse_args as parse_report_args, render, Options as ReportOptions};

// `Sink` and everything it touches in `store` are unreachable from `main`
// until Task 6 wires this re-export into `cost.rs`'s transcript-walk hot
// loop, so a `--features analytics` build without that wiring yet flags the
// re-export (and, transitively, `store`'s own items) as dead. Task 5 only
// builds the store; Task 6 is what makes it live.
#[cfg(feature = "analytics")]
#[allow(unused_imports)]
pub use store::Sink;

// Unreachable until Task 6 wires a call from `cost.rs`'s hot loop, so a
// plain (no `analytics` feature) build flags this stub as dead code today.
// It stays defined now, ahead of that caller, because its signature must be
// pinned to match the real `Sink` in `store.rs` exactly — see the module
// doc comment above.
#[cfg(not(feature = "analytics"))]
#[allow(dead_code)]
pub struct Sink;

#[cfg(not(feature = "analytics"))]
#[allow(dead_code)]
impl Sink {
    pub fn new(_enabled: bool, _today: String, _yesterday: String) -> Self {
        Self
    }

    // `&mut self`, non-`const`, and ignoring both arguments are all
    // deliberate here: this no-op stub's signature must match the real
    // (`feature = "analytics"`) `Sink::record` exactly, since Task 6 calls
    // both variants identically from the same call site.
    #[allow(
        clippy::unused_self,
        clippy::missing_const_for_fn,
        clippy::needless_pass_by_ref_mut
    )]
    pub fn record(&mut self, _rec: &crate::cost::ParsedRecord, _cost: f64) {}

    // Same rationale as `record` above, minus the `&mut` lint (this one
    // takes `self` by value).
    #[allow(clippy::unused_self, clippy::missing_const_for_fn)]
    pub fn flush(self, _data_dir: &std::path::Path) {}
}
