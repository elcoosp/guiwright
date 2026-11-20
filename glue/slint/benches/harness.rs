//! The m78 slint harness bench (architecture §15.7, design-target row
//! `slint_introspection`): one full introspection pass on the real
//! `.slint` app — the `ElementHandle` walk (`visit_descendants` +
//! bounds-containment nesting) feeding the engine's
//! [`guiwright_adapter_slint::SlintTreeBuilder::rebuild`] fold, the cycle
//! the adapter's `logical_snapshot` rides per frame.
//!
//! Gated nowhere: the whole crate is the glue workspace (the skia-links
//! record scopes the framework stack here), so the bench always builds.
//! The budget row stays a design target (`m0_measurable = false`) — it is
//! measured by this criterion suite, not by the main workspace's
//! `check_budgets` samples.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use guiwright_adapter_slint_glue::harness::SlintAdapter;
use guiwright_driver::FrameworkAdapter;
use slint::include_modules;

include_modules!();

/// One measurement window: `iters` full logical-snapshot cycles (the
/// introspection walk + engine fold) timed wall-clock.
fn measure_passes(adapter: &mut SlintAdapter<CounterForm>, iters: u64) -> Duration {
    let start = Instant::now();
    for _ in 0..iters {
        assert!(adapter.logical_snapshot().is_some(), "diff per pass");
    }
    start.elapsed()
}

/// One logical-snapshot cycle: introspection walk + engine fold.
fn bench_introspection_pass(c: &mut Criterion) {
    let mut adapter =
        SlintAdapter::boot(CounterForm::new, (480.0, 320.0)).expect("slint testing harness boots");
    let mut group = c.benchmark_group("slint_harness");
    group.bench_function("introspection_pass", |b| {
        b.iter_custom(|iters| measure_passes(&mut adapter, iters));
    });
    group.finish();
}

criterion_group!(benches, bench_introspection_pass);
criterion_main!(benches);
