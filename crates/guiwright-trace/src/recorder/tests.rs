//! Recorder integration tests (TDD for the §12 contract): archive layout,
//! hash dedup, eviction under the byte budget, backpressure drop counting
//! and idempotent stop.

use std::io::Read;

use crate::schema::{AdapterRecord, ClockCommand, ClockJump, TreeNode, TreeSnapshot};
use crate::{TraceOptions, TraceRecorder};

fn options(path: std::path::PathBuf) -> TraceOptions {
    TraceOptions {
        output_path: path,
        test: "login_flow".into(),
        app: "demo-app".into(),
        adapter: AdapterRecord {
            name: "egui".into(),
            framework: "egui".into(),
            framework_version: "0.37.0".into(),
        },
        seed: Some(7),
        resolution: (320, 200),
        scale: 2.0,
        frame_budget_bytes: 4 * 1024, // tiny budget → eviction is exercised
        ring_capacity: 64,
    }
}

fn png_bytes(seed: u8, size: usize) -> Vec<u8> {
    vec![seed; size]
}

#[test]
fn archive_has_the_documented_layout() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("guiwright-login_flow-1.zip");
    let mut rec = TraceRecorder::start(options(path.clone()));

    rec.event(1, "action", r#"click @ (12, 8)"#);
    rec.tree_snapshot(TreeSnapshot {
        frame_seq: 1,
        nodes: vec![TreeNode {
            id: 1,
            parent: None,
            role: "window".into(),
            label: None,
            value: None,
            bounds: Some([0.0, 0.0, 320.0, 200.0]),
        }],
        focus: Some(1),
    });
    rec.tree_diff(2, vec![2], vec![], vec![], false);
    rec.frame(1, 0x1000, png_bytes(1, 512));
    rec.frame(2, 0x1000, png_bytes(2, 512)); // same hash → deduped
    rec.frame(3, 0x1001, png_bytes(3, 512));
    rec.clock_jump(ClockJump {
        frame_seq: 3,
        command: ClockCommand::FastForward { micros: 1_000 },
        virtual_time_us: 1_000,
    });

    let file = rec.stop().expect("stop");
    assert_eq!(file.path, path);
    let stats = file.stats;
    assert_eq!(stats.events, 1);
    assert_eq!(stats.tree_snapshots, 1);
    assert_eq!(stats.tree_diffs, 1);
    assert_eq!(stats.frames, 2, "identical hash skipped");
    assert_eq!(stats.frames_deduped, 1);
    assert_eq!(stats.clock_jumps, 1);

    let bytes = std::fs::read(&path).expect("zip exists");
    assert_eq!(stats.archive_bytes as usize, bytes.len());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip");

    for expected in [
        "meta.json",
        "events.ndjson",
        "trees/1.bin",
        "trees/diff.bin",
        "frames/1.png",
        "frames/3.png",
        "clock.bin",
        "index.bin",
    ] {
        assert!(archive.by_name(expected).is_ok(), "missing {expected}");
    }
    assert!(
        archive.by_name("frames/2.png").is_err(),
        "deduped frame must not be archived"
    );

    let mut meta = String::new();
    archive
        .by_name("meta.json")
        .expect("meta")
        .read_to_string(&mut meta)
        .expect("read");
    let meta: serde_json::Value = serde_json::from_str(&meta).expect("meta json");
    assert_eq!(meta["schema"], "gwtrace/v1");
    assert_eq!(meta["test"], "login_flow");
    assert_eq!(meta["adapter"]["name"], "egui");
    assert_eq!(meta["resolution"][0], 320);
    assert_eq!(meta["resolution"][1], 200);
    assert_eq!(meta["scale"], 2.0);
    assert_eq!(meta["frame_codec"], "png");

    let mut events = String::new();
    archive
        .by_name("events.ndjson")
        .expect("events")
        .read_to_string(&mut events)
        .expect("read");
    let line: serde_json::Value =
        serde_json::from_str(events.lines().next().expect("one line")).expect("ndjson");
    assert_eq!(line["kind"], "action");
    assert_eq!(line["seq"], 1);
    assert_eq!(line["detail"], r#"click @ (12, 8)"#);

    // trees/<seq>.bin is postcard+LZ4 — decompress and decode.
    let mut snap = Vec::new();
    archive
        .by_name("trees/1.bin")
        .expect("snapshot")
        .read_to_end(&mut snap)
        .expect("read");
    let decoded: TreeSnapshot =
        postcard::from_bytes(&lz4_flex::decompress_size_prepended(&snap).expect("lz4"))
            .expect("postcard");
    assert_eq!(decoded.frame_seq, 1);
    assert_eq!(decoded.nodes[0].role, "window");

    let mut index = Vec::new();
    archive
        .by_name("index.bin")
        .expect("index")
        .read_to_end(&mut index)
        .expect("read");
    let index: Vec<crate::schema::FrameIndexEntry> =
        postcard::from_bytes(&index).expect("postcard");
    assert_eq!(index.len(), 2, "one row per archived frame");
    assert_eq!(index[0].seq, 1);
    assert_eq!(index[0].hash, 0x1000);
    assert_eq!(index[1].seq, 3);
}

#[test]
fn byte_budget_evicts_oldest_frames_first() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("evict.zip");
    let mut rec = TraceRecorder::start(options(path.clone()));

    // Budget is 4 KiB; each frame is 1 KiB → frames 1-4 evicted as 5..8
    // land (oldest-first), 5..8 survive. Hashes stay distinct here.
    for (seq, hash, seed) in [
        (1, 0x01u64, 1u8),
        (2, 0x02, 2),
        (3, 0x03, 3),
        (4, 0x04, 4),
        (5, 0x05, 5),
        (6, 0x06, 6),
        (7, 0x07, 7),
        (8, 0x08, 8),
    ] {
        rec.frame(seq, hash, png_bytes(seed, 1024));
    }
    let file = rec.stop().expect("stop");
    assert_eq!(file.stats.frames_evicted, 4, "{:?}", file.stats);

    let bytes = std::fs::read(&path).expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    assert!(archive.by_name("frames/1.png").is_err(), "oldest evicted");
    assert!(archive.by_name("frames/4.png").is_err(), "oldest evicted");
    assert!(archive.by_name("frames/5.png").is_ok(), "in budget kept");
    assert!(archive.by_name("frames/8.png").is_ok(), "newest kept");

    let mut meta = String::new();
    archive
        .by_name("meta.json")
        .expect("meta")
        .read_to_string(&mut meta)
        .expect("read");
    let meta: serde_json::Value = serde_json::from_str(&meta).expect("meta json");
    assert_eq!(meta["frames_evicted"], 4, "eviction recorded in meta");
}

