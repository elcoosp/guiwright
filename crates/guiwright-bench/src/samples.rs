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

/// §15.1 "Tree diff apply (1k nodes)" — consumer `ChangeHandler` fan-out,
/// measuring only the apply; setup is excluded by measuring
/// `Instant::now()` after the tree is built.
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

/// §13 overlay replay (m8's named viewer gaps): one deep-inspect plan
/// rebuilt from an archived 1k-node snapshot through the same
/// `OverlayPlan::from_facts` cost model the live session speaks — the
/// replay's per-export cost is the live plan build's twin
/// ([`crate::samples::inspect_overlay_plan`]), paid at HTML-export time,
/// not per frame.
pub fn overlay_replay_plan() -> Duration {
    use guiwright_trace::schema::{AdapterRecord, TreeNode, TreeSnapshot};
    use guiwright_trace::{TraceOptions, TraceRecorder};
    use guiwright_viewer::overlay;
    use guiwright_viewer::{Timeline, TraceArchive};
    use std::time::Instant;

    const ITERATIONS: u32 = 64;

    let dir = std::env::temp_dir().join(format!("gw-bench-overlay-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("overlay.zip");

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
    recorder.tree_snapshot(TreeSnapshot {
        frame_seq: 1,
        nodes,
        focus: Some(0),
    });
    recorder.stop().expect("bench archive");

    let archive = TraceArchive::open(&path).expect("open");
    let timeline = Timeline::from_archive(&archive);

    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let plan = overlay::replay_plan_at(&timeline, 1).expect("bench replay");
        assert_eq!(plan.badges.len(), 1000, "every bounds-carrying node badged");
        assert!(plan.hover.is_some(), "the recorded focus outlined");
    }
    started.elapsed() / ITERATIONS
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
            | guiwright_driver::server::Endpoint::Ws(_)
            | guiwright_driver::server::Endpoint::InProcess => {
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

// --- M16 sample (in-process ping round-trip, §7's embedded door) ------------

/// §15.1 `inprocess_ping_roundtrip`: one Ping round-trip over the
/// in-process door (M16) — the embedded mode's steady-state latency, the
/// floor every socket door adds its transport cost on top of.
/// Byte-identical to the WS row's conversation; the duplex pipe is paid
/// once at connect, outside the measured window.
pub fn inprocess_ping_roundtrip() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
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

// --- M17 sample (wire action round-trip, §8.3's actionability gate) ---------

/// §15.1 `wire_action_click`: one Action round-trip over the in-process
/// door (M17) — resolve once outside the window, then measure the
/// force-less action path: the pre-resolved actionability gate (initial
/// evaluation + stability-confirmed frames), the generation-checked `act`
/// and the frame-boundary rendezvous pump. The ride-along cost of M17's
/// gate on top of the M16 transport floor.
pub fn wire_action_click() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::schema::{RequestV1, ResponseV1, SelectorV1, SemActionV1, TextMatchV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let selector = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: false,
                ignore_case: true,
            }),
        };
        let resolved = client
            .request(RequestV1::Resolve {
                selector: selector.clone(),
                timeout_ms: 2000,
            })
            .await
            .expect("bench resolve");
        let (node, generation) = match resolved {
            ResponseV1::Resolved {
                node, generation, ..
            } => (node, generation),
            other => unreachable!("expected Resolved, got {other:?}"),
        };

        for _ in 0..4 {
            let done = client
                .request(RequestV1::Action {
                    node: node.clone(),
                    generation,
                    action: SemActionV1::Click,
                    force: false,
                    timeout_ms: 2000,
                })
                .await
                .expect("warm action");
            assert!(matches!(done, ResponseV1::ActionDone { .. }));
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .request(RequestV1::Action {
                    node: node.clone(),
                    generation,
                    action: SemActionV1::Click,
                    force: false,
                    timeout_ms: 2000,
                })
                .await
                .expect("bench action");
            assert!(matches!(done, ResponseV1::ActionDone { .. }));
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- M30 sample (wire resolve-all, §8.2's set read over the door) -----------

/// §15.1 `resolve_all_wire`: one `ResolveAll` round-trip over the
/// in-process door (M30) — the m22/m26/m29 records' named protocol half,
/// the kit's `count()`/`all()` set read over GWP. The measured window is
/// the non-strict enumeration of the four-button fixture: the selector
/// conversion, the `resolve_all` walk and the per-match `ResolvedMatchV1`
/// rows marshalled back — no waiting (the snapshot read never waits, so
/// the floor is one rendezvous + one walk, the `resolve_remote_uds`
/// round-trip shape carrying four rows instead of one).
pub fn resolve_all_wire() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::schema::{ResponseV1, SelectorV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let selector = SelectorV1::Role {
            role: "Button".into(),
            name: None,
        };

        for _ in 0..4 {
            let done = client
                .request(guiwright_proto::schema::RequestV1::ResolveAll {
                    selector: selector.clone(),
                })
                .await
                .expect("warm set read");
            match done {
                ResponseV1::ResolvedAll { matches } => {
                    assert_eq!(matches.len(), 4, "the fixture's four buttons");
                }
                other => unreachable!("expected ResolvedAll, got {other:?}"),
            }
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .request(guiwright_proto::schema::RequestV1::ResolveAll {
                    selector: selector.clone(),
                })
                .await
                .expect("bench set read");
            match done {
                ResponseV1::ResolvedAll { matches } => {
                    assert_eq!(matches.len(), 4, "the fixture's four buttons");
                }
                other => unreachable!("expected ResolvedAll, got {other:?}"),
            }
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

// --- Expect screenshot retry sample (§16 over §10) --------------------------

/// §16 screenshot retry waiter, kit side: one
/// `expect(locator).to_have_screenshot(name, bless)` on its satisfied
/// path — the resolve consistency pump, one frame render, the clip-aware
/// crop (`crop_element`, the §10 hot path the retry shares with the
/// single-shot) and the waitable `check_snapshot` diff. The launch,
/// settle and baseline bootstrap stay outside the measured span; the
/// measured operation is the first-frame-match poll — the floor cost
/// every retry iteration rides (each mismatching poll adds one pump and
/// the artifact writes, bounded by the timeout, not by this budget).
pub fn expect_screenshot_retry() -> Duration {
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
        "gwp-bench-expect-shot-{}-{}",
        std::process::id(),
        guiwright_trace::video::epoch_ms()
    )));
    let mut app = App::launch("expect_screenshot_retry", adapter).with_baseline_dir(&scratch.0);
    app.wait_for_settle();
    app.get_by_role(accesskit::Role::Button, "Save")
        .screenshot("save", BlessMode::Failing)
        .expect("baseline bootstrapped");

    let start = Instant::now();
    guiwright_kit::expect(app.get_by_role(accesskit::Role::Button, "Save"))
        .to_have_screenshot("save", BlessMode::Off)
        .expect("first-frame match inside the retry waiter");
    let elapsed = start.elapsed();
    // The artifacts dir stays clean: a first-frame match writes nothing.
    assert!(
        !scratch
            .0
            .join("save")
            .read_dir()
            .expect("baseline dir")
            .filter_map(std::result::Result::ok)
            .any(|e| e.file_name().to_string_lossy().ends_with(".new.png")),
        "the satisfied poll wrote no mismatch artifacts"
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

/// §12 `viewer_live_sync`: one live-feed tree sync over the embedded
/// door (M18) — attach and the warm-ups outside the window, then
/// measure `sync_tree`: the `Snapshot::Full` request, the DFS-row →
/// `TreeNode` conversion with the parent-link rebuild and the scrubber
/// rebuild the resolved view reads. The M18 live viewer's steady-state
/// update step, on top of the M16 transport floor.
pub fn viewer_live_sync() -> Duration {
    const ITERATIONS: u32 = 32;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_viewer::live::LiveFeed;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        let mut feed = LiveFeed::new(client);
        feed.attach("gw-bench/0.1").await.expect("bench attach");

        for _ in 0..4 {
            feed.sync_tree().await.expect("warm sync");
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            feed.sync_tree().await.expect("bench sync");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Checked-state assertion sample (§8 check vocabulary, M20) ---------------

/// §16 assertions row, M20: one `to_be_checked()` kit assertion on the
/// checkbox-trio fixture — strict resolve plus the checked evaluation
/// through the session's event-driven wait. The assertion completes on the
/// initial evaluation (the state already holds, no pump), so this is the
/// floor every `expect(locator).to_*` adds on top of a resolve.
pub fn checked_assert() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::with_checkboxes();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("checked_assert", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        guiwright_kit::expect(app.get_by_role(Role::CheckBox, "Notifications"))
            .to_be_checked()
            .expect("bench assertion holds");
    }
    start.elapsed() / ITERATIONS
}

// --- Count-assertion sample (§16 assertions row, M22) ------------------------

/// §16 assertions row, M22: one `to_have_count(n)` kit assertion on the
/// button fixture — the selector-level assertion's steady-state cost: one
/// non-strict `resolve_all` walk (the filter/codegen index path) plus the
/// set-size compare through the session's zero-polling wait. The
/// assertion completes on the initial evaluation, so this is the floor
/// every count assertion adds on top of the `resolve_cached_index` row —
/// the first waiter that resolves the whole match set instead of one node.
pub fn count_assert() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("count_assert", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        guiwright_kit::expect(app.role_locator(Role::Button))
            .to_have_count(4)
            .expect("bench assertion holds");
    }
    start.elapsed() / ITERATIONS
}

// --- Negated-assertion sample (§16 assertions row, M24) ----------------------

/// §16 assertions row, M24: one `not().to_be_visible()` kit assertion on
/// the button fixture's absent "Ghost" locator — the negation decorator's
/// steady-state cost. The decorator inverts the inner `ChecksWaiter`'s
/// outcome, so the sampled path is one strict resolve that misses plus the
/// three-way outcome inversion; the assertion completes on the initial
/// evaluation (the node is absent, so the negated form holds), making this
/// the floor every `expect(locator).not().to_*` adds on top of a resolve.
pub fn not_assert() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("not_assert", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        guiwright_kit::expect(app.get_by_role(Role::Button, "Ghost"))
            .not()
            .to_be_visible()
            .expect("bench assertion holds");
    }
    start.elapsed() / ITERATIONS
}

// --- Count-comparator sample (§16 assertions row, M25) -----------------------

/// §16 assertions row, M25: one `to_have_count_greater_than(3)` kit
/// assertion on the button fixture — the count waiter's comparator mode
/// (the M22 record's reserved widening). The sampled path is the same
/// non-strict `resolve_all` walk the exact count rides, plus the one-sided
/// comparison; the assertion completes on the initial evaluation (4 > 3
/// holds), so this is the floor every comparator assertion adds on top of
/// the `resolve_cached_index` row.
pub fn count_compare() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("count_compare", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        guiwright_kit::expect(app.role_locator(Role::Button))
            .to_have_count_greater_than(3)
            .expect("bench assertion holds");
    }
    start.elapsed() / ITERATIONS
}

// --- Locator all-matches sample (§16 assertions row over the kit, m22/m25 gap) ---

/// §16 assertions row, the `all()` enumeration: one
/// `role_locator(Role::Button).all(|item| item.resolve())` over the button
/// fixture — one non-strict `resolve_all` walk (the snapshot set read) plus
/// one index-pinned strict resolve per visited item (the `.nth(i)` pin,
/// each with its own pump). The launch and settle stay outside the measured
/// span; the measured operation is one full enumeration, the hot path every
/// `all()`-driven iterate-and-collect flow rides. Zero matches enumerate to
/// an empty `Vec` with no per-item cost (the `count_assert` row is the set read
/// alone).
pub fn locator_all() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("locator_all", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let keys = app
            .role_locator(Role::Button)
            .all(|item| Ok(item.resolve()?.key))
            .expect("bench enumeration");
        assert_eq!(keys.len(), 4, "the fixture's four buttons");
    }
    start.elapsed() / ITERATIONS
}

// --- Editable-assertion sample (§16 assertions row, M28) ---------------------

/// §16 assertions row, M28: one `to_be_editable()` kit assertion on the
/// read-only fixture's editable input — the effective-read-only-state
/// assertion's steady-state cost: one strict resolve (auto-wait index
/// path) plus the `is_read_only()` evaluation through the session's
/// zero-polling wait. The assertion completes on the initial evaluation,
/// so this is the floor every editable/read-only assertion adds on top of
/// the `resolve_cached_index` row — the same shape as the M20 checked row
/// (a state check rides an existing waiter, no new machinery).
pub fn editable_assert() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::with_readonly();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("editable_assert", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        guiwright_kit::expect(app.get_by_role(Role::TextInput, "Bio"))
            .to_be_editable()
            .expect("bench assertion holds");
    }
    start.elapsed() / ITERATIONS
}

