//! The m80 glue benchmark (§15.7): one full introspection pass over the
//! real gpui counter+form app (boot → pump → fold). The design-target
//! row rides the `gpui_introspection` entry (`m0_measurable = false`:
//! the glue workspace owns the sample, the main workspace carries no
//! gpui stack).

use std::time::Instant;

use guiwright_adapter_gpui_glue::harness::GpuiAdapter;

criterion::criterion_group! {
    name = benches;
    config = criterion::Criterion::default().sample_size(10);
    targets = gpui_introspection
}
criterion::criterion_main!(benches);

fn gpui_introspection(c: &mut criterion::Criterion) {
    c.bench_function("gpui_introspection", |b| {
        let mut adapter = GpuiAdapter::boot((480.0, 320.0), 0).expect("harness boots");
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = adapter.introspect_once();
            }
            start.elapsed()
        });
    });
}