#[test]
fn backpressure_drops_and_counts_never_blocks() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("flood.zip");
    let mut rec = TraceRecorder::start(options(path.clone()));

    // Flood far beyond the ring (capacity 64): every call must return
    // immediately; overflow is counted, and no record vanishes silently.
    let started = std::time::Instant::now();
    for i in 0..5_000u64 {
        rec.event(i, "protocol", "flood");
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "event() must never block: {:?}",
        started.elapsed()
    );
    let file = rec.stop().expect("stop");
    assert!(
        file.stats.events + file.stats.records_dropped >= 5_000,
        "no record lost silently: {:?}",
        file.stats
    );
    assert!(
        file.stats.records_dropped > 0,
        "flood must overflow the ring"
    );
}

#[test]
fn archive_is_intact_and_late_producers_are_safe() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("idem.zip");
    let (rec, sink) = TraceRecorder::start_with_sink(options(path.clone()));
    let mut rec = rec;
    rec.event(1, "action", "before stop");
    let file = rec.stop().expect("first stop");
    assert_eq!(file.stats.events, 1);

    // `stop(self)` consumes the recorder, so a second stop cannot even be
    // expressed; late producers (the sink lives on) must be no-ops, not
    // panics, and must not corrupt the finished archive.
    sink.log_line("\"late\"".into());

    let bytes = std::fs::read(&path).expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut events = String::new();
    archive
        .by_name("events.ndjson")
        .expect("events")
        .read_to_string(&mut events)
        .expect("read");
    assert_eq!(events.lines().count(), 1, "late records stay out");
}

#[test]
fn sink_logs_from_another_thread() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("sink.zip");
    let (rec, sink) = TraceRecorder::start_with_sink(options(path));
    let handle = std::thread::spawn(move || {
        sink.log_line(r#"{"level":"INFO","message":"from elsewhere"}"#.into());
    });
    handle.join().expect("producer thread");
    let file = rec.stop().expect("stop");
    assert_eq!(file.stats.log_lines, 1);
}