// --- Bulk text-read sample (§16 assertions row, M29) -------------------------

/// §16 assertions row, M29 (the m26 record's open kit gap): one
/// `role_locator(Role::Button).all_text_contents()` over the button
/// fixture — one non-strict `resolve_all` walk (the snapshot set read,
/// the `count_assert`/`locator_all` rows' cost) plus one label/value read
/// per match. The launch and settle stay outside the measured span; the
/// measured operation is one full set read, the hot path every
/// enumerate-and-collect flow rides. The per-match text read is an index
/// walk over the same consumer tree the resolution used — no extra
/// pumping, no waiter.
pub fn text_contents() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("text_contents", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let texts = app
            .role_locator(Role::Button)
            .all_text_contents()
            .expect("bench set read");
        assert_eq!(texts.len(), 4, "the fixture's four buttons");
    }
    start.elapsed() / ITERATIONS
}

// --- M31 sample (the value-first bulk read, the m29 record's named gap) -----

/// §15.1 `all_values`: one `all_values()` read (M31) — the m29 record's
/// named follow-up: the `text_contents` row's walk with the per-node
/// source order flipped (value first, label fallback). The measured
/// operation is one full set read over the button fixture: one
/// non-strict `resolve_all` walk plus one value/label read per match —
/// the same cost shape as the `text_contents` row (the flip is a
/// client-side accessor choice, not a session path).
pub fn all_values() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("all_values", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let values = app
            .role_locator(Role::Button)
            .all_values()
            .expect("bench value read");
        assert_eq!(values.len(), 4, "the fixture's four buttons");
    }
    start.elapsed() / ITERATIONS
}

