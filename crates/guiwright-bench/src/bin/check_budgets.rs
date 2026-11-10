//! Budget guard (§15.7 "perf guards in CI: budget regressions fail the
//! build"). Runs every §15.1 sample, compares against the budget with the
//! documented CI headroom, and prints a table.
//!
//! Run standalone: `cargo run --release -p guiwright-bench --bin check_budgets`
//! (invoked by the `bench-guard` CI job). The `--release` profile is
//! mandatory: budgets describe optimized behavior, and a dev-profile run
//! reports false regressions (frame digest is ~1000× slower unoptimized).

use std::time::Duration;

use guiwright_bench::budgets::{Budget, CI_HEADROOM, budgets};

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort_unstable();
    let idx = usize::try_from(((samples.len() as f64 - 1.0) * p).round() as i64)
        .unwrap_or(samples.len().saturating_sub(1));
    samples.get(idx).copied().unwrap_or(Duration::ZERO)
}

fn measure(budget: &Budget, iterations: usize) -> (Duration, Duration) {
    let mut samples = Vec::with_capacity(iterations);
    // Warm-up pass (JIT/page-cache effects).
    for _ in 0..3 {
        budget.run_sample();
    }
    for _ in 0..iterations {
        samples.push(budget.run_sample());
    }
    let p50 = percentile(&mut samples, 0.50);
    let p95 = percentile(&mut samples, 0.95);
    (p50, p95)
}

fn main() {
    let iterations: usize = std::env::var("GUIWRIGHT_BENCH_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);

    println!(
        "{:<40} {:>12} {:>12} {:>14} {:>14} {:>8}",
        "budget", "p50", "limit50", "p95", "limit95", "verdict"
    );
    let mut failures = Vec::new();
    for budget in budgets() {
        let (p50, p95) = measure(budget, iterations);
        let limit50 = budget.p50 * CI_HEADROOM;
        let limit95 = budget.p95 * CI_HEADROOM;
        let ok = p50 <= limit50 && p95 <= limit95;
        if !ok {
            failures.push(budget.name.clone());
        }
        println!(
            "{:<40} {:>12?} {:>12?} {:>14?} {:>14?} {:>8}",
            budget.name,
            p50,
            limit50,
            p95,
            limit95,
            if ok { "OK" } else { "REGRESSED" }
        );
    }
    if !failures.is_empty() {
        eprintln!(
            "\n{} budget(s) regressed: {failures:?} (headroom ×{CI_HEADROOM}, {iterations} iterations)",
            failures.len()
        );
        std::process::exit(1);
    }
    println!("\nall budgets OK (headroom ×{CI_HEADROOM}, {iterations} iterations)");
}
