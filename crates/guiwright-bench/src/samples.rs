//! Sample workloads behind the §15.1 budgets: each returns the duration of
//! ONE measured operation (the caller loops and aggregates).

use std::time::{Duration, Instant};

use accesskit::Role;
use guiwright_core::selector::Selector;
use guiwright_core::tree::TreeService;
use guiwright_proto::schema::{GwpRequest, RequestV1};

/// A synthetic 1 000-node tree (§15.1's "1k nodes" scale), built from ONE
/// full update (window → group → 998 labeled buttons).
pub fn thousand_node_tree() -> TreeService {
    let mut svc = TreeService::new();
    svc.apply_update(full_thousand_update());
    svc
}

pub fn full_thousand_update() -> accesskit::TreeUpdate {
    const BUTTONS: u32 = 998;
    let mut window = accesskit::Node::new(Role::Window);
    window.set_label("Bench App");
    window.set_children([accesskit::NodeId(1)]);
    let mut group = accesskit::Node::new(Role::Group);
    group.set_children(
        (0..BUTTONS)
            .map(|i| accesskit::NodeId(2 + u64::from(i)))
            .collect::<Vec<_>>(),
    );
    let mut nodes = vec![
        (accesskit::NodeId(0), window),
        (accesskit::NodeId(1), group),
    ];
    for i in 0..BUTTONS {
        let mut btn = accesskit::Node::new(Role::Button);
        btn.set_label(format!("Export {i}"));
        btn.set_bounds(accesskit::Rect {
            x0: f64::from(((i % 20) * 40) as i16),
            y0: f64::from(((i / 20) * 20) as i16),
            x1: f64::from((((i % 20) * 40) + 38) as i16),
            y1: f64::from((((i / 20) * 20) + 18) as i16),
        });
        nodes.push((accesskit::NodeId(2 + u64::from(i)), btn));
    }
    accesskit::TreeUpdate {
        nodes,
        tree: Some(accesskit::TreeInfo {
            root: accesskit::NodeId(0),
            toolkit_name: Some("guiwright-bench".to_string()),
            toolkit_version: None,
        }),
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    }
}

thread_local! {
    static BENCH_TREE: TreeService = thousand_node_tree();
}

/// §15.1 "Resolve locator, cached index, embedded" — role+name hash hit
/// (exact-ignoring-case matches resolve through the `(role, name)` hash
/// index, §8.2 step 1). The 1k tree is built once per thread; only the
/// resolve is measured. Fuzzy (substring) matches scan the role bucket —
/// that is the `resolve_text_scan` bench, not this row.
pub fn resolve_cached_index() -> Duration {
    BENCH_TREE.with(|svc| {
        let selector = Selector::role_named(
            Role::Button,
            guiwright_core::TextMatch::exact_ignoring_case("Export 500"),
        );
        let start = Instant::now();
        let _ = svc.resolve(&selector).expect("unique Export 500");
        start.elapsed()
    })
}

/// §15.1 "Tree diff apply (1k nodes)" — consumer `ChangeHandler` fan-out.
/// §15.1 "Tree diff apply (1k nodes)" — measures only the apply; setup is
/// excluded by measuring `Instant::now()` after the tree is built.
pub fn tree_diff_apply_1k() -> Duration {
    let mut svc = thousand_node_tree();
    let update = rename_export_500_update();
    let start = Instant::now();
    svc.apply_update(update);
    start.elapsed()
}

