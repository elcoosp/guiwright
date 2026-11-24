//! End-to-end: the recorder writes a `gwtrace/v1` archive, the viewer
//! reads it back, resolves the timeline and renders the HTML (§12 full
//! pipeline).

use guiwright_trace::TraceOptions;
use guiwright_trace::schema::{
    AdapterRecord, ClockCommand, ClockJump, SCHEMA, TreeNode, TreeSnapshot,
};
use guiwright_viewer::{Timeline, TraceArchive};

fn record_archive(path: std::path::PathBuf) -> guiwright_trace::TraceFile {
    let options = TraceOptions::new(
        path,
        "viewer_e2e",
        AdapterRecord {
            name: "mock".into(),
            framework: "fixture".into(),
            framework_version: "0.1.0".into(),
        },
    );
    let mut recorder = guiwright_trace::TraceRecorder::start(options);

    // Two snapshots, two diffs between them, a couple of events, a clock
    // jump, and one deduped frame pair.
    recorder.tree_snapshot(TreeSnapshot {
        frame_seq: 1,
        nodes: vec![
            TreeNode {
                id: 1,
                parent: None,
                role: "Window".into(),
                label: Some("App".into()),
                value: None,
                bounds: Some([0.0, 0.0, 800.0, 600.0]),
            },
            TreeNode {
                id: 2,
                parent: Some(1),
                role: "Button".into(),
                label: Some("Save".into()),
                value: None,
                bounds: Some([16.0, 16.0, 136.0, 48.0]),
            },
        ],
        focus: Some(2),
    });
    recorder.event(1, "click", "click @ (76, 32)");
    recorder.tree_diff(2, vec![], vec![2], vec![], false);
    recorder.frame(1, 0xAAAA, vec![1, 2, 3, 4]);
    recorder.frame(2, 0xAAAA, vec![]); // deduped (same hash)
    recorder.frame(3, 0xABCD, vec![5, 6, 7, 8]);
    recorder.tree_snapshot(TreeSnapshot {
        frame_seq: 4,
        nodes: vec![
            TreeNode {
                id: 1,
                parent: None,
                role: "Window".into(),
                label: Some("App".into()),
                value: None,
                bounds: Some([0.0, 0.0, 800.0, 600.0]),
            },
            TreeNode {
                id: 2,
                parent: Some(1),
                role: "Button".into(),
                label: Some("Save All".into()),
                value: None,
                bounds: Some([16.0, 16.0, 136.0, 48.0]),
            },
            TreeNode {
                id: 3,
                parent: Some(1),
                role: "Button".into(),
                label: Some("Export".into()),
                value: None,
                bounds: Some([16.0, 56.0, 136.0, 88.0]),
            },
            // The deep-inspect total rule's honest case: a bounds-less node
            // carries no badge (nothing to pin to a point, §13).
            TreeNode {
                id: 4,
                parent: Some(1),
                role: "Text".into(),
                label: Some("Ready".into()),
                value: None,
                bounds: None,
            },
        ],
        focus: Some(3),
    });
    recorder.tree_diff(5, vec![], vec![], vec![2], false);
    recorder.clock_jump(ClockJump {
        frame_seq: 6,
        command: ClockCommand::FastForward { micros: 30_000_000 },
        virtual_time_us: 30_000_000,
    });
    recorder.stop().expect("archive written")
}

