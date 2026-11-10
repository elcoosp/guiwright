//! The `gwtrace/v1` schema (architecture §12): every record in the archive
//! is versioned and forward-compatible.
//!
//! Archive layout (`guiwright-<test>-<ts>.zip`):
//!
//! ```text
//! meta.json          app, adapter, versions, os, seed, resolution, scale, settings
//! events.ndjson      input events, semantic actions, protocol calls (amortized, seq'd)
//! trees/<seq>.bin    logical-tree snapshots (postcard+LZ4) every N frames
//! trees/diff.bin     incremental diffs between snapshots (postcard records)
//! frames/<seq>.png   framebuffer captures (hash-deduped: identical frames skipped)
//! logs.ndjson        `tracing` subscriber output (frameworks already use tracing)
//! clock.bin          virtual-time ledger (install/fast-forward history)
//! index.bin          frame_seq → offsets (O(1) random access)
//! ```
//!
//! Design notes (M1):
//! * The container is a plain **stored** (uncompressed) zip — compression
//!   lives in the payloads (postcard + LZ4), so the writer thread never
//!   spends CPU re-compressing already-compressed records.
//! * §10 specifies WebP for trace frames; M1 records the codec in
//!   [`TraceMeta::frame_codec`] (default `"png"`, lossless) so the viewer
//!   can route; the webp pipeline is a drop-in behind that tag.

use serde::{Deserialize, Serialize};

/// Schema identifier stored in `meta.json` (§12 "schema gwtrace/v1,
/// versioned & forward-compatible").
pub const SCHEMA: &str = "gwtrace/v1";

/// `meta.json` (§12).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceMeta {
    pub schema: String,
    /// Test/artifact name (nextest `NEXTEST_TEST_NAME` when present, §11).
    pub test: String,
    pub app: String,
    /// Adapter metadata ([`AdapterRecord`]: name, framework, version,
    /// caps) — §18 "`AdapterMeta` reports versions in traces".
    pub adapter: AdapterRecord,
    pub os: String,
    pub arch: String,
    pub seed: Option<u64>,
    pub resolution: (u32, u32),
    pub scale: f32,
    /// Frame codec tag (§10 wants WebP; M1 ships PNG, see module docs).
    pub frame_codec: String,
    /// Frames skipped by the byte budget (oldest-first eviction, §12).
    pub frames_evicted: u64,
    /// Records dropped because the SPSC ring was full (backpressure, §12).
    pub records_dropped: u64,
}

/// Adapter identity inside the trace (kept serde-only: the driver's
/// `AdapterMeta` is not `Serialize` and the trace crate must not depend on
/// the driver).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AdapterRecord {
    pub name: String,
    pub framework: String,
    pub framework_version: String,
}

/// One NDJSON line of `events.ndjson`: input events, semantic actions and
/// protocol calls, amortized and sequence-tagged (§12).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceEvent {
    /// Monotonic per-session sequence.
    pub seq: u64,
    /// Nanoseconds since the recorder started.
    pub elapsed_ns: u64,
    /// Frame the event belongs to (best effort — the adapter's last stamp).
    pub frame_seq: u64,
    /// `input` | `action` | `protocol` | `clock` (§12 vocabulary).
    pub kind: &'static str,
    /// Human payload (`click @ (12, 8)`, `fill("ada@…")`, …).
    pub detail: String,
}

/// A logical-tree snapshot record (`trees/<seq>.bin`, postcard+LZ4, §12).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreeSnapshot {
    /// Frame sequence the snapshot was taken at.
    pub frame_seq: u64,
    /// Node records (adapter-synthesized or AccessKit-consumer view).
    pub nodes: Vec<TreeNode>,
    pub focus: Option<u64>,
}

/// One node of a tree snapshot — the viewer's minimum vocabulary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreeNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    /// Window-relative physical px (AccessKit convention, §9).
    pub bounds: Option<[f64; 4]>,
}

/// One incremental diff record (`trees/diff.bin`, §12 "incremental diffs
/// between snapshots"). Added/updated/removed carry node ids; the viewer
/// resolves them against the nearest snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TreeDiffRecord {
    pub frame_seq: u64,
    pub added: Vec<u64>,
    pub updated: Vec<u64>,
    pub removed: Vec<u64>,
    pub focus_changed: bool,
}

/// One virtual-clock jump (`clock.bin`, §14 "clock.bin in traces replays
/// timing").
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClockJump {
    pub frame_seq: u64,
    pub command: ClockCommand,
    pub virtual_time_us: u64,
}

/// Virtual-clock command vocabulary (mirrors the driver's `ClockCmd`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClockCommand {
    InstallVirtual,
    FastForward { micros: u64 },
    RunFor { micros: u64 },
    Pause,
    Resume,
}

/// `index.bin` row: O(1) random access into `frames/` (§12).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct FrameIndexEntry {
    pub seq: u64,
    pub hash: u64,
    /// Byte offset inside the PNG payload (kept for the future streaming
    /// writer; M1 stores whole frames so this is always 0).
    pub offset: u64,
    pub len: u32,
}

/// Writer-thread statistics returned by [`crate::recorder::TraceRecorder::stop`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraceStats {
    pub events: u64,
    pub tree_snapshots: u64,
    pub tree_diffs: u64,
    pub frames: u64,
    pub frames_deduped: u64,
    pub frames_evicted: u64,
    pub records_dropped: u64,
    pub log_lines: u64,
    pub clock_jumps: u64,
    pub archive_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
        let bytes = postcard::to_allocvec(value).expect("serialize");
        postcard::from_bytes(&bytes).expect("deserialize")
    }

    #[test]
    fn postcard_roundtrips_every_record() {
        let snapshot = TreeSnapshot {
            frame_seq: 7,
            nodes: vec![TreeNode {
                id: 1,
                parent: None,
                role: "button".into(),
                label: Some("Save".into()),
                value: None,
                bounds: Some([0.0, 0.0, 40.0, 20.0]),
            }],
            focus: Some(1),
        };
        assert_eq!(roundtrip(&snapshot), snapshot);

        let diff = TreeDiffRecord {
            frame_seq: 8,
            added: vec![2, 3],
            updated: vec![1],
            removed: vec![9],
            focus_changed: true,
        };
        assert_eq!(roundtrip(&diff), diff);

        let jump = ClockJump {
            frame_seq: 9,
            command: ClockCommand::FastForward { micros: 30_000_000 },
            virtual_time_us: 30_000_000,
        };
        assert_eq!(roundtrip(&jump), jump);

        let entry = FrameIndexEntry {
            seq: 4,
            hash: 0xdead_beef,
            offset: 0,
            len: 128,
        };
        assert_eq!(roundtrip(&entry), entry);
    }

    #[test]
    fn meta_json_roundtrip() {
        let meta = TraceMeta {
            schema: SCHEMA.into(),
            test: "login_flow".into(),
            app: "demo".into(),
            adapter: AdapterRecord {
                name: "egui".into(),
                framework: "egui".into(),
                framework_version: "0.37.0".into(),
            },
            os: "linux".into(),
            arch: "x86_64".into(),
            seed: Some(42),
            resolution: (1280, 800),
            scale: 1.0,
            frame_codec: "png".into(),
            frames_evicted: 0,
            records_dropped: 0,
        };
        let text = serde_json::to_string(&meta).expect("json");
        let back: TraceMeta = serde_json::from_str(&text).expect("parse");
        assert_eq!(back, meta);
        assert!(text.contains("gwtrace/v1"));
    }
}
