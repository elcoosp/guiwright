//! Performance budgets (architecture §15.1) — enforced by
//! `check_budgets` in CI (perf guards fail the build on p95 regressions,
//! §15.7).
//!
//! Canonical storage is `bench/budgets.toml` (§15.7: "budgets from §15.1
//! stored in bench/budgets.toml"); this module parses it and binds every
//! `m0_measurable` row to its sample function. Rows without a sample stay
//! design targets until their milestone binds them.
//!
//! CI headroom: raw-host budgets are 3× the design targets because CI
//! runners are noisy; the guard exists to catch *order-of-magnitude*
//! regressions, not to benchmark hardware (see `CI_HEADROOM`).

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

/// Budget table (in-crate copy of the canonical `bench/budgets.toml`,
/// shipped this way so the packaged/published crate compiles; equality
/// with the canonical file is enforced by [`tests::crate_copy_matches_canonical`]).
const BUDGETS_TOML: &str = include_str!("budgets.toml");

/// CI noise factor (documented, not hidden).
pub const CI_HEADROOM: u32 = 3;

/// One budget row of the §15.1 table (the subset that is measurable without
/// a GPU at M0).
#[derive(Clone, Debug)]
pub struct Budget {
    pub name: String,
    /// Design target p50 (§15.1 table).
    pub p50: Duration,
    /// Design target p95 (§15.1 table).
    pub p95: Duration,
    /// Measured sample function — the guard runs it N times and compares
    /// the median/p95 against `p50 × CI_HEADROOM` / `p95 × CI_HEADROOM`.
    pub sample: fn() -> Duration,
}

impl Budget {
    /// Run one measured sample.
    pub fn run_sample(&self) -> Duration {
        (self.sample)()
    }
}

/// A row of `bench/budgets.toml`.
#[derive(Debug, Deserialize)]
struct BudgetRow {
    id: String,
    name: String,
    p50_us: f64,
    p95_us: f64,
    #[allow(dead_code)]
    notes: Option<String>,
    m0_measurable: bool,
    #[allow(dead_code)]
    sample: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BudgetFile {
    #[allow(dead_code)]
    schema: String,
    budget: Vec<BudgetRow>,
}

/// Sample registry: `budgets.toml` names a sample, this maps the name to
/// the implementation. A measurable row without a registry entry is a
/// configuration error (caught by the test below and at load time).
type SampleFn = fn() -> Duration;

const SAMPLES: &[(&str, SampleFn)] = &[
    ("resolve_cached_index", crate::samples::resolve_cached_index),
    ("tree_diff_apply_1k", crate::samples::tree_diff_apply_1k),
    ("snapshot_serialize", crate::samples::snapshot_serialize),
    ("diff_blob_lz4", crate::samples::diff_blob_lz4),
    ("frame_digest", crate::samples::frame_digest),
    ("codec_roundtrip", crate::samples::codec_roundtrip),
    (
        "trace_write_amplification",
        crate::samples::trace_write_amplification,
    ),
    (
        "element_screenshot_crop",
        crate::samples::element_screenshot_crop,
    ),
];

fn parse_budgets() -> Vec<Budget> {
    let file: BudgetFile =
        toml::from_str(BUDGETS_TOML).expect("bench/budgets.toml must parse as gw-budgets/v1");
    file.budget
        .into_iter()
        .filter(|row| row.m0_measurable)
        .map(|row| {
            let sample_name = row.sample.as_deref().unwrap_or_else(|| {
                panic!("budget `{}` is m0_measurable but names no sample", row.id)
            });
            let sample = SAMPLES
                .iter()
                .find(|(name, _)| *name == sample_name)
                .unwrap_or_else(|| {
                    panic!("budget `{}` names unknown sample `{sample_name}`", row.id)
                })
                .1;
            Budget {
                name: row.name,
                p50: Duration::from_micros(row.p50_us.round() as u64),
                p95: Duration::from_micros(row.p95_us.round() as u64),
                sample,
            }
        })
        .collect()
}

/// The enforced (M0-measurable) subset of the §15.1 table.
pub fn budgets() -> &'static [Budget] {
    static BUDGETS: OnceLock<Vec<Budget>> = OnceLock::new();
    BUDGETS.get_or_init(parse_budgets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_measurable_row_binds_a_known_sample() {
        let budgets = budgets();
        let file: BudgetFile = toml::from_str(BUDGETS_TOML).expect("parse budgets.toml");
        let measurable = file.budget.iter().filter(|row| row.m0_measurable).count();
        assert_eq!(budgets.len(), measurable, "all measurable rows loaded");
        assert!(measurable >= 5, "the §15.1 core rows must stay enforced");
    }

    #[test]
    fn s15_1_core_rows_are_present() {
        let file: BudgetFile = toml::from_str(BUDGETS_TOML).expect("parse budgets.toml");
        for id in [
            "resolve_cached_index",
            "tree_diff_apply",
            "snapshot_serialize",
            "diff_blob_lz4",
        ] {
            assert!(
                file.budget.iter().any(|row| row.id == id),
                "missing §15.1 row `{id}`"
            );
        }
    }

    /// Repo-only test: the packaged crate ships its own `budgets.toml`
    /// copy (`include_str!` cannot escape the package root); the canonical
    /// file stays `bench/budgets.toml` (§15.7) and must never drift.
    #[test]
    fn crate_copy_matches_canonical() {
        let canonical = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../bench/budgets.toml"
        ))
        .expect("canonical bench/budgets.toml (present in the repository)");
        assert_eq!(BUDGETS_TOML, canonical, "budgets.toml copies drifted");
    }

    #[test]
    fn targets_survive_the_roundtrip() {
        let budgets = budgets();
        let resolve = budgets
            .iter()
            .find(|b| b.name.contains("Resolve locator, cached"))
            .expect("resolve row");
        assert_eq!(resolve.p50, Duration::from_micros(50));
        assert_eq!(resolve.p95, Duration::from_micros(300));
    }
}
