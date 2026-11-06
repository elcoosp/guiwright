//! Guiwright benchmark suites and perf-budget guards (architecture §15.7).
//!
//! * [`samples`] — the measured workloads (§15.1 rows, 1 000-node trees).
//! * [`budgets`] — the budget table + CI headroom.
//! * `benches/` — criterion suites (`cargo bench -p guiwright-bench`).
//! * `bin/check_budgets.rs` — the CI perf guard: budget regressions fail
//!   the build.

pub mod budgets;
pub mod samples;