// --- Content-getter sample (the m19 record's named §16 follow-on) ------

/// §15.1 `content_getter_read`: one single-node content getter read
/// (the m19 record's remaining-gaps list, cashed beside the m29/m31
/// bulk readers) — the `input_value()` getter on the fixture's Email
/// input. The measured operation is one strict auto-wait resolve (the
/// `describe` surface's single-node path) plus the value-fact read —
/// the `count_assert` row's resolve shape with the fact accessor in
/// place of the set walk; `inner_text()` rides the same surface (the
/// label-or-value accessor over the identical resolve), so one number
/// covers both getters' cost shape. The launch and settle stay outside
/// the measured span; the read is a snapshot, no waiter rides it.
pub fn content_getter_read() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("content_getter_read", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let value = app
            .get_by_role(Role::TextInput, "Email")
            .input_value()
            .expect("bench value read");
        assert_eq!(value, "ada@example.dev", "the carried value");
    }
    start.elapsed() / ITERATIONS
}

/// §15.1 `drag_to`: one locator `drag_to` (M33) — the m19 parity
/// stream's named drag item. The measured window is the whole embedded
/// operation on the button fixture: two `Attached`-gated auto-wait
/// resolves in the source locator's window context (the wire Resolve
/// discipline on both ends), the bounds-center pair, the
/// [`guiwright_driver::Gesture::Drag`] synthesis under the adapter's
/// pacing table, and the one rendezvous pump — the `kit_input_click`
/// gesture shape plus the second gated resolve.
pub fn drag_to() -> Duration {
    use accesskit::Role;
    use guiwright_core::Selector;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("drag_to", adapter);
    app.wait_for_settle();
    let target = Selector::role_named(Role::Button, "Cancel");

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let outcome = app
            .get_by_role(Role::Button, "Save")
            .drag_to(&target)
            .expect("bench drag");
        assert!(outcome.detail.contains("dragged"), "{outcome:?}");
    }
    start.elapsed() / ITERATIONS
}

// --- M34 sample (kit locator filter surface, §8.1 composition) --------------

/// §15.1 `locator_filter`: one filtered chained resolve (M34) — the
/// §8.1 filter vocabulary completed on the kit surface. The measured
/// window is one composed `Locator` read on the button fixture:
/// `group >> Button` narrowed by `has_not(Confirm)` — the core
/// resolver's `All`-chain walk plus one `Filter { has_not }` pass (the
/// per-candidate subtree probes), the composition every kit filter
/// method produces. A snapshot read: no waiting, no strict mode, the
/// `locator_all` row's shape with the subtree probes added.
pub fn locator_filter() -> Duration {
    use accesskit::Role;
    use guiwright_core::Selector;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;
    use std::time::Instant;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("locator_filter", adapter);
    app.wait_for_settle();
    let confirm = Selector::role_named(Role::Button, "Confirm");

    // Warm-up: the composed selector builds its chain + filter once; the
    // measured loop reads the steady-state set.
    let warm = app
        .role_locator(Role::Group)
        .locator(&Selector::role(Role::Button))
        .has_not(&confirm)
        .count()
        .expect("warm filter read");
    assert_eq!(warm, 3, "Save, Cancel, Invisible — Confirm excluded");

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let count = app
            .role_locator(Role::Group)
            .locator(&Selector::role(Role::Button))
            .has_not(&confirm)
            .count()
            .expect("bench filter read");
        assert_eq!(count, 3, "the composed set is stable");
    }
    start.elapsed() / ITERATIONS
}

// --- M32 sample (wire hit-test, §8.3's mechanism over the door) --------------

