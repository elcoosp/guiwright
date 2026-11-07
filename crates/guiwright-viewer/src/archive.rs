//! The `gwtrace/v1` archive reader (§12 archive layout).
//!
//! Everything is version-checked through `meta.json.schema` before any
//! record is parsed. The tricky stream is `trees/diff.bin`: the M1 writer
//! concatenated raw LZ4 blobs (`compress_prepend_size` output = 4-byte LE
//! uncompressed size + LZ4 block), so blob boundaries are recovered with a
//! smallest-valid-prefix scan — a truncated LZ4 block never decodes to its
//! declared size, and each candidate must fully parse as one
//! `TreeDiffRecord` (postcard `take_from_bytes` rejects trailing bytes),
//! which makes the first successful prefix the true blob end.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use guiwright_trace::ClockJump;
use guiwright_trace::schema::{FrameIndexEntry, SCHEMA, TraceMeta, TreeDiffRecord, TreeSnapshot};

/// The `events.ndjson` row in owned form. The trace schema stores `kind`
/// as `&'static str` (write-side vocabulary); the reader owns it because
/// NDJSON text carries no static lifetime.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct ArchiveEvent {
    pub seq: u64,
    pub elapsed_ns: u64,
    pub frame_seq: u64,
    pub kind: String,
    pub detail: String,
}

/// The record collections of an archive (bundled so constructors stay
/// below the argument-count lint).
pub(crate) struct ArchiveParts {
    pub events: Vec<ArchiveEvent>,
    pub logs: Vec<String>,
    pub snapshots: BTreeMap<u64, TreeSnapshot>,
    pub diffs: Vec<TreeDiffRecord>,
    pub frame_index: Vec<FrameIndexEntry>,
    pub clock: Vec<ClockJump>,
    pub frames: BTreeMap<u64, Vec<u8>>,
}

/// The opened archive: everything the timeline needs, in memory.
#[derive(Debug)]
pub struct TraceArchive {
    pub path: std::path::PathBuf,
    pub meta: TraceMeta,
    pub events: Vec<ArchiveEvent>,
    pub logs: Vec<String>,
    /// `trees/<seq>.bin` — frame_seq → snapshot (postcard+LZ4).
    pub snapshots: BTreeMap<u64, TreeSnapshot>,
    /// `trees/diff.bin` — sequential diffs (recovered blob boundaries).
    pub diffs: Vec<TreeDiffRecord>,
    /// `index.bin` — frame_seq → offsets (O(1) random access, §12).
    pub frame_index: Vec<FrameIndexEntry>,
    /// `clock.bin` — virtual-time ledger (§14).
    pub clock: Vec<ClockJump>,
    /// Stored PNG payloads by frame seq (frames are hash-deduped upstream).
    frames: BTreeMap<u64, Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ViewerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(String),
    #[error("not a gwtrace archive: schema is `{0}` (expected `{SCHEMA}`)")]
    Schema(String),
    #[error("corrupt archive: {0}")]
    Corrupt(String),
}

impl TraceArchive {
    /// Open and fully parse a `gwtrace/v1` archive.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ViewerError> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path)?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| ViewerError::Zip(format!("{}: {e}", path.display())))?;

        let meta_text = read_string(&mut zip, "meta.json")?;
        // Schema gate first (§12 "versioned & forward-compatible"): reject
        // foreign archives before touching any record.
        let probe: serde_json::Value = serde_json::from_str(&meta_text)
            .map_err(|e| ViewerError::Corrupt(format!("meta.json: {e}")))?;
        let schema = probe
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if schema != SCHEMA {
            return Err(ViewerError::Schema(schema));
        }
        let meta: TraceMeta = serde_json::from_str(&meta_text)
            .map_err(|e| ViewerError::Corrupt(format!("meta.json: {e}")))?;

        let events = parse_ndjson(&read_string(&mut zip, "events.ndjson")?)
            .map_err(|e| ViewerError::Corrupt(format!("events.ndjson: {e}")))?;
        let logs = read_string(&mut zip, "logs.ndjson")?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();