#[test]
fn full_pipeline_recorder_to_html() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("trace.zip");
    let file = record_archive(path.clone());
    let stats = file.stats;
    assert!(stats.tree_snapshots >= 2);
    assert!(stats.frames >= 2, "dedup keeps {stats:?}");
    assert_eq!(stats.frames_deduped, 1, "the same-hash frame is skipped");

    let archive = TraceArchive::open(&path).expect("archive opens");
    assert_eq!(archive.meta.schema, SCHEMA);
    assert_eq!(archive.meta.test, "viewer_e2e");

    // Snapshots: two, keyed by frame.
    assert_eq!(archive.snapshots.len(), 2);
    assert!(archive.snapshots.contains_key(&1) && archive.snapshots.contains_key(&4));

    // Diffs: two, recovered from the concatenated LZ4 stream.
    assert_eq!(archive.diffs.len(), 2);
    assert_eq!(archive.diffs[0].updated, vec![2]);
    assert_eq!(archive.diffs[1].removed, vec![2]);

    // Events.
    assert_eq!(archive.events.len(), 1);
    assert_eq!(archive.events[0].kind, "click");

    // Clock ledger.
    assert_eq!(archive.clock.len(), 1);
    assert_eq!(archive.clock[0].virtual_time_us, 30_000_000);

    // Frames: hash-dedup left seq 1 and 3.
    let png_keys = archive.frames_png_keys();
    assert_eq!(png_keys, [1u64, 3].into_iter().collect());
    assert_eq!(
        archive.frame_png(3).expect("payload"),
        &[5, 6, 7, 8],
        "stored bytes round-trip"
    );

    // Timeline: ticks cover frames 1..=6.
    let timeline = Timeline::from_archive(&archive);
    let seqs: Vec<u64> = timeline.ticks.iter().map(|t| t.frame_seq).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);

    // Tree at frame 3: snapshot 1 payload, updated id 2 pending.
    let view = timeline.tree_at(3).expect("view");
    assert_eq!(view.snapshot_frame, Some(1));
    assert!(view.pending_changes.contains(&2));
    let save = view.nodes.iter().find(|n| n.id == 2).expect("node row");
    assert_eq!(save.label.as_deref(), Some("Save"), "old payload");

    // Tree at frame 5: snapshot 4 payload, id 2 removed at frame 5.
    let view = timeline.tree_at(5).expect("view");
    let save = view.nodes.iter().find(|n| n.id == 2).expect("greyed row");
    assert!(save.removed);
    assert_eq!(view.focus, Some(3));

    // The HTML carries the payload and renders standalone.
    let html = guiwright_viewer::render_html(&archive, &timeline);
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("viewer_e2e"));
    assert!(html.contains("Save"));
    assert!(html.contains("FastForward"));
    assert!(!html.contains("http://"), "self-contained: {html}");
}

