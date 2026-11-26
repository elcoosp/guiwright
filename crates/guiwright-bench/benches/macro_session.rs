//! §15.7 macro bench (m97): the scripted-session macro leg the micro
//! suites never measured — a scripted resolve+act action stream and the
//! 1 000-assert suite wall clock, end-to-end through the in-process door
//! on the **real egui adapter** (Level-1 a11y tree, §4.1), plus the one
//! full-session resident-set profile. The budgets these inform live in
//! `bench/budgets.toml` (`macro_egui_*`, design targets — m0_measurable
//! = false until the framework runtime matrix lands in the CI guard,
//! the `iced_tree_readback` row's rule). The masonry leg stays the
//! honest gap (the m97 record's list): the adapter compiles here but
//! its fixture path needed masonry-core widget re-exports the bench
//! crate does not carry.
//!
//! ```text
//! cargo bench -p guiwright-bench --bench macro_session
//! ```

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use guiwright_adapter_egui::EguiAdapter;
use guiwright_driver::SeamRegistry;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{NodeKeyV1, RequestV1, SelectorV1, SemActionV1, TextMatchV1};
use std::time::Instant;

fn save_selector() -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: Some(TextMatchV1 {
            text: "Save".into(),
            exact: false,
            ignore_case: true,
        }),
    }
}

fn spawn_egui_door(
    rt: &tokio::runtime::Runtime,
) -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || {
            EguiAdapter::new(move |ctx: &egui::Context| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if ui.button("Save").clicked() {}
                    if ui.button("Cancel").clicked() {}
                });
            })
        },
        SeamRegistry::new(),
    )
    .expect("macro bench in-process server starts");
    let _ = rt;
    (handle, connect)
}

async fn connected(connect: &guiwright_proto::duplex::DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("macro bench client");
    client
        .handshake("gw-macro-bench/0.1")
        .await
        .expect("handshake");
    client
}

/// Resident set (KiB) from /proc/self/statm — the safe memory profile
/// (the workspace forbids `unsafe`, so no counting global allocator).
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages * 4)
        .unwrap_or(0)
}

fn bench_macro(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("macro bench runtime");
    let (handle, connect) = spawn_egui_door(&rt);
    let mut client = rt.block_on(connected(&connect));

    let mut group = c.benchmark_group("macro_egui_session");

    // The scripted 200-action session, per-action latency (§15.7 "the
    // end-to-end action latency histogram" — criterion's samples ARE the
    // histogram, 200 actions per iteration).
    group.throughput(Throughput::Elements(200));
    group.bench_function("actions_200_scripted", |b| {
        b.iter(|| {
            rt.block_on(async {
                for _ in 0..200 {
                    let node = client
                        .resolve(save_selector(), 2000)
                        .await
                        .expect("macro resolve");
                    client
                        .action(node.node, node.generation, SemActionV1::Click, false, 2000)
                        .await
                        .expect("macro click");
                }
            })
        })
    });

    // The suite wall clock for 1 000 asserts (§15.7): one iteration IS
    // the suite — the per-assert composition rides the m54 wake loop
    // over the M39 describe-all round trip.
    group.throughput(Throughput::Elements(1000));
    group.bench_function("asserts_1000_suite", |b| {
        b.iter(|| {
            rt.block_on(async {
                for _ in 0..1000 {
                    client
                        .expect_text(save_selector(), "Save", 2000)
                        .await
                        .expect("macro assert");
                }
            })
        })
    });
    group.finish();

    // The memory profile (§15.7): one full session's resident-set delta —
    // spawn, handshake, a short scripted drive, shutdown; the allocator's
    // retention stays in the delta (the honest whole-process number).
    let before = rss_kb();
    let (h2, c2) = spawn_egui_door(&rt);
    rt.block_on(async {
        let mut client = connected(&c2).await;
        for _ in 0..10 {
            let node = client
                .resolve(save_selector(), 2000)
                .await
                .expect("profile resolve");
            client
                .action(node.node, node.generation, SemActionV1::Click, false, 2000)
                .await
                .expect("profile click");
        }
        let _ = client.request(RequestV1::Ping).await.expect("profile ping");
        let _ = NodeKeyV1::AccessKit {
            tree: [0u8; 16],
            node: 0,
        };
        client.close().await.expect("profile close");
    });
    h2.shutdown();
    let after = rss_kb();
    println!(
        "macro_egui_session memory profile: resident-set delta {} KiB across one full session (spawn → 10 actions → close)",
        after.saturating_sub(before)
    );

    handle.shutdown();
}

criterion_group!(benches, bench_macro);
criterion_main!(benches);