        let mut snapshots = BTreeMap::new();
        for name in zip
            .file_names()
            .filter_map(Result::ok)
            .map(|n| n.into_owned().to_string())
            .collect::<Vec<_>>()
        {
            let Some(rest) = name.strip_prefix("trees/") else {
                continue;
            };
            let Some(seq_str) = rest.strip_suffix(".bin") else {
                continue;
            };
            let Ok(seq) = seq_str.parse::<u64>() else {
                continue;
            };
            let bytes = read_bytes(&mut zip, &name)?;
            let decompressed = lz4_flex::decompress_size_prepended(&bytes)
                .map_err(|e| ViewerError::Corrupt(format!("trees/{seq}.bin: {e}")))?;
            let snapshot: TreeSnapshot = postcard::from_bytes(&decompressed)
                .map_err(|e| ViewerError::Corrupt(format!("trees/{seq}.bin: {e}")))?;
            snapshots.insert(seq, snapshot);
        }

        let diffs = match read_bytes(&mut zip, "trees/diff.bin") {
            Ok(bytes) => parse_diff_stream(&bytes)?,
            Err(_) => Vec::new(),
        };

        let frame_index = match read_bytes(&mut zip, "index.bin") {
            Ok(bytes) => postcard::from_bytes(&bytes)
                .map_err(|e| ViewerError::Corrupt(format!("index.bin: {e}")))?,
            Err(_) => Vec::new(),
        };

        let clock = match read_bytes(&mut zip, "clock.bin") {
            Ok(bytes) => postcard::from_bytes(&bytes)
                .map_err(|e| ViewerError::Corrupt(format!("clock.bin: {e}")))?,
            Err(_) => Vec::new(),
        };

        let mut frames = BTreeMap::new();
        for name in zip
            .file_names()
            .filter_map(Result::ok)
            .map(|n| n.into_owned().to_string())
            .collect::<Vec<_>>()
        {
            let Some(rest) = name.strip_prefix("frames/") else {
                continue;
            };
            let Some(seq_str) = rest.strip_suffix(".png") else {
                continue;
            };
            let Ok(seq) = seq_str.parse::<u64>() else {
                continue;
            };
            frames.insert(seq, read_bytes(&mut zip, &name)?);
        }

        Ok(Self::from_parts(
            path,
            meta,
            ArchiveParts {
                events,
                logs,
                snapshots,
                diffs,
                frame_index,
                clock,
                frames,
            },
        ))
    }

    /// Crate-internal assembly (shared by [`Self::open`] and the timeline
    /// tests).
    pub(crate) fn from_parts(
        path: std::path::PathBuf,
        meta: TraceMeta,
        parts: ArchiveParts,
    ) -> Self {
        Self {
            path,
            meta,
            events: parts.events,
            logs: parts.logs,
            snapshots: parts.snapshots,
            diffs: parts.diffs,
            frame_index: parts.frame_index,
            clock: parts.clock,
            frames: parts.frames,
        }
    }

    /// Frame seqs that carry a stored PNG payload.
    pub fn frames_png_keys(&self) -> std::collections::BTreeSet<u64> {
        self.frames.keys().copied().collect()
    }

    /// The stored PNG payload of a frame, if present.
    pub fn frame_png(&self, seq: u64) -> Option<&[u8]> {
        self.frames.get(&seq).map(Vec::as_slice)
    }

    /// The last presented frame's payload (the scrubber's end state).
    pub fn last_frame_png(&self) -> Option<(u64, &[u8])> {
        self.frames
            .iter()
            .next_back()
            .map(|(seq, png)| (*seq, png.as_slice()))
    }
}

