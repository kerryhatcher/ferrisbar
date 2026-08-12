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

// `main.rs`'s `run_report` calls both of these by name; nothing references
// `report::Options` outside this module, so only the two actually-used items
// are re-exported.
#[cfg(feature = "analytics")]
pub use report::{parse_args as parse_report_args, render};

// `cost.rs`'s transcript-walk hot loop calls this unconditionally (see the
// module doc comment above), so it's a real, always-live re-export.
#[cfg(feature = "analytics")]
pub use store::Sink;

// Only called from `store`'s own tests today. `cost.rs`'s `daily_chip` will
// call this unconditionally starting in Task 2, mirroring `Sink`'s rationale
// above — until then this re-export is unused-but-harmless outside tests.
#[cfg(feature = "analytics")]
#[allow(unused_imports)]
pub use store::today_repo_cost;

// The zero-cost no-op used in place of `store::Sink` when the `analytics`
// feature is off. `cost.rs` still constructs and calls it unconditionally
// (see the module doc comment above), so it is not dead code in a plain
// build either.
#[cfg(not(feature = "analytics"))]
pub struct Sink;

#[cfg(not(feature = "analytics"))]
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

// The zero-cost no-op used in place of `store::today_repo_cost` when the
// `analytics` feature is off. `cost.rs`'s `daily_chip` will call it
// unconditionally starting in Task 2; until then it's unused-but-harmless
// in a plain build too.
// Not `const`, despite doing nothing but returning `None`: this stub's
// signature must match the real (`feature = "analytics"`) `today_repo_cost`
// exactly, same rationale as `Sink::record`/`flush` above, and the real one
// can't be `const` (it opens a redb database).
#[cfg(not(feature = "analytics"))]
#[allow(dead_code, clippy::missing_const_for_fn)]
pub fn today_repo_cost(
    _enabled: bool,
    _data_dir: &std::path::Path,
    _repo_key: &str,
) -> Option<f64> {
    None
}
