//! §15.7 macro bench (m97): the scripted-session macro leg the micro
//! suites never measured — a scripted resolve+act action stream and the
//! 1 000-assert suite wall clock, end-to-end through the in-process door
//! on the **real egui adapter** (Level-1 a11y tree, §4.1), plus the one
//! full-session resident-set profile. The budgets these inform live in
//! `bench/budgets.toml` (`macro_egui_*`, design targets — `m0_measurable`
//! = false until the framework runtime matrix lands in the CI guard,
//! the `iced_tree_readback` row's rule).
//!
//! The **synthetic masonry app (1 k nodes)** half — the second surface
//! the §15.7 macro row names — rides the same shape on the real masonry
//! adapter (`macro_masonry_session`, `macro_masonry_*`): the fixture the
//! m97 bench leg's header called the honest gap (the adapter compiled
//! here but its fixture path needed the masonry widget crates the bench
//! crate did not carry — they are dev-deps now).
//!
//! ```text
//! cargo bench -p guiwright-bench --bench macro_session
//! ```

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use guiwright_adapter_egui::EguiAdapter;
use guiwright_adapter_masonry::MasonryAdapter;
use guiwright_driver::SeamRegistry;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{RequestV1, SelectorV1, SemActionV1, TextMatchV1};
use masonry::widgets::{Button, Flex, Label};
use masonry_core::core::{NewWidget, WidgetTag};

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

/// The assert target's wire shape (the m54 order: selector, expected
/// `TextMatchV1`, the timeout).
fn save_text() -> TextMatchV1 {
    TextMatchV1 {
        text: "Save".into(),
        exact: false,
        ignore_case: true,
    }
}

fn save_timeout() -> std::time::Duration {
    std::time::Duration::from_secs(2)
}

fn spawn_egui_door(
    rt: &tokio::runtime::Runtime,
) -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    // The door spawns its serve task on the runtime (tokio::spawn) — the
    // bench calls it from inside the runtime context it drives the client
    // with.
    rt.block_on(async {
        GwpServerHandle::spawn_inprocess(
            || {
                EguiAdapter::new(move |ctx: &egui::Context| {
                    ctx.root_ui(|ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            // The buttons exist for the drive to click —
                            // their responses are consumed by egui itself.
                            if ui.button("Save").clicked() {
                                // (no app state: the fixture is the bench target)
                            }
                            if ui.button("Cancel").clicked() {
                                // (same — the scripted click is the measurement)
                            }
                        });
                    });
                })
            },
            SeamRegistry::new(),
        )
        .expect("macro bench in-process server starts")
    })
}

/// The synthetic masonry app (§15.7 "a synthetic masonry app (1 k nodes)"):
/// a column of ~1 000 labels plus the one tagged `Save` button the
/// scripted drive resolves — the filler rows carry no `Save` name, so the
/// strict-mode resolve stays unique at 1 k nodes.
fn save_tag() -> WidgetTag<Button> {
    WidgetTag::new("save-button")
}

fn synthetic_app() -> NewWidget<Flex> {
    let mut flex = Flex::column().with_child(NewWidget::new(Label::new("Welcome, Ada")));
    for row in 0..999 {
        flex = flex.with_child(NewWidget::new(Label::new(format!("row {row}"))));
    }
    let flex = flex.with_child(NewWidget::new_with_tag(
        Button::with_text("Save"),
        save_tag(),
    ));
    NewWidget::new(flex)
}

fn spawn_masonry_door(
    rt: &tokio::runtime::Runtime,
) -> (GwpServerHandle, guiwright_proto::duplex::DuplexConnect) {
    // The egui leg's rule: the door spawns on the runtime's context.
    rt.block_on(async {
        GwpServerHandle::spawn_inprocess(
            || MasonryAdapter::<Flex>::new_with_size(synthetic_app(), 400.0, 300.0),
            SeamRegistry::new(),
        )
        .expect("macro bench masonry door starts")
    })
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
        .map_or(0, |pages| pages * 4)
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
            });
        });
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
                        .expect_text(save_selector(), save_text(), save_timeout())
                        .await
                        .expect("macro assert");
                }
            });
        });
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
        client.bye().await.expect("profile close");
    });
    h2.shutdown();
    let after = rss_kb();
    println!(
        "macro_egui_session memory profile: resident-set delta {} KiB across one full session (spawn → 10 actions → close)",
        after.saturating_sub(before)
    );

    handle.shutdown();
}

/// The §15.7 masonry leg: the same scripted session shape on the real
/// masonry adapter over the synthetic 1 k-node app — the per-action
/// histogram, the 1 000-assert suite wall clock and the memory profile,
/// each the masonry surface of the row the egui leg already measured.
fn bench_masonry_macro(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("macro masonry bench runtime");
    let (handle, connect) = spawn_masonry_door(&rt);
    let mut client = rt.block_on(connected(&connect));

    let mut group = c.benchmark_group("macro_masonry_session");

    // The scripted 200-action session, per-action latency — the
    // resolve's a11y walk at 1 k nodes + the act rendezvous pump.
    group.throughput(Throughput::Elements(200));
    group.bench_function("actions_200_scripted", |b| {
        b.iter(|| {
            rt.block_on(async {
                for _ in 0..200 {
                    let node = client
                        .resolve(save_selector(), 2000)
                        .await
                        .expect("macro masonry resolve");
                    client
                        .action(node.node, node.generation, SemActionV1::Click, false, 2000)
                        .await
                        .expect("macro masonry click");
                }
            });
        });
    });

    // The suite wall clock for 1 000 asserts (§15.7) on the synthetic
    // app — the m54 wake loop over the M39 describe-all round trip.
    group.throughput(Throughput::Elements(1000));
    group.bench_function("asserts_1000_suite", |b| {
        b.iter(|| {
            rt.block_on(async {
                for _ in 0..1000 {
                    client
                        .expect_text(save_selector(), save_text(), save_timeout())
                        .await
                        .expect("macro masonry assert");
                }
            });
        });
    });
    group.finish();

    // The memory profile (§15.7), masonry surface: one full session's
    // resident-set delta, spawn → 10 actions → close.
    let before = rss_kb();
    let (h2, c2) = spawn_masonry_door(&rt);
    rt.block_on(async {
        let mut client = connected(&c2).await;
        for _ in 0..10 {
            let node = client
                .resolve(save_selector(), 2000)
                .await
                .expect("profile masonry resolve");
            client
                .action(node.node, node.generation, SemActionV1::Click, false, 2000)
                .await
                .expect("profile masonry click");
        }
        let _ = client
            .request(RequestV1::Ping)
            .await
            .expect("profile masonry ping");
        client.bye().await.expect("profile masonry close");
    });
    h2.shutdown();
    let after = rss_kb();
    println!(
        "macro_masonry_session memory profile: resident-set delta {} KiB across one full session (spawn → 10 actions → close)",
        after.saturating_sub(before)
    );

    handle.shutdown();
}

criterion_group!(benches, bench_macro, bench_masonry_macro);
criterion_main!(benches);
