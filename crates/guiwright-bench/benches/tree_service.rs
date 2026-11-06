//! Tree-service benches (§15.7 micro suite over §15.1 rows).

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use accesskit::Role;
use guiwright_bench::samples;
use guiwright_core::selector::Selector;
use guiwright_core::tree::TreeService;

fn bench_resolve_cached_index(c: &mut Criterion) {
    let svc: std::sync::OnceLock<TreeService> = std::sync::OnceLock::new();
    let svc = svc.get_or_init(samples::thousand_node_tree);
    let mut group = c.benchmark_group("tree_service");
    group.throughput(criterion::Throughput::Elements(1));
    group.bench_function("resolve_cached_index_1k_nodes", |b| {
        b.iter(|| {
            let resolved = svc
                .resolve(&Selector::role_named(Role::Button, "Export 500"))
                .expect("unique node");
            black_box(resolved);
        });
    });
    group.bench_function("resolve_text_scan_1k_nodes", |b| {
        b.iter(|| {
            let matches = svc.resolve_all(&Selector::text("Export 99")).expect("scan");
            black_box(matches.len());
        });
    });
    group.finish();
}

fn bench_diff_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_service");
    group.bench_function("diff_apply_single_node_update", |b| {
        b.iter_with_large_drop(|| {
            let mut svc = samples::thousand_node_tree();
            let mut renamed = accesskit::Node::new(Role::Button);
            renamed.set_label("Export 500 (renamed)");
            renamed.set_bounds(accesskit::Rect {
                x0: 40.0,
                y0: 1000.0,
                x1: 78.0,
                y1: 1018.0,
            });
            let update = accesskit::TreeUpdate {
                nodes: vec![(accesskit::NodeId(502), renamed)],
                tree: None,
                tree_id: accesskit::TreeId::ROOT,
                focus: accesskit::NodeId(0),
            };
            black_box(svc.apply_update(update));
        });
    });
    group.finish();
}

fn bench_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_service");
    group.bench_function("snapshot_serialize_postcard_1k_nodes", |b| {
        b.iter(|| {
            black_box(samples::snapshot_serialize());
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_resolve_cached_index,
    bench_diff_apply,
    bench_snapshot
);
criterion_main!(benches);