#[test]
fn schema_mismatch_is_rejected() {
    use std::io::Write as _;

    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("foreign.zip");
    let file = std::fs::File::create(&path).expect("create");
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("meta.json", zip::write::SimpleFileOptions::default())
        .expect("start");
    zip.write_all(br#"{"schema":"other/v9"}"#).expect("write");
    zip.finish().expect("finish");

    let err = TraceArchive::open(&path).expect_err("foreign schema");
    assert!(err.to_string().contains("not a gwtrace archive"), "{err}");
}

#[test]
fn empty_run_opens_with_no_content() {
    // An archive from an empty run: no diffs, no events, no ticks.
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("sparse.zip");
    let options = TraceOptions::new(
        path.clone(),
        "sparse",
        AdapterRecord {
            name: "mock".into(),
            framework: "fixture".into(),
            framework_version: "0.1.0".into(),
        },
    );
    let recorder = guiwright_trace::TraceRecorder::start(options);
    recorder.stop().expect("archive written");

    let archive = TraceArchive::open(&path).expect("archive opens");
    assert_eq!(archive.diffs.len(), 0);
    assert_eq!(archive.events.len(), 0);
    let timeline = Timeline::from_archive(&archive);
    assert!(timeline.is_empty(), "no frames recorded");
}

// --- The replayable §13 overlay (deep-inspect mode) ------------------------
//
// Plan pins (docs/m8-inspect-highlight.md §4 "Remaining gaps" — the two
// viewer bullets this milestone pays):
//
// * "Full-tree badges (the §13 'deep-inspect mode') land with the viewer;
//   live sessions composite interactive-only badges."
// * "The overlay does not ride `gwtrace/v1` archives yet (the plan is
//   present-time state; replayable overlays belong to the viewer)."
//
// The acceptance shape: the viewer replays the §13 overlay plan from the
// archived tree facts — every bounds-carrying node badged (full-tree, the
// deep-inspect rule: non-interactive roles included), the recorded focus
// outlined, and the emitted `trace.html` carrying the layer.
#[test]
fn the_deep_inspect_overlay_replays_from_the_archive() {
    use guiwright_viewer::overlay;

    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("trace.zip");
    let _file = record_archive(path.clone());
    let archive = TraceArchive::open(&path).expect("archive opens");
    let timeline = Timeline::from_archive(&archive);

    // Frame 3 resolves snapshot 1 (Window + Save): the deep-inspect plan
    // badges EVERY node with bounds — the Window (non-interactive) role
    // included, document order kept.
    let plan = overlay::replay_plan_at(&timeline, 3).expect("plan at 3");
    let texts: Vec<&str> = plan.badges.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["Window \u{201c}App\u{201d}", "Button \u{201c}Save\u{201d}"]
    );
    // The recorded focus is the outline (the archived stand-in for the
    // live hover outline): node 2's rect.
    assert_eq!(
        plan.hover.expect("focus outline"),
        guiwright_inspect::overlay::OverlayRect::new(16.0, 16.0, 136.0, 48.0)
    );

    // Frame 5 resolves snapshot 4 with id 2 removed: the greyed row earns
    // no badge (it is not on screen), the bounds-less node neither —
    // two badges, the new focus (id 3) outlined.
    let plan = overlay::replay_plan_at(&timeline, 5).expect("plan at 5");
    let texts: Vec<&str> = plan.badges.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "Window \u{201c}App\u{201d}",
            "Button \u{201c}Export\u{201d}"
        ],
        "the removed row and the bounds-less node earn no badge"
    );
    assert_eq!(
        plan.hover.expect("new focus"),
        guiwright_inspect::overlay::OverlayRect::new(16.0, 56.0, 136.0, 88.0)
    );

    // A frame before any snapshot answers honestly: no facts, no plan.
    assert!(overlay::replay_plan_at(&timeline, 0).is_none());

    // The emitted HTML carries the layer: per-snapshot overlay rows with
    // badge geometry and the archive's resolution as the canvas —
    // self-contained like the rest of the payload.
    let html = guiwright_viewer::render_html(&archive, &timeline);
    assert!(
        html.contains(r#""overlays":"#),
        "the layer rides: {html:.400}"
    );
    assert!(html.contains("Save All"), "badge text rides the payload");
    assert!(
        html.contains(r#""canvas":[1280.0,800.0]"#),
        "the archive resolution"
    );
    assert!(!html.contains("src=\"http"), "still self-contained");

    // The scrubber JS renders the layer (an SVG with the outline + badges).
    assert!(html.contains("renderOverlay"), "the layer is scrubbed");
}

// The live half rides the same seam: a live capture's accumulated
// snapshots replay the layer too (the live capture has no honest
// resolution — the canvas falls back to the overlay's own extent, the
// m18 record's honest-metadata rule).
#[test]
fn the_live_capture_replays_the_layer_too() {
    use guiwright_proto::schema::{NodeKeyV1, SnapshotNodeV1};

    let row = |id: u64, role: &str, name: &str, bounds: Option<[f64; 4]>| SnapshotNodeV1 {
        key: NodeKeyV1::AccessKit {
            tree: [0u8; 16],
            node: id,
        },
        role: role.into(),
        name: Some(name.into()),
        value: None,
        bounds,
        children: 0,
    };

    let mut live = guiwright_viewer::LiveTimeline::new("live-overlay/0.1");
    live.record_frame(2, 0x2);
    live.record_snapshot(
        &[
            row(1, "Window", "W", Some([0.0, 0.0, 300.0, 200.0])),
            row(2, "Button", "Save", Some([8.0, 8.0, 88.0, 40.0])),
        ],
        None,
    );
    let html = guiwright_viewer::render_live_html(&live);
    assert!(html.contains(r#""overlays":"#), "the live layer rides");
    assert!(html.contains("Button"), "the badge text");
    // No resolution to speak of: the fallback canvas is computed, not
    // invented.
    assert!(
        !html.contains(r#""canvas":[1280.0,800.0]"#),
        "no archive resolution"
    );
}