fn read_string(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<String, ViewerError> {
    let bytes = read_bytes(zip, name)?;
    String::from_utf8(bytes).map_err(|e| ViewerError::Corrupt(format!("{name}: {e}")))
}

fn read_bytes(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Vec<u8>, ViewerError> {
    let mut entry = zip
        .by_name(name)
        .map_err(|e| ViewerError::Zip(format!("{name}: {e}")))?;
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut out)
        .map_err(|e| ViewerError::Zip(format!("{name}: {e}")))?;
    Ok(out)
}

fn parse_ndjson<T: serde::de::DeserializeOwned>(text: &str) -> Result<Vec<T>, serde_json::Error> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

/// Recover the sequential LZ4 blob boundaries of `trees/diff.bin`
/// (smallest-valid-prefix scan, validated by a complete postcard record).
fn parse_diff_stream(bytes: &[u8]) -> Result<Vec<TreeDiffRecord>, ViewerError> {
    let mut diffs = Vec::new();
    let mut offset = 0usize;
    while offset + 4 <= bytes.len() {
        // Declared uncompressed size (LE u32 prefix, compress_prepend_size).
        let declared = u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("4 bytes checked"),
        ) as usize;
        if declared == 0 || declared > bytes.len() * 8 + 64 {
            return Err(ViewerError::Corrupt(format!(
                "trees/diff.bin: implausible record size {declared} at offset {offset}"
            )));
        }
        // Find the smallest prefix that decompresses to exactly one
        // complete TreeDiffRecord.
        let mut found: Option<(usize, TreeDiffRecord)> = None;
        let mut candidate = 4usize;
        while candidate <= bytes.len() - offset {
            let Ok(decompressed) =
                lz4_flex::decompress_size_prepended(&bytes[offset..offset + candidate])
            else {
                candidate += 1;
                continue;
            };
            if decompressed.len() != declared {
                candidate += 1;
                continue;
            }
            match postcard::take_from_bytes::<TreeDiffRecord>(&decompressed) {
                Ok((record, rest)) => {
                    if rest.is_empty() {
                        found = Some((candidate, record));
                        break;
                    }
                    candidate += 1;
                }
                Err(_) => candidate += 1,
            }
        }
        match found {
            Some((len, record)) => {
                diffs.push(record);
                offset += len;
            }
            None => {
                return Err(ViewerError::Corrupt(format!(
                    "trees/diff.bin: no valid record boundary at offset {offset}"
                )));
            }
        }
    }
    Ok(diffs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_trace::schema::TreeNode;

    /// Direct stream construction: two concatenated LZ4 blobs must yield
    /// two records (the reader never sees the writer's bookkeeping).
    #[test]
    fn diff_stream_boundaries_are_recovered() {
        let records = vec![
            TreeDiffRecord {
                frame_seq: 3,
                added: vec![7],
                updated: vec![2, 4],
                removed: vec![],
                focus_changed: false,
            },
            TreeDiffRecord {
                frame_seq: 9,
                added: vec![],
                updated: vec![],
                removed: vec![7],
                focus_changed: true,
            },
        ];
        let mut stream = Vec::new();
        for r in &records {
            stream.extend(lz4_flex::compress_prepend_size(
                &postcard::to_allocvec(r).expect("postcard"),
            ));
        }
        let parsed = parse_diff_stream(&stream).expect("parsed");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].frame_seq, 3);
        assert_eq!(parsed[1].removed, vec![7]);
        assert!(parsed[1].focus_changed);
    }

    #[test]
    fn compressible_records_do_not_confuse_the_scan() {
        // Long runs compress hard — the compressed length is much shorter
        // than the declared size; the scan must still land exactly.
        let records = vec![
            TreeDiffRecord {
                frame_seq: 1,
                added: (0..64).map(|i| i * 3).collect(),
                updated: vec![],
                removed: vec![],
                focus_changed: false,
            },
            TreeDiffRecord {
                frame_seq: 2,
                added: vec![],
                updated: vec![1],
                removed: vec![],
                focus_changed: false,
            },
        ];
        let mut stream = Vec::new();
        for r in &records {
            stream.extend(lz4_flex::compress_prepend_size(
                &postcard::to_allocvec(r).expect("postcard"),
            ));
        }
        let parsed = parse_diff_stream(&stream).expect("parsed");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].added.len(), 64);
    }

    #[test]
    fn garbage_stream_is_a_clean_error() {
        let stream = vec![0xFF; 64];
        let err = parse_diff_stream(&stream).expect_err("no boundaries");
        assert!(matches!(err, ViewerError::Corrupt(_)), "{err}");
    }

    #[test]
    fn snapshot_payload_shape() {
        let snapshot = TreeSnapshot {
            frame_seq: 4,
            nodes: vec![TreeNode {
                id: 1,
                parent: None,
                role: "Window".into(),
                label: None,
                value: None,
                bounds: None,
            }],
            focus: Some(1),
        };
        let bytes = lz4_flex::compress_prepend_size(&postcard::to_allocvec(&snapshot).unwrap());
        let back: TreeSnapshot =
            postcard::from_bytes(&lz4_flex::decompress_size_prepended(&bytes).unwrap()).unwrap();
        assert_eq!(back, snapshot);
    }
}
