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

fn full_thousand_update() -> accesskit::TreeUpdate {
    const BUTTONS: u32 = 998;
    let mut window = accesskit::Node::new(Role::Window);
    window.set_label("Bench App");
    window.set_children([accesskit::NodeId(1)]);
    let mut group = accesskit::Node::new(Role::Group);
    group.set_children(
        (0..BUTTONS)
            .map(|i| accesskit::NodeId(2 + i as u64))
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
        nodes.push((accesskit::NodeId(2 + i as u64), btn));
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

/// §15.1 "Tree diff apply (1k nodes)" — consumer ChangeHandler fan-out.
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
                        _ => 0,
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

#[derive(serde::Serialize)]
pub struct SnapshotNode {
    id: u64,
    role: String,
    label: Option<String>,
    value: Option<String>,
    bounds: Option<[f64; 4]>,
}

/// §15.1 "TreeChanged diff blob (LZ4)" — compress a diff payload.
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
    const FRAMES: u64 = 256;
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