/// The smallest possible diff (typ. < 2 % nodes changed, §15.2).
pub fn rename_export_500_update() -> accesskit::TreeUpdate {
    let mut renamed = accesskit::Node::new(Role::Button);
    renamed.set_label("Export 500 (renamed)");
    renamed.set_bounds(accesskit::Rect {
        x0: 40.0,
        y0: 1000.0,
        x1: 78.0,
        y1: 1018.0,
    });
    accesskit::TreeUpdate {
        nodes: vec![(accesskit::NodeId(502), renamed)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    }
}

/// §15.1 "Full tree snapshot serialize (postcard)" — 1k-node snapshot.
pub fn snapshot_serialize() -> Duration {
    BENCH_TREE.with(|svc| {
        let keys: Vec<guiwright_core::NodeKey> = svc.all_nodes().to_vec();
        let nodes: Vec<SnapshotNode> = keys
            .iter()
            .filter_map(|key| {
                let node = svc.node(key)?;
                Some(SnapshotNode {
                    id: match key {
                        guiwright_core::NodeKey::AccessKit { node, .. } => node.0,
                        guiwright_core::NodeKey::Logical { .. } => 0,
                    },
                    role: format!("{:?}", node.role()),
                    label: node.label(),
                    value: node.value(),
                    bounds: node.bounding_box().map(|r| [r.x0, r.y0, r.x1, r.y1]),
                })
            })
            .collect();
        let start = Instant::now();
        let bytes = postcard::to_allocvec(&nodes).expect("snapshot encodes");
        let _ = bytes.len();
        start.elapsed()
    })
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SnapshotNode {
    id: u64,
    role: String,
    label: Option<String>,
    value: Option<String>,
    bounds: Option<[f64; 4]>,
}

/// §15.1 "`TreeChanged` diff blob (LZ4)" — compress a diff payload.
pub fn diff_blob_lz4() -> Duration {
    BENCH_TREE.with(|svc| {
        // Serialize the raw node ids of the current tree (the diff payload shape).
        let ids: Vec<u64> = svc
            .all_nodes()
            .iter()
            .map(|key| match key {
                guiwright_core::NodeKey::AccessKit { node, .. } => node.0,
                guiwright_core::NodeKey::Logical { id, .. } => *id,
            })
            .collect();
        let payload = postcard::to_allocvec(&ids).expect("diff payload encodes");
        let start = Instant::now();
        let blob = guiwright_proto::blob::encode_blob(&payload);
        let _ = guiwright_proto::blob::decode_blob(&blob).expect("roundtrip");
        start.elapsed()
    })
}

/// §15.1 frame hash (§10): digest one contiguous pixel/vertex buffer
/// (~160 KB ≈ a strided sample of a dense 1280×800 pass). The adapter
/// hashes buffers wholesale, so the sample does too.
pub fn frame_digest() -> Duration {
    use std::hash::Hasher;
    // Setup outside the timed region: 10k vertices × 4 f32 = 160 KB.
    let mut buffer = Vec::with_capacity(10_000 * 16);
    for i in 0..10_000u32 {
        buffer.extend_from_slice(&((i % 1280) as f32).to_bits().to_le_bytes());
        buffer.extend_from_slice(&((i % 800) as f32).to_bits().to_le_bytes());
        buffer.extend_from_slice(&((i % 64) as f32).to_bits().to_le_bytes());
        buffer.extend_from_slice(&((i % 32) as f32).to_bits().to_le_bytes());
    }
    let start = Instant::now();
    let mut hasher = twox_hash::xxhash3_64::Hasher::new();
    hasher.write(&buffer);
    let _hash = hasher.finish();
    start.elapsed()
}

/// §15.1 codec round-trip: postcard encode+decode of a GWP request.
pub fn codec_roundtrip() -> Duration {
    let request = GwpRequest::V1(RequestV1::Hello {
        proto_version: 1,
        agent: "bench-client".into(),
    });
    let start = Instant::now();
    let bytes = postcard::to_allocvec(&request).expect("encode");
    let _back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
    start.elapsed()
}

// --- M1 samples (trace writer off critical path, §12; CPU crop, §10) -------

/// §15.1 `trace_write_amplification`: the UI-thread cost of recording ONE
/// frame's worth of trace records (1 event + 1 dedup-checked 4 KiB frame +
/// 1 tree diff) through the bounded ring. The writer thread is off the
/// critical path — this is the pure `try_send` cost the app feels.
pub fn trace_write_amplification() -> Duration {
    const FRAMES: u64 = 256;
    use guiwright_trace::schema::AdapterRecord;
    use guiwright_trace::{TraceOptions, TraceRecorder};
    use std::time::Instant;

    let dir = std::env::temp_dir().join(format!("gw-bench-trace-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let options = TraceOptions {
        output_path: dir.join("amplification.zip"),
        test: "bench".into(),
        app: "bench".into(),
        adapter: AdapterRecord {
            name: "bench".into(),
            framework: "fixture".into(),
            framework_version: "0".into(),
        },
        seed: None,
        resolution: (1280, 800),
        scale: 1.0,
        frame_budget_bytes: 64 * 1024 * 1024,
        ring_capacity: 8192,
    };
    let mut recorder = TraceRecorder::start(options);
    let png = vec![7u8; 4096];

    let started = Instant::now();
    for i in 0..FRAMES {
        recorder.event(i, "action", "click @ (12, 8)");
        recorder.frame(i, 0x1000 + i, png.clone());
        recorder.tree_diff(i, vec![i], vec![], vec![], false);
    }
    let per_frame = started.elapsed() / u32::try_from(FRAMES).unwrap_or(u32::MAX);
    let _ = recorder.stop();
    let _ = std::fs::remove_dir_all(&dir);
    per_frame
}

/// §15.1 `element_screenshot_crop`: crop a 240×160 element rect from a
/// 1280×800 frame buffer (CPU path — no re-render, no GPU, §10).
pub fn element_screenshot_crop() -> Duration {
    use guiwright_shot::Image;
    use std::time::Instant;

    let frame = Image::solid(1280, 800, [40, 80, 120, 255]);
    let started = Instant::now();
    let region = guiwright_shot::crop(&frame, 100, 80, 340, 240).expect("crop");
    let _ = (region.width, region.height);
    started.elapsed()
}

// --- M2 samples (remote UDS round-trip, §5.3/§15.1; viewer replay, §12) ------

/// §15.1 `resolve_remote_uds`: one Resolve round-trip over a real UDS
/// socket — the §15.6 IPC shape (postcard framing, session-thread
/// rendezvous included). The server is spawned once per sample with the
/// mock adapter; the measured operation is the client call only.
pub fn resolve_remote_uds() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::schema::{RequestV1, ResponseV1, SelectorV1, TextMatchV1};
    use guiwright_proto::uds::Client;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let fixture = Fixture::buttons();
    let runtime = Runtime::new().expect("bench runtime");
    let dir = std::env::temp_dir().join(format!("gw-bench-uds-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("resolve.sock");

    // The server spawns tokio tasks: it must come up inside the runtime
    // context (the handle escapes block_on for shutdown).
    let (measured, handle) = runtime.block_on(async {
        let handle = GwpServerHandle::spawn_with_seams(
            path.clone(),
            move || MockAdapter::with_fixture(&fixture),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench server binds");
        let mut client = Client::connect(&path).await.expect("bench client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let selector = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };

        // Warm the index once (the §15.1 row is the cached-index path).
        for _ in 0..4 {
            let resp = client
                .request(RequestV1::Resolve {
                    selector: selector.clone(),
                    timeout_ms: 1000,
                })
                .await
                .expect("warm resolve");
            assert!(matches!(resp, ResponseV1::Resolved { .. }));
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let resp = client
                .request(RequestV1::Resolve {
                    selector: selector.clone(),
                    timeout_ms: 1000,
                })
                .await
                .expect("bench resolve");
            assert!(matches!(resp, ResponseV1::Resolved { .. }));
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
    measured.max(Duration::from_nanos(1))
}

/// §15.7 micro-suite guard (M2): open a synthetic archive and resolve the
/// tree at the last frame — the viewer's timeline replay path (§12).
pub fn viewer_timeline_replay() -> Duration {
    use guiwright_trace::schema::{AdapterRecord, TreeNode, TreeSnapshot};
    use guiwright_trace::{TraceOptions, TraceRecorder};
    use guiwright_viewer::Timeline;
    use std::time::Instant;

    let dir = std::env::temp_dir().join(format!("gw-bench-viewer-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("replay.zip");

    let options = TraceOptions {
        output_path: path.clone(),
        test: "bench".into(),
        app: "bench".into(),
        adapter: AdapterRecord {
            name: "bench".into(),
            framework: "fixture".into(),
            framework_version: "0".into(),
        },
        seed: None,
        resolution: (1280, 800),
        scale: 1.0,
        frame_budget_bytes: 64 * 1024 * 1024,
        ring_capacity: 8192,
    };
    let mut recorder = TraceRecorder::start(options);
    // ~1 000-node tree snapshots at 3 frames + diffs between.
    let nodes: Vec<TreeNode> = (0..1000)
        .map(|i| TreeNode {
            id: i,
            parent: Some(0),
            role: "Button".into(),
            label: Some(format!("button-{i}")),
            value: None,
            bounds: Some([0.0, 0.0, 40.0, 20.0]),
        })
        .collect();
    for frame in [1u64, 30, 60] {
        recorder.tree_snapshot(TreeSnapshot {
            frame_seq: frame,
            nodes: nodes.clone(),
            focus: Some(0),
        });
        recorder.tree_diff(frame + 1, vec![], vec![frame], vec![], false);
    }
    recorder.stop().expect("bench archive");

    let started = Instant::now();
    let archive = guiwright_viewer::TraceArchive::open(&path).expect("open");
    let timeline = Timeline::from_archive(&archive);
    let view = timeline
        .tree_at(60)
        .expect("resolved view at the last frame");
    assert_eq!(view.nodes.len(), 1000);
    started.elapsed()
}

// --- M3 samples (remote TCP round-trip §7/§15.1; video dedup §10) -----------

/// §15.1 `resolve_remote_tcp`: one Resolve round-trip over loopback TCP —
/// the M3 transport other-language clients drive (§7 "TCP for CI
/// containers"; byte-identical to the UDS row, only the socket differs).
pub fn resolve_remote_tcp() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::schema::{RequestV1, ResponseV1, SelectorV1, TextMatchV1};
    use guiwright_proto::tcp::Client;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let fixture = Fixture::buttons();
    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let handle = GwpServerHandle::spawn_tcp(
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            move || MockAdapter::with_fixture(&fixture),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench tcp server binds");
        let addr = match handle.endpoint() {
            guiwright_driver::server::Endpoint::Tcp(addr) => *addr,
            guiwright_driver::server::Endpoint::Uds(_)
            | guiwright_driver::server::Endpoint::Ws(_) => {
                unreachable!("spawn_tcp reports a TCP endpoint")
            }
        };
        let mut client = Client::connect(&addr.to_string())
            .await
            .expect("bench client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let selector = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };

        for _ in 0..4 {
            let resp = client
                .request(RequestV1::Resolve {
                    selector: selector.clone(),
                    timeout_ms: 1000,
                })
                .await
                .expect("warm resolve");
            assert!(matches!(resp, ResponseV1::Resolved { .. }));
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let resp = client
                .request(RequestV1::Resolve {
                    selector: selector.clone(),
                    timeout_ms: 1000,
                })
                .await
                .expect("bench resolve");
            assert!(matches!(resp, ResponseV1::Resolved { .. }));
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- M15 sample (WebSocket ping round-trip, §7's third door) -----------------

/// §15.1 `ws_ping_roundtrip`: one Ping round-trip over the WebSocket
/// transport (M15) — the viewer live-mode door's steady-state latency.
/// Byte-identical to the TCP row; the RFC 6455 upgrade is paid once at
/// connect, outside the measured window.
pub fn ws_ping_roundtrip() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::WsClient;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let handle = GwpServerHandle::spawn_ws(
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench ws server binds");
        let addr = match handle.endpoint() {
            guiwright_driver::server::Endpoint::Ws(addr) => *addr,
            other => unreachable!("spawn_ws reports a WS endpoint, got {other:?}"),
        };
        let mut client = WsClient::connect(&addr.to_string())
            .await
            .expect("bench ws client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        for _ in 0..4 {
            client.ping().await.expect("warm ping");
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            client.ping().await.expect("bench ping");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

/// §15.7 micro-suite guard (M3): the §10 video backpressure hot path —
/// hash-equality dedup + bounded ring push per presented frame. The
/// measured window is the `push_frame` loop only (encoding runs on the
/// writer thread, off the session path, §15.4).
pub fn video_frame_dedup() -> Duration {
    const FRAMES: u64 = 512;
    use guiwright_trace::video::{EncoderChoice, VideoOptions, VideoRecorder};
    use std::time::Instant;

    let dir = std::env::temp_dir().join(format!("gw-bench-video-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let mut options = VideoOptions::new(25);
    options.encoder = EncoderChoice::PngSequence;
    let mut recorder =
        VideoRecorder::start(dir.join("bench.frames"), &options).expect("bench recorder");

    let frame: std::sync::Arc<[u8]> =
        std::sync::Arc::from(vec![7u8; 64 * 64 * 4].into_boxed_slice());
    // Half the frames repeat the previous hash (a static UI compresses to
    // keyframe + no-ops, §10); the others change.
    let started = Instant::now();
    for i in 0..FRAMES {
        let hash = if i % 2 == 0 { i } else { i - 1 };
        recorder.push_frame(i + 1, hash, 64, 64, Some(&frame));
    }
    let measured = started.elapsed() / u32::try_from(FRAMES).expect("frames fit u32");

    recorder.stop().expect("bench recording finalized");
    let _ = std::fs::remove_dir_all(&dir);
    measured.max(Duration::from_nanos(1))
}

// --- M7 sample (gesture synthesis hot path, §9) -----------------------------

/// §9 gesture synthesis: build the raw event batch for one 200 px drag
/// (down + 40 ratchet-interpolated moves + up at the default 5 px/tick)
/// and one double-click. Pure CPU on the session path — the pacing
/// ratchet runs per gesture request, so the batch build must stay
/// trivially cheap next to the frame rendezvous it feeds (§15.4).
pub fn gesture_synthesis() -> Duration {
    use guiwright_driver::{Gesture, GesturePacing, PointerButton, synthesize};
    let pacing = GesturePacing::default();
    let drag = Gesture::Drag {
        from_x: 10.0,
        from_y: 10.0,
        to_x: 210.0,
        to_y: 10.0,
        button: PointerButton::Primary,
    };
    let click = Gesture::Click {
        x: 12.5,
        y: 30.0,
        button: PointerButton::Primary,
        count: 2,
    };
    let start = Instant::now();
    let drag_events = synthesize(&drag, &pacing);
    let click_events = synthesize(&click, &pacing);
    let elapsed = start.elapsed();
    assert_eq!(drag_events.len(), 42, "down + 40 ratchet moves + up");
    assert_eq!(click_events.len(), 5, "move + down/up ×2");
    elapsed
}

// --- Kit embedded-input sample (§9 over the kit) -----------------------------

/// §9 input pipeline, kit side: one `app.mouse().click(x, y)` through the
/// embedded surface — handle → session gesture synthesis (the §9 recipes,
/// adapter pacing) → per-window injection → rendezvous pump. The launch
/// and settle stay outside the measured span; the measured operation is
/// one click, the hot path every embedded input command rides.
pub fn kit_input_click() -> Duration {
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("kit_input_click", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    let outcome = app.mouse().click(10.0, 20.0).expect("bench click applies");
    let elapsed = start.elapsed();
    assert_eq!(outcome.events, 3, "move + down + up");
    elapsed
}

// --- Inspect overlay sample (§13, M8) ----------------------------------------

/// §13 overlay plan build + hand-off (1k nodes): the per-pump cost a live
/// inspector session adds — session builds every node's facts, the
/// §13 cost model composites the plan, the mock adapter records it (the
/// real adapter paints it inside the same pass, §15.4). `Session::pump`
/// keeps the zero-polling contract for sessions without the overlay; this
/// row guards the opt-in cost of the sessions that turned it on.
pub fn inspect_overlay_plan() -> Duration {
    use guiwright_driver::Session;
    use guiwright_driver::mock::MockAdapter;

    let mut adapter = MockAdapter::bare();
    adapter.push_update(full_thousand_update());
    adapter.push_frame(1);
    let mut session = Session::new(adapter);
    session.pump();

    let start = Instant::now();
    session
        .inspect_enable_overlay()
        .expect("the mock composites");
    let elapsed = start.elapsed();
    let plan = session
        .adapter()
        .overlay()
        .expect("plan handed to the adapter");
    assert_eq!(
        plan.badges.len(),
        998,
        "every Export button carries a badge"
    );
    elapsed
}

// --- Locator element-screenshot sample (§10 over the kit, M9) ----------------

/// Scratch baseline root (no tempfile dep in the bench crate): a
/// process-unique directory under the OS temp dir, removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// §10 element screenshot, kit side: one `locator.screenshot(name, bless)`
/// end-to-end — resolve (auto-wait) → one frame render → the clip-aware
/// crop (`guiwright-shot::crop`, the M1-bound hot path) → baseline assert.
/// The launch, settle and baseline bootstrap stay outside the measured
/// span; the measured operation is one element screenshot, the path every
/// visual assertion on a locator rides. The baseline is re-blessed per
/// call (Failing mode) so the measured cost includes the honest PNG
/// encode + IO, not a warm cache.
pub fn locator_element_screenshot() -> Duration {
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::{App, BlessMode};

    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    adapter.push_frame(1);
    let rgba: Vec<u8> = (0..240u32)
        .flat_map(|y| (0..480u32).map(move |x| (x, y)))
        .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
        .collect();
    let adapter = adapter.with_frame_pixels(480, 240, rgba);
    let scratch = ScratchDir(std::env::temp_dir().join(format!(
        "gwp-bench-element-{}-{}",
        std::process::id(),
        guiwright_trace::video::epoch_ms()
    )));
    let mut app = App::launch("locator_element_screenshot", adapter).with_baseline_dir(&scratch.0);
    app.wait_for_settle();
    app.get_by_role(accesskit::Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline bootstrapped");

    let start = Instant::now();
    let outcome = app
        .get_by_role(accesskit::Role::Button, "Save")
        .screenshot("save", BlessMode::Failing);
    let elapsed = start.elapsed();
    assert!(
        matches!(
            outcome,
            Ok(guiwright_shot::baseline::SnapshotOutcome::Matched { .. })
        ),
        "the deterministic frame matches the bootstrapped baseline"
    );
    elapsed
}

// --- Wire screenshot sample (§7 render domain over §10 capture, M10) ---------

/// §7 render domain (M10): one `Session::screenshot_window` end-to-end —
/// the pump-consistency pass, the single §10 render, and the PNG encode
/// that the `Shot` reply ships. The fixture launch + settle stay outside
/// the measured span; the measured operation is the hot path every wire
/// screenshot request rides (the transport round-trip itself is the
/// `resolve_remote_*` rows' marshalling pattern, already bound).
pub fn wire_screenshot_window() -> Duration {
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::Session;
    use guiwright_driver::mock::MockAdapter;

    let mut adapter = MockAdapter::with_fixture(&Fixture::buttons());
    adapter.push_frame(1);
    let rgba: Vec<u8> = (0..240u32)
        .flat_map(|y| (0..480u32).map(move |x| (x, y)))
        .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
        .collect();
    let adapter = adapter.with_frame_pixels(480, 240, rgba);
    let mut session = Session::new(adapter);
    session.pump();

    let start = Instant::now();
    let shot = session
        .screenshot_window()
        .expect("the scripted adapter renders");
    let elapsed = start.elapsed();
    assert_eq!((shot.width, shot.height), (480, 240));
    assert!(
        shot.png.len() > 64,
        "the PNG payload is a real encoded frame"
    );
    elapsed
}

// --- Tree-dump sample (§7 tree domain, M11) ----------------------------------

/// §7 tree-dump domain (M11): one `Session::tree_dump` over the 1k-node
/// tree — the pump-consistency pass, the per-window DFS walk and the row
/// build (role/name/value/bounds/children per node) that the `TreeDump`
/// reply ships. The wire serialization itself is the existing
/// `snapshot_serialize` row's postcard cost; this is the hot path every
/// `snapshot full` request rides.
pub fn tree_dump_walk() -> Duration {
    use guiwright_driver::Session;
    use guiwright_driver::mock::MockAdapter;

    let mut adapter = MockAdapter::bare();
    adapter.push_update(full_thousand_update());
    adapter.push_frame(1);
    let mut session = Session::new(adapter);
    session.pump();

    let start = Instant::now();
    let dump = session.tree_dump();
    let elapsed = start.elapsed();
    assert_eq!(dump.node_count, 1_000, "the whole synthetic tree");
    assert_eq!(dump.nodes[0].children, 1, "window → group");
    elapsed
}

// --- Event-channel sample (§7 server-push events over §15.6, M12) -----------

/// §7 event channel (M12): one publish-to-receipt round trip — a tree
/// diff lands on the session (the shared-script path), the idle tick
/// pumps at frame cadence, and the subscribed client's `next_event`
/// observes the `TreeChanged` frame. The measured span includes the frame
/// tick (the event loop pumps once per `FRAME_STEP` while anyone is
/// subscribed — that pacing IS the design, §15.4), so the row is a
/// liveness bound, not a CPU cost.
pub fn wire_event_delivery() -> Duration {
    const ITERATIONS: u32 = 16;
    use accesskit::{Node, NodeId, Role, TreeUpdate};
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::schema::EventKindV1;
    use guiwright_proto::uds::Client;
    use std::sync::{Arc, Mutex};
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let dir = std::env::temp_dir().join(format!("gw-bench-events-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("events.sock");

    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let scripts = Arc::clone(&shared);

    let (measured, handle) = runtime.block_on(async {
        let handle = GwpServerHandle::spawn_with_seams(
            path.clone(),
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench server binds");
        let mut client = Client::connect(&path).await.expect("bench client");
        client.handshake("gw-bench/0.1").await.expect("handshake");
        client
            .subscribe(&[EventKindV1::TreeChanged])
            .await
            .expect("subscribe");

        // A one-button rename per iteration: each lands as exactly one
        // coalesced TreeChanged (the frame events are filtered out).
        let started = Instant::now();
        for i in 0..ITERATIONS {
            let mut save = Node::new(Role::Button);
            save.set_label(format!("Save (v{i})"));
            save.set_bounds(accesskit::Rect {
                x0: 16.0,
                y0: 16.0,
                x1: 136.0,
                y1: 48.0,
            });
            shared.lock().expect("script lock").push(TreeUpdate {
                nodes: vec![(NodeId(2), save)],
                tree: None,
                tree_id: accesskit::TreeId::ROOT,
                focus: accesskit::NodeId(2),
            });
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let event = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(deadline),
                    client.next_event(),
                )
                .await
                .expect("event arrives within the tick bound")
                .expect("event frame");
                if matches!(
                    event,
                    guiwright_proto::schema::GwpEvent::V1(
                        guiwright_proto::schema::EventV1::TreeChanged { .. }
                    )
                ) {
                    break;
                }
            }
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
    measured.max(Duration::from_nanos(1))
}

// --- Log-tail sample (§7 event channel, the log-tail milestone) -----------

/// The log-tail milestone's session-side cost: ONE publish-point batch —
/// 64 records pushed through the bounded live tap (`LOG_TAIL_CAPACITY`,
/// the same mechanics `ServerOptions::log_tail` runs server-side), then
/// the tail drain + `EventV1::Log` frame build the publish performs while
/// a `Log` subscriber is attached. Record formatting rides the §12
/// `TraceLogLayer`-shaped JSON path upstream of the tap; the wire
/// round-trip is the `wire_event_delivery` row.
pub fn log_tail_batch() -> Duration {
    use guiwright_driver::{LOG_TAIL_CAPACITY, LogTail};
    use guiwright_proto::schema::{EventV1, GwpEvent};

    let (tail, tap) = LogTail::new(LOG_TAIL_CAPACITY);
    let records: Vec<String> = (0..64)
        .map(|i| {
            format!(
                r#"{{"ts_ns":{},"level":"DEBUG","target":"gw-bench","pid":{},"message":"record {i}"}}"#,
                i * 1_000,
                std::process::id()
            )
        })
        .collect();

    let start = Instant::now();
    for record in &records {
        tap.send(record.clone());
    }
    let drained = tail.drain();
    let events: Vec<GwpEvent> = drained
        .into_iter()
        .map(|record| GwpEvent::V1(EventV1::Log { record }))
        .collect();
    assert_eq!(events.len(), 64, "the batch drains fully");
    start.elapsed()
}

// --- Filtered-dump sample (§7 tree domain, M13) ------------------------------

/// §7 tree-dump domain (M13): one filtered dump over the 1k-node tree —
/// strict resolve of the group node + the depth-first subtree walk + row
/// build for its 998 children. The full-dump walk is the `tree_dump_walk`
/// row; this is the subtree form `snapshot under`/`treeDumpUnder` ride.
pub fn tree_dump_filtered() -> Duration {
    use guiwright_driver::Session;
    use guiwright_driver::mock::MockAdapter;

    let mut adapter = MockAdapter::bare();
    adapter.push_update(full_thousand_update());
    adapter.push_frame(1);
    let mut session = Session::new(adapter);
    session.pump();

    let start = Instant::now();
    let resolved = session
        .resolve(&Selector::Role {
            role: Role::Group,
            name: None,
        })
        .expect("the group resolves");
    let dump = session
        .tree_dump_under(&resolved.key)
        .expect("the subtree dumps");
    let elapsed = start.elapsed();
    assert_eq!(dump.node_count, 999, "the group + its 998 buttons");
    elapsed
}