/// §15.1 `hit_test_wire`: one `HitTest` round-trip over the in-process
/// door (M32) — the §8.3 "receives events" mechanism as a public read.
/// The measured window is one point walk at the Save button's center plus
/// the one marshalled `ResolvedMatchV1` row: pump, `node_at_point` under
/// the common filter, generation + bounds attached — no waiting (a total
/// read; the warm-up's miss probe pins the `None` shape, the same round
/// trip with an empty answer).
pub fn hit_test_wire() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::schema::{RequestV1, ResponseV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        for (x, y, expected) in [(76.0, 32.0, true), (400.0, 220.0, false)] {
            let done = client
                .request(RequestV1::HitTest { x, y })
                .await
                .expect("warm hit read");
            match done {
                ResponseV1::HitTest { hit } => {
                    assert_eq!(hit.is_some(), expected, "warm probe shape");
                }
                other => unreachable!("expected HitTest, got {other:?}"),
            }
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .request(RequestV1::HitTest { x: 76.0, y: 32.0 })
                .await
                .expect("bench hit read");
            match done {
                ResponseV1::HitTest { hit } => {
                    assert!(hit.is_some(), "the save button sits under the point");
                }
                other => unreachable!("expected HitTest, got {other:?}"),
            }
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Kit window-handle sample (§16 v0.4 over the kit, M6 kit half) -----------
/// §16 v0.4 multi-window, kit side: one `app.window(handle).get_by_role(…)
/// .resolve()` through the window-handle surface — aim (validated against
/// the tree), subtree-scoped strict resolution, metadata read. The launch
/// and settle stay outside the measured span; the measured operation is
/// one scoped resolve, the hot path every window-pinned locator action
/// rides (the discriminator fixture: two "Save" buttons, one per window).
pub fn kit_window_scoped_resolve() -> Duration {
    use guiwright_core::test_util::MultiWindowFixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;
    use std::time::Instant;

    use accesskit::Role;

    const ITERATIONS: u32 = 64;

    let fx = MultiWindowFixture::default();
    let mut app = App::launch(
        "kit_window_scoped_resolve",
        MockAdapter::with_multi_window(&fx),
    );
    app.wait_for_settle();
    let w2 = fx.w2.0.to_string();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let resolved = app
            .window(&w2)
            .expect("Preview is a live target")
            .get_by_role(Role::Button, "Save")
            .resolve()
            .expect("Save resolves inside Preview");
        assert_eq!(resolved.key, fx.w2_save_key(), "the scope discriminates");
    }
    start.elapsed() / ITERATIONS
}

// --- M2-of-the-delivery sample: wire window targets (§16 v0.4 client) -------

/// §15.1 `wire_window_targets`: one `window_list` + `window_use` pair over
/// the in-process door through the `ClientConn` typed methods (§16 v0.4,
/// the Rust-client window parity) — the aim-and-scope round-trip every
/// window-aware Rust consumer rides. The handshake and warmup stay
/// outside the measured span; the measured operation is the pair.
pub fn wire_window_targets() -> Duration {
    const ITERATIONS: u32 = 32;
    use guiwright_core::test_util::MultiWindowFixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::{ClientConn, DuplexClient};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let fx = MultiWindowFixture::default();
    let w2 = fx.w2.0.to_string();
    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_multi_window(&fx),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        for _ in 0..4 {
            client.window_list().await.expect("warm list");
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let windows = client.window_list().await.expect("bench list");
            assert_eq!(windows.len(), 2, "the fixture's two targets");
            let aimed = client.window_use(&w2).await.expect("bench aim");
            assert_eq!(aimed, w2, "the ack echoes the aimed handle");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Node-describe sample (§8.1 over the wire, M36) -------------------------
/// §15.1 `node_describe`: one `Describe` round-trip over the in-process
/// door (M36) — the m32 record's standing gap, closed: one node's
/// identity facts (role/name/value/bounds) for a handle the client holds.
/// The measured window is one round trip on the Save button's resolve row:
/// pump, the generation guard, one consumer-tree node read, one
/// `DescribedV1` row marshalled back — no waiting (a handle-addressed
/// read; the warm-up's resolve mints the handle the loop describes).
pub fn node_describe() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::schema::{DescribeV1, RequestV1, ResponseV1, SelectorV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        // Warm-up: mint the handle the measured loop describes, and pin
        // the row shape once (role/name/bounds ride every reply).
        let (node, generation) = {
            let done = client
                .request(RequestV1::Resolve {
                    selector: SelectorV1::Role {
                        role: "Button".into(),
                        name: Some(guiwright_proto::schema::TextMatchV1 {
                            text: "Save".into(),
                            exact: true,
                            ignore_case: false,
                        }),
                    },
                    timeout_ms: 2000,
                })
                .await
                .expect("warm resolve");
            match done {
                ResponseV1::Resolved {
                    node, generation, ..
                } => (node, generation),
                other => unreachable!("expected Resolved, got {other:?}"),
            }
        };
        let warm = client
            .request(RequestV1::Describe(DescribeV1 {
                node: node.clone(),
                generation,
            }))
            .await
            .expect("warm describe");
        match warm {
            ResponseV1::Described(ref row) => {
                assert_eq!(row.role, "Button", "the warm probe pins the row shape");
            }
            other => unreachable!("expected Described, got {other:?}"),
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .request(RequestV1::Describe(DescribeV1 {
                    node: node.clone(),
                    generation,
                }))
                .await
                .expect("bench describe read");
            match done {
                ResponseV1::Described(ref row) => {
                    assert_eq!(row.name.as_deref(), Some("Save"), "still the Save button");
                }
                other => unreachable!("expected Described, got {other:?}"),
            }
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Identity-assertion sample (§16 assertions row, M38) ---------------------

/// §16 assertions row, M38: one identity-family kit assertion (`to_have_role`
/// on the button fixture's Save row — the cheapest of the three rows, whose
/// name/description siblings ride the same `ChecksWaiter` at the same cost
/// shape). The steady-state cost is the `checked_assert` row's: one strict
/// resolve (index hit) plus the check evaluation against the consumer node
/// (`role()` compare — no label/value string materialization), completing on
/// the initial evaluation. The family adds no new waiter machinery, so this
/// is the floor every `to_have_role` / `to_have_accessible_name` /
/// `to_have_accessible_description` adds on top of the `resolve_cached_index`
/// row.
pub fn identity_assert() -> Duration {
    use accesskit::Role;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_kit::App;

    const ITERATIONS: u32 = 64;

    let fixture = Fixture::buttons();
    let mut adapter = MockAdapter::with_fixture(&fixture);
    adapter.push_frame(1);
    adapter.push_frame(1);
    let mut app = App::launch("identity_assert", adapter);
    app.wait_for_settle();

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        guiwright_kit::expect(app.get_by_role(Role::Button, "Save"))
            .to_have_role(Role::Button)
            .expect("bench assertion holds");
    }
    start.elapsed() / ITERATIONS
}

// --- Describe-all sample (§8.2 over the wire, M39) ---------------------------
/// §15.1 `describe_all_wire`: one `DescribeAll` round-trip over the
/// in-process door (M39) — the m37 record's standing gap, closed: the
/// resolve-all walk's rows carrying the identity facts (role/name/value/
/// bounds), one round trip instead of one `Describe` per row. The
/// measured window is one round trip on the four-button set: pump, the
/// non-strict `resolve_all` walk (the `resolve_cached_index` index path),
/// one consumer-tree node read per row, the `DescribedAll` rows
/// marshalled back — no waiting (a snapshot read; the set is total).
pub fn describe_all_wire() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::schema::{DescribeAllV1, RequestV1, ResponseV1, SelectorV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        // Warm-up: pin the row shape once (role/name ride every row).
        let warm = client
            .request(RequestV1::DescribeAll(DescribeAllV1 {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: None,
                },
            }))
            .await
            .expect("warm describe-all");
        match warm {
            ResponseV1::DescribedAll { ref rows } => {
                assert_eq!(rows.len(), 4, "the fixture's four buttons");
                assert_eq!(rows[0].name.as_deref(), Some("Save"));
            }
            other => unreachable!("expected DescribedAll, got {other:?}"),
        }

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .request(RequestV1::DescribeAll(DescribeAllV1 {
                    selector: SelectorV1::Role {
                        role: "Button".into(),
                        name: None,
                    },
                }))
                .await
                .expect("bench describe-all read");
            match done {
                ResponseV1::DescribedAll { ref rows } => {
                    assert_eq!(rows.len(), 4, "still the four-button set");
                    assert_eq!(rows[0].name.as_deref(), Some("Save"));
                }
                other => unreachable!("expected DescribedAll, got {other:?}"),
            }
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Client-side wait sample (the m21 follow-up over the wire, m42) ---------

/// §15.1 `client_wait_satisfied`: one `ClientConn::wait_for` satisfied on
/// entry over the in-process door (m42 — the m21 record's named follow-up,
/// closed): the subscribe round-trip, the describe-all entry evaluation and
/// the unsubscribe cleanup — the steady-state floor every remote wait adds.
/// The Save button is visible from the start, so the wait completes on the
/// first row set: no wake, no re-read (the event-driven cost is the
/// `wire_event_delivery` row's, not this one's).
pub fn client_wait_satisfied() -> Duration {
    const ITERATIONS: u32 = 32;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::{SelectorV1, TextMatchV1};
    use guiwright_proto::wait::WaitState;
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        // Warm-up: one full satisfied wait (subscription window, entry
        // evaluation, cleanup) pins the row shape once.
        let warm = client
            .wait_for(save.clone(), WaitState::Visible, Duration::from_secs(1))
            .await
            .expect("warm wait");
        let warm = warm.expect("a positive state answers its row");
        assert_eq!(warm.name.as_deref(), Some("Save"));

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .wait_for(save.clone(), WaitState::Visible, Duration::from_secs(1))
                .await
                .expect("bench wait");
            let done = done.expect("a positive state answers its row");
            assert_eq!(done.name.as_deref(), Some("Save"), "still the Save row");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Client seam aim-and-act sample (the §7 loop on the seam, m47) -----------

/// §15.1 `client_resolve_act`: one [`ClientConn::resolve`] → one
/// [`ClientConn::action`] over the in-process door (m47 — the §7
/// locate-and-act loop as typed seam sugar): the strict resolve with the
/// driver's auto-wait (the wire `Resolve` path, the `resolve_cached_index`
/// index answer), then the action on that row (the `wire_action` act-time
/// gate re-checking the pinned node, the frame rendezvous) — the floor
/// the TS client's `resolve()`/`action()` pair has carried since M3, now
/// on the Rust seam; no wire change.
pub fn client_resolve_act() -> Duration {
    const ITERATIONS: u32 = 32;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::{SelectorV1, SemActionV1, TextMatchV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        // Warm-up: one full aim-and-act loop pins the receipt shape once.
        let warm_row = client
            .resolve(save.clone(), 2000)
            .await
            .expect("warm resolve");
        let warm_receipt = client
            .action(
                warm_row.node.clone(),
                warm_row.generation,
                SemActionV1::Click,
                false,
                5000,
            )
            .await
            .expect("warm action");
        assert_eq!(warm_receipt.detail, "clicked");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let row = client
                .resolve(save.clone(), 2000)
                .await
                .expect("bench resolve");
            let receipt = client
                .action(
                    row.node.clone(),
                    row.generation,
                    SemActionV1::Click,
                    false,
                    5000,
                )
                .await
                .expect("bench action");
            assert_eq!(receipt.detail, "clicked", "still the Save row");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Client seam bulk-content sample (the m41 half on the Rust seam, m46) ----

/// §15.1 `client_text_contents`: one `ClientConn::all_text_contents`
/// read over the in-process door (m46 — the m41 record's Rust-client
/// half, closed): the describe-all round trip plus the client-side m29
/// derivation — the steady-state floor every bulk content read adds on
/// top of the `describe_all_wire` row-set read (the same round-trip
/// shape; the derivation is a per-row map, no extra wire work).
pub fn client_text_contents() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::SelectorV1;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let buttons = SelectorV1::Role {
            role: "Button".into(),
            name: None,
        };
        // Warm-up: the derivation pins the row shape once (label first,
        // the fixture's hidden button included — the non-strict walk).
        let warm = client
            .all_text_contents(buttons.clone())
            .await
            .expect("warm bulk read");
        assert_eq!(
            warm,
            vec!["Save", "Cancel", "Invisible", "Confirm"],
            "the m29 order, document order"
        );

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .all_text_contents(buttons.clone())
                .await
                .expect("bench bulk read");
            assert_eq!(
                done,
                vec!["Save", "Cancel", "Invisible", "Confirm"],
                "still the four-button set"
            );
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Client seam gesture sample (the §9 input half on the seam, m48) ---------

/// §15.1 `client_gesture`: one `ClientConn::gesture` click over the
/// in-process door (m48 — the §9 input half as typed seam sugar): the
/// driver-side synthesis from the adapter's pacing table (the
/// `gesture_synthesis` micro-path) plus the batch injection through the
/// one-rendezvous pump (the `wire_action_click` round-trip shape, minus
/// the actionability gate) — the floor the TS client's `gesture()` and
/// the REPL's `mouse`/`key`/`text` rows have carried since M3/M7, now on
/// the Rust seam; a client-side sugar over an existing domain, no wire
/// change.
pub fn client_gesture() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::{GestureV1, PointerButtonV1};
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let click = GestureV1::Click {
            x: 76.0,
            y: 32.0,
            button: PointerButtonV1::Primary,
            count: 1,
        };
        // Warm-up: one click pins the receipt shape once (the synthesis
        // summary the REPL's gesture row renders).
        let warm = client.gesture(click.clone()).await.expect("warm gesture");
        assert!(
            warm.detail.contains("clicked at"),
            "the synthesis summary: {}",
            warm.detail
        );

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let receipt = client.gesture(click.clone()).await.expect("bench gesture");
            assert!(receipt.frame_seq > 0, "the batch applied inside a frame");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Client seam tree-dump sample (the M11/M13 half on the seam, m49) --------

/// §15.1 `client_tree_dump`: one `ClientConn::tree_dump_under` read over
/// the in-process door (m49 — the m13 record's dump-row half on the
/// seam): the strict resolve with the driver's auto-wait (the
/// `resolve_cached_index` index answer) plus the depth-first subtree walk
/// and row build (the `tree_dump_filtered` shape on the four-button
/// fixture) — the floor the TS client's `treeDumpUnder()` and the REPL's
/// `snapshot under` row have carried since M3/M13, now on the Rust seam;
/// a client-side sugar over an existing domain, no wire change.
pub fn client_tree_dump() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::{SelectorV1, TextMatchV1};
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        // Warm-up: one subtree dump pins the carrier shape once (a leaf
        // locator answers one row, itself).
        let warm = client
            .tree_dump_under(save.clone(), 2000)
            .await
            .expect("warm subtree dump");
        assert_eq!(warm.nodes.len(), 1, "a leaf locator dumps itself");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let dump = client
                .tree_dump_under(save.clone(), 2000)
                .await
                .expect("bench subtree dump");
            assert_eq!(dump.nodes[0].name.as_deref(), Some("Save"));
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Client seam screenshot sample (the M10 render half on the seam, m50) ----

/// §15.1 `client_screenshot`: one `ClientConn::screenshot_element` read
/// over the in-process door (m50 — the M10 render half as typed seam
/// sugar): the strict resolve with the driver's auto-wait (the
/// `resolve_cached_index` index answer) plus the one §10 render, the
/// clip-aware crop and the PNG encode the `Shot` reply ships (the
/// `wire_screenshot_window` shape plus the resolve, on the 480×240
/// fixture) — the floor the TS client's `screenshotElement()` and the
/// REPL's `screenshot element` row have carried since M9/M10, now on the
/// Rust seam; a client-side sugar over an existing domain, no wire change.
pub fn client_screenshot() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::{SelectorV1, TextMatchV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let rgba: Vec<u8> = (0..240u32)
        .flat_map(|y| (0..480u32).map(move |x| (x, y)))
        .flat_map(|(x, y)| [(x & 0xff) as u8, (y & 0xff) as u8, 99u8, 255u8])
        .collect();
    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || {
                MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(480, 240, rgba)
            },
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        // Warm-up: one element shot pins the carrier shape once (the Save
        // button's clip-aware crop).
        let warm = client
            .screenshot_element(save.clone(), 2000)
            .await
            .expect("warm element shot");
        assert_eq!((warm.width, warm.height), (120, 32));

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let shot = client
                .screenshot_element(save.clone(), 2000)
                .await
                .expect("bench element shot");
            assert_eq!((shot.width, shot.height), (120, 32), "still the crop");
            assert!(!shot.png.is_empty(), "the PNG payload ships");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- Client seam clock sample (the §14 half on the seam, m51) ----------------

/// §15.1 `client_clock`: one `ClientConn::clock` fast-forward over the
/// in-process door (m51 — the §14 virtual-clock half as typed seam sugar):
/// the wire `Clock` round trip with the §14 pacing service planning the
/// frame-sized steps (the session's `clock` path, one pump per step) and
/// the `ClockState` reply marshalled back — the floor the TS client's
/// `clock()`/`fastForward()` helpers and the REPL's `clock ff` row have
/// carried since §14/M4, now on the Rust seam; a client-side sugar over an
/// existing domain, no wire change.
pub fn client_clock() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::ClockV1;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        // Warm-up: install + one jump pins the carrier shape once (the
        // whole budget lands on the virtual clock, steps reported).
        client
            .clock(ClockV1::InstallVirtual)
            .await
            .expect("warm install");
        let warm = client
            .clock(ClockV1::FastForward { ms: 100 })
            .await
            .expect("warm fast-forward");
        assert_eq!(warm.virtual_time_ms, Some(100));
        assert!(warm.steps >= 1);

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let state = client
                .clock(ClockV1::FastForward { ms: 100 })
                .await
                .expect("bench fast-forward");
            assert!(
                state.virtual_time_ms.is_some(),
                "still on the virtual clock"
            );
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

/// §15.1 `client_mock`: one `ClientConn::mock` call over the in-process
/// door (m52 — the §14 state-seam half as typed seam sugar): the wire
/// `Mock` round trip running a registered seam inside the adapter's runner
/// context (`run_in` + the settle pump) and the `MockDone` reply
/// marshalled into the carrier — the floor the TS client's `mock()` /
/// `callSeam()` / `resetState()` helpers and the REPL's `mock` rows have
/// carried since §14/M4, now on the Rust seam; a client-side sugar over an
/// existing domain, no wire change.
pub fn client_mock() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::MockV1;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            // The hook runs in the runner context (§14): the state slot must
            // be attached or `run_in` is loud (the mock's own contract).
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_state(()),
            {
                let mut seams = guiwright_driver::SeamRegistry::new();
                seams.register("seed_users", |_ctx, _scope| Ok("3 users seeded".into()));
                seams
            },
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        // Warm-up: one call pins the carrier shape once (the hook ran in
        // the runner context, the detail rode back).
        let warm = client
            .mock(MockV1::Call {
                id: "seed_users".into(),
                scope: "warm".into(),
            })
            .await
            .expect("warm call");
        assert_eq!(warm.detail, "3 users seeded");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let receipt = client
                .mock(MockV1::Call {
                    id: "seed_users".into(),
                    scope: "bench".into(),
                })
                .await
                .expect("bench call");
            assert_eq!(receipt.id, "seed_users", "the reply names the seam");
            assert_eq!(receipt.detail, "3 users seeded");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

/// §15.1 `client_inspect`: one `ClientConn::inspect` overlay-enable round
/// trip over the in-process door (m53 — the §13 inspector half as typed
/// seam sugar): the wire `Inspect` round trip rebuilding the overlay plan
/// from the tree indexes (the session's enable path, one pump + the
/// adapter's `show_overlay`) and the `Pong` ack marshalled into the
/// carrier — the floor the TS client's `inspectOverlay()` helper and the
/// REPL's `inspect` row have carried since §13/M8, now on the Rust seam; a
/// client-side sugar over an existing domain, no wire change.
pub fn client_inspect() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::client::InspectReply;
    use guiwright_proto::schema::InspectV1;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        // Warm-up: enable + disable pins the carrier shape once (the acks
        // ride the wire's shared `Pong`).
        let warm = client
            .inspect(InspectV1::EnableOverlay)
            .await
            .expect("warm enable");
        assert_eq!(warm, InspectReply::Applied);
        client
            .inspect(InspectV1::DisableOverlay)
            .await
            .expect("warm disable");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let ack = client
                .inspect(InspectV1::EnableOverlay)
                .await
                .expect("bench enable");
            assert_eq!(ack, InspectReply::Applied, "the ack arm");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}
/// §15.1 `client_assert`: one `ClientConn::expect_text` satisfied on
/// entry over the in-process door (m54 — the kit's `expect` vocabulary's
/// content half as a client-side composition): the subscribe round-trip,
/// the describe-all entry evaluation (the `describe_all_wire` round-trip
/// shape with the m46 name-first derivation on top) and the unsubscribe
/// cleanup — the Save row satisfies on the first row set, so no wake and
/// no re-read ride this number (the event-driven cost is the
/// `wire_event_delivery` row's); a client-side composition of existing
/// domains, no wire change.
pub fn client_assert() -> Duration {
    const ITERATIONS: u32 = 32;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::{SelectorV1, TextMatchV1};
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: true,
                ignore_case: false,
            }),
        };
        let expected = TextMatchV1 {
            text: "Save".into(),
            exact: true,
            ignore_case: false,
        };
        // Warm-up: one full satisfied assertion (subscription window,
        // entry evaluation, cleanup) pins the row shape once.
        let warm = client
            .expect_text(save.clone(), expected.clone(), Duration::from_secs(1))
            .await
            .expect("warm assertion");
        assert_eq!(warm.name.as_deref(), Some("Save"));

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let done = client
                .expect_text(save.clone(), expected.clone(), Duration::from_secs(1))
                .await
                .expect("bench assertion");
            assert_eq!(done.name.as_deref(), Some("Save"), "still the Save row");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

/// §15.1 `client_video`: one `ClientConn::video` record cycle over the
/// in-process door (m55) — the start round trip, one presented frame (the
/// resolve's rendezvous pump) and the stop round trip answering the
/// finalized `VideoOutput` carrier. The measured operation is the whole
/// cycle: the §10 finalize (manifest + encoded frames) is the stop arm's
/// payload, not setup.
pub fn client_video() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::client::VideoReply;
    use guiwright_proto::schema::SelectorV1;
    use guiwright_proto::schema::{TextMatchV1, VideoOptsV1, VideoV1};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    // Scratch artifact root (no tempfile dep in the bench crate): one dir
    // for the whole run, 64 finalized files inside it.
    let dir = std::env::temp_dir().join(format!("gw-bench-video-seam-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("bench artifact dir");

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            // The script pixels give the adapter the §6.2 pixel row the
            // recorder needs — uniform bytes hash-0, so the static span
            // records first frame + dedup no-ops (§10 backpressure).
            || {
                MockAdapter::with_fixture(&Fixture::buttons()).with_frame_pixels(
                    24,
                    12,
                    vec![64; 24 * 12 * 4],
                )
            },
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let save = SelectorV1::Role {
            role: "Button".into(),
            name: Some(TextMatchV1 {
                text: "Save".into(),
                exact: false,
                ignore_case: true,
            }),
        };

        // Warm-up: one full record cycle pins the carrier shape once (the
        // start ack, the presented frame, the finalized artifact).
        let warm = client
            .video(VideoV1::Start(VideoOptsV1 {
                fps: 25,
                dir: Some(dir.display().to_string()),
            }))
            .await
            .expect("warm start");
        assert_eq!(warm, VideoReply::Started);
        let _ = client
            .resolve(save.clone(), 2000)
            .await
            .expect("warm resolve");
        let warm = client.video(VideoV1::Stop).await.expect("warm stop");
        let VideoReply::File(file) = warm else {
            panic!("expected the File arm");
        };
        assert!(file.frames >= 1, "at least the first presented frame");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let ack = client
                .video(VideoV1::Start(VideoOptsV1 {
                    fps: 25,
                    dir: Some(dir.display().to_string()),
                }))
                .await
                .expect("bench start");
            assert_eq!(ack, VideoReply::Started);
            let _ = client
                .resolve(save.clone(), 2000)
                .await
                .expect("bench resolve");
            let stopped = client.video(VideoV1::Stop).await.expect("bench stop");
            let VideoReply::File(file) = stopped else {
                panic!("expected the File arm");
            };
            assert!(file.frames >= 1, "the cycle recorded a frame");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
    measured.max(Duration::from_nanos(1))
}

/// §15.1 `client_input`: one `ClientConn::input` call over the in-process
/// door (m56) — a two-move pointer batch riding the `Input` domain's
/// rendezvous pump, the reply's frame seq marshalled into the
/// [`InputReceipt`] carrier. The measured operation is the round trip
/// (injection + pump), the same shape the TS client's `input(events)` has
/// spoken since M7.
pub fn client_input() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::InputEventV1;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        let batch = || {
            vec![
                InputEventV1::PointerMove { x: 12.0, y: 34.0 },
                InputEventV1::PointerMove { x: 13.0, y: 35.0 },
            ]
        };

        // Warm-up: one batch pins the carrier shape once (the frame seq
        // rode back).
        let warm = client.input(batch()).await.expect("warm batch");
        assert!(warm.frame_seq >= 1, "the receipt names the applied frame");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let receipt = client.input(batch()).await.expect("bench batch");
            assert!(receipt.frame_seq >= 1, "the rendezvous advanced");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

/// §15.1 `client_tree_snapshot`: one `ClientConn::tree_snapshot` call over
/// the in-process door (m57) — the §7 summary read riding the
/// `TreeSnapshot` domain, the reply's generation + node count marshalled
/// into the [`TreeSnapshotSummary`] carrier. The measured operation is the
/// round trip (the pump + the count), the same shape the TS client's
/// `treeSnapshot()` has spoken since its M3 landing.
pub fn client_tree_snapshot() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::{ClientConn, TreeSnapshotSummary};
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        // Warm-up: one summary read pins the carrier shape once (the 8
        // nodes rode back with their generation).
        let warm = client.tree_snapshot().await.expect("warm summary read");
        assert_eq!(warm.node_count, 8, "the M0 fixture's 8 nodes");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let summary: TreeSnapshotSummary =
                client.tree_snapshot().await.expect("bench summary read");
            assert_eq!(summary.node_count, 8, "still the whole tree");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

/// §15.1 `client_bye`: one `ClientConn::bye` call over the in-process door
/// (m58) — the §7 lifecycle farewell riding the `Bye` domain, answered
/// `Ok(())` before the conversation ends (`run_connection` returns right
/// after the `Bye` frame, so each measured iteration opens a fresh
/// connection; the open is setup, the round trip is the measured
/// operation). The same round trip the TS client's `close()` has made
/// since its M3 landing before its own transport teardown.
pub fn client_bye() -> Duration {
    const ITERATIONS: u32 = 64;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use std::time::Instant;
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");

        // Warm-up: one full farewell pins the row shape once (the bye
        // ends the conversation — the successor connection below carries
        // the rest).
        let mut warm_client = DuplexClient::connect(&connect)
            .await
            .expect("warm in-process client");
        warm_client
            .handshake("gw-bench/0.1")
            .await
            .expect("handshake");
        warm_client.bye().await.expect("warm farewell");
        drop(warm_client);

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let mut client = DuplexClient::connect(&connect)
                .await
                .expect("bench in-process client");
            client.handshake("gw-bench/0.1").await.expect("handshake");
            client.bye().await.expect("bench farewell");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}
/// §15.1 `client_count_assert`: one `ClientConn::expect_count` satisfied
/// on entry over the in-process door (m59 — the kit's `to_have_count`
/// family as a client-side composition): the subscribe round-trip, the
/// resolve-all entry evaluation (the `resolve_all_wire` round-trip shape
/// with the `CountMatch` comparison on top) and the unsubscribe cleanup —
/// the whole-match-set floor satisfies on the first row set, so no wake
/// and no re-read ride this number (the event-driven cost is the
/// `wire_event_delivery` row's); a client-side composition of existing
/// domains, no wire change.
pub fn client_count_assert() -> Duration {
    const ITERATIONS: u32 = 32;
    use guiwright_core::test_util::Fixture;
    use guiwright_driver::mock::MockAdapter;
    use guiwright_driver::server::GwpServerHandle;
    use guiwright_proto::DuplexClient;
    use guiwright_proto::client::ClientConn;
    use guiwright_proto::schema::SelectorV1;
    use std::time::{Duration, Instant};
    use tokio::runtime::Runtime;

    let runtime = Runtime::new().expect("bench runtime");
    let (measured, handle) = runtime.block_on(async {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("bench in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("bench in-process client");
        client.handshake("gw-bench/0.1").await.expect("handshake");

        // The whole match set (strict mode never consulted — the m22
        // record's set read): the fixture's four buttons.
        let buttons = SelectorV1::Role {
            role: "Button".into(),
            name: None,
        };
        // Warm-up: one full satisfied assertion (subscription window,
        // entry evaluation, cleanup) pins the row shape once.
        let warm = client
            .expect_count(buttons.clone(), 4, Duration::from_secs(1))
            .await
            .expect("warm assertion");
        assert_eq!(warm, 4, "the fixture's four buttons");

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let found = client
                .expect_count(buttons.clone(), 4, Duration::from_secs(1))
                .await
                .expect("bench assertion");
            assert_eq!(found, 4, "still the four-button set");
        }
        (started.elapsed() / ITERATIONS, handle)
    });

    handle.shutdown();
    measured.max(Duration::from_nanos(1))
}

// --- §15.6 shared-frame ring hop (m88) --------------------------------------

/// The §15.1 bench frame (1280×800 RGBA8) the hop measures.
const RING_W: u32 = 1280;
const RING_H: u32 = 800;

thread_local! {
    /// One ring per bench thread: the store is built once, the hop
    /// (publish + digest-validated read) is the measured operation.
    /// `RefCell` because the writer half commits by nature.
    static RING_HOP: std::cell::RefCell<RingBench> =
        std::cell::RefCell::new(RingBench::new());
}

/// The §15.6 ring fixture: a process-unique scratch root (the bench
/// crate's no-tempfile posture), one writer + one reader + one reused
/// read buffer over a 1280×800 RGBA8 store.
struct RingBench {
    _scratch: ScratchDir,
    writer: guiwright_proto::shmring::FrameRingWriter,
    reader: guiwright_proto::shmring::FrameRingReader,
    pixels: Vec<u8>,
    buf: Vec<u8>,
}

impl RingBench {
    fn new() -> Self {
        use guiwright_proto::shmring::{FrameRingReader, FrameRingWriter};
        let scratch = ScratchDir(std::env::temp_dir().join(format!(
            "gw-bench-shm-ring-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        )));
        std::fs::create_dir_all(&scratch.0).expect("bench ring dir");
        let path = scratch.0.join("session.gwring");
        // Pseudo-random pixels: the copy is the cost, never a compressible
        // pattern (the same honesty the blob sample's incompressible
        // buffer carries).
        let pixels: Vec<u8> = (0..(u64::from(RING_W) * u64::from(RING_H) * 4))
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let mut writer = FrameRingWriter::create(&path, RING_W, RING_H).expect("bench ring");
        // The reader opens against a committed store (the client-attach
        // order the §15.6 seam documents); the warm commit is setup.
        writer.publish(1, &pixels).expect("warm commit");
        let reader = FrameRingReader::open(&path).expect("bench ring reader");
        Self {
            _scratch: scratch,
            writer,
            reader,
            pixels,
            buf: Vec::new(),
        }
    }
}

/// §15.6 "a shared-memory frame ring for same-host clients" (m88): one
/// publication hop — `FrameRingWriter::publish` of the 1280×800 RGBA8
/// frame (payload + record commit + header stats) followed by the reader's
/// `latest_into` digest-validated read into a reused buffer. The ring is
/// built once per thread; the hop is the measured operation.
pub fn shm_ring_frame_hop() -> Duration {
    RING_HOP.with_borrow_mut(|bench| {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(2);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let started = Instant::now();
        bench
            .writer
            .publish(seq, &bench.pixels)
            .expect("bench publish");
        let frame = bench
            .reader
            .latest_into(std::mem::take(&mut bench.buf))
            .expect("bench read")
            .expect("the just-published frame");
        debug_assert_eq!(frame.seq, seq, "the hop serves the newest commit");
        bench.buf = frame.pixels;
        started.elapsed()
    })
}
