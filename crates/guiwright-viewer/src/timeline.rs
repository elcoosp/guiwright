//! The timeline model (§12 "timeline scrubber (frames) + tree inspector at
//! any point, resolved from snapshot+diffs").
//!
//! One [`TimelineTick`] per **presented frame** — the union of the frame
//! index, the events that landed on it and the clock jumps that settled
//! there. [`Timeline::tree_at`] resolves the UI tree at an arbitrary
//! frame: the nearest snapshot ≤ target, minus every id a diff removed in
//! `(snapshot, target]`, with added/updated ids flagged as changes
//! pending their next snapshot (diffs carry ids, snapshots carry
//! payloads — that split is the §12 design).

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use guiwright_trace::schema::TreeNode;

use crate::archive::ArchiveEvent;

use crate::archive::TraceArchive;

/// One scrubber tick (one presented frame).
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineTick {
    pub frame_seq: u64,
    /// Frame hash from `index.bin` (the visual-change signal, §10).
    pub hash: Option<u64>,
    /// Events recorded against this frame, in seq order.
    pub events: Vec<ArchiveEvent>,
    /// Clock jump that settled at this frame (last wins, §12 `clock.bin`).
    pub clock_command: Option<String>,
    /// `true` when a frame payload is stored for this tick.
    pub has_png: bool,
}

/// The resolved tree view at a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeView {
    pub frame_seq: u64,
    /// Frame the payload came from (≤ `frame_seq`).
    pub snapshot_frame: Option<u64>,
    pub nodes: Vec<TreeViewNode>,
    /// Ids added/updated in `(snapshot, target]` — shown as pending
    /// changes until their next snapshot (diffs carry ids only).
    pub pending_changes: Vec<u64>,
    pub focus: Option<u64>,
}

/// One node of the resolved view.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeViewNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    pub bounds: Option<[f64; 4]>,
    /// Set when the node was removed in `(snapshot, target]` — the viewer
    /// greys it out instead of dropping it silently.
    pub removed: bool,
}

/// The scrubber model built from an archive.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub meta_test: String,
    pub ticks: Vec<TimelineTick>,
    /// Snapshot payloads by frame (the tree inspector's source).
    snapshots: BTreeMap<u64, Vec<TreeNode>>,
    /// Focus per snapshot.
    snapshot_focus: BTreeMap<u64, Option<u64>>,
    /// Diffs in frame order (applied between snapshots).
    diffs: Vec<guiwright_trace::schema::TreeDiffRecord>,
}

impl Timeline {
    pub fn from_archive(archive: &TraceArchive) -> Self {
        // Events grouped by their frame stamp.
        let mut events_by_frame: BTreeMap<u64, Vec<ArchiveEvent>> = BTreeMap::new();
        for ev in &archive.events {
            events_by_frame
                .entry(ev.frame_seq)
                .or_default()
                .push(ev.clone());
        }
        // Clock commands per frame (last wins at a frame).
        let mut clock_by_frame: BTreeMap<u64, String> = BTreeMap::new();
        for jump in &archive.clock {
            clock_by_frame.insert(jump.frame_seq, format!("{:?}", jump.command));
        }
        let index_hashes: BTreeMap<u64, u64> = archive
            .frame_index
            .iter()
            .map(|e| (e.seq, e.hash))
            .collect();
        let stored_png: BTreeSet<u64> = archive.frames_png_keys();

        // The scrubber axis: every frame the archive mentions.
        let mut all_frames: BTreeSet<u64> = BTreeSet::new();
        all_frames.extend(index_hashes.keys().copied());
        all_frames.extend(events_by_frame.keys().copied());
        all_frames.extend(clock_by_frame.keys().copied());
        all_frames.extend(archive.snapshots.keys().copied());
        all_frames.extend(stored_png.iter().copied());
        // Diff frames belong on the axis too — a removal changes the tree
        // even when the frame produced no new payload.
        all_frames.extend(archive.diffs.iter().map(|d| d.frame_seq));

        let ticks = all_frames
            .into_iter()
            .map(|seq| TimelineTick {
                frame_seq: seq,
                hash: index_hashes.get(&seq).copied(),
                events: events_by_frame.remove(&seq).unwrap_or_default(),
                clock_command: clock_by_frame.get(&seq).cloned(),
                has_png: stored_png.contains(&seq),
            })
            .collect();

        let mut snapshots = BTreeMap::new();
        let mut snapshot_focus = BTreeMap::new();
        for (seq, snap) in &archive.snapshots {
            snapshots.insert(*seq, snap.nodes.clone());
            snapshot_focus.insert(*seq, snap.focus);
        }

        Self {
            meta_test: archive.meta.test.clone(),
            ticks,
            snapshots,
            snapshot_focus,
            diffs: archive.diffs.clone(),
        }
    }

    pub fn len(&self) -> usize {
        self.ticks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }

    /// The nearest snapshot at or before `target` (binary search).
    fn nearest_snapshot(&self, target: u64) -> Option<u64> {
        self.snapshots
            .range(..=target)
            .next_back()
            .map(|(seq, _)| *seq)
    }

    /// Resolve the UI tree at `target` (§12 "resolved from snapshot+diffs").
    pub fn tree_at(&self, target: u64) -> Option<TreeView> {
        let snap_seq = self.nearest_snapshot(target)?;
        let nodes = self.snapshots.get(&snap_seq)?.clone();

        // Diffs strictly inside (snapshot, target].
        let mut removed: BTreeSet<u64> = BTreeSet::new();
        let mut pending: Vec<u64> = Vec::new();
        let focus = self.snapshot_focus.get(&snap_seq).copied().flatten();
        for diff in &self.diffs {
            if diff.frame_seq <= snap_seq || diff.frame_seq > target {
                continue;
            }
            removed.extend(diff.removed.iter().copied());
            pending.extend(diff.added.iter().chain(diff.updated.iter()).copied());
            // `focus_changed` alone carries no payload — the new focus id
            // lands with the next snapshot (§12 diff record shape).
        }

        let view = nodes
            .into_iter()
            .map(|node| {
                let is_removed = removed.contains(&node.id);
                if is_removed {
                    // Keep the row (greyed out) so the inspector shows the
                    // disappearance instead of a silent gap.
                    TreeViewNode {
                        removed: true,
                        ..TreeViewNode::from(node)
                    }
                } else {
                    TreeViewNode::from(node)
                }
            })
            .collect();

        Some(TreeView {
            frame_seq: target,
            snapshot_frame: Some(snap_seq),
            nodes: view,
            pending_changes: pending,
            focus,
        })
    }

    /// Snapshot payloads for the HTML embed (frame → nodes).
    pub fn snapshot_payloads(&self) -> &BTreeMap<u64, Vec<TreeNode>> {
        &self.snapshots
    }
}

impl From<TreeNode> for TreeViewNode {
    fn from(node: TreeNode) -> Self {
        Self {
            id: node.id,
            parent: node.parent,
            role: node.role,
            label: node.label,
            value: node.value,
            bounds: node.bounds,
            removed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_trace::schema::{AdapterRecord, TraceMeta};

    fn empty_meta() -> TraceMeta {
        TraceMeta {
            schema: guiwright_trace::schema::SCHEMA.into(),
            test: "demo".into(),
            app: "demo".into(),
            adapter: AdapterRecord {
                name: "mock".into(),
                framework: "fixture".into(),
                framework_version: "0.1.0".into(),
            },
            os: "linux".into(),
            arch: "x86_64".into(),
            seed: None,
            resolution: (1280, 800),
            scale: 1.0,
            frame_codec: "png".into(),
            frames_evicted: 0,
            records_dropped: 0,
        }
    }

    fn snap(frame: u64, ids: &[u64], focus: Option<u64>) -> guiwright_trace::schema::TreeSnapshot {
        guiwright_trace::schema::TreeSnapshot {
            frame_seq: frame,
            nodes: ids
                .iter()
                .map(|id| TreeNode {
                    id: *id,
                    parent: Some(1),
                    role: "Button".into(),
                    label: Some(format!("node-{id}")),
                    value: None,
                    bounds: Some([0.0, 0.0, 10.0, 10.0]),
                })
                .collect(),
            focus,
        }
    }

    fn diff(
        frame: u64,
        added: Vec<u64>,
        updated: Vec<u64>,
        removed: Vec<u64>,
    ) -> guiwright_trace::schema::TreeDiffRecord {
        guiwright_trace::schema::TreeDiffRecord {
            frame_seq: frame,
            added,
            updated,
            removed,
            focus_changed: false,
        }
    }

    #[test]
    fn tree_at_resolves_snapshot_minus_removals() {
        let archive = TraceArchive::from_parts(
            "memory".into(),
            empty_meta(),
            crate::archive::ArchiveParts {
                events: vec![],
                logs: vec![],
                snapshots: [
                    (2u64, snap(2, &[1, 2, 3], Some(2))),
                    (10u64, snap(10, &[1, 2, 5], Some(5))),
                ]
                .into_iter()
                .collect(),
                diffs: vec![
                    diff(4, vec![], vec![], vec![3]),
                    diff(6, vec![5], vec![], vec![]),
                ],
                frame_index: vec![],
                clock: vec![],
                frames: BTreeMap::new(),
            },
        );
        let timeline = Timeline::from_archive(&archive);

        // Frame 3 → snapshot 2, nothing removed yet.
        let view = timeline.tree_at(3).expect("view");
        assert_eq!(view.snapshot_frame, Some(2));
        assert_eq!(view.nodes.len(), 3);
        assert!(!view.nodes.iter().any(|n| n.removed));
        assert!(view.pending_changes.is_empty());

        // Frame 5 → snapshot 2 + removal of id 3 at frame 4.
        let view = timeline.tree_at(5).expect("view");
        let gone = view.nodes.iter().find(|n| n.id == 3).expect("row kept");
        assert!(gone.removed, "removed ids are greyed, not dropped");

        // Frame 9 → snapshot 2 + id 3 removed + id 5 pending: diffs carry
        // ids only, the payload lands with snapshot 10 (§12 design).
        let view = timeline.tree_at(9).expect("view");
        assert!(
            view.pending_changes.contains(&5),
            "{:?}",
            view.pending_changes
        );
        assert!(
            !view.nodes.iter().any(|n| n.id == 5),
            "no row before the payload snapshot"
        );

        // Frame 12 → snapshot 10 (fresh payload, no pending).
        let view = timeline.tree_at(12).expect("view");
        assert_eq!(view.snapshot_frame, Some(10));
        assert!(view.pending_changes.is_empty());
        assert_eq!(view.focus, Some(5));
    }

    #[test]
    fn before_any_snapshot_there_is_no_tree() {
        let archive = TraceArchive::from_parts(
            "memory".into(),
            empty_meta(),
            crate::archive::ArchiveParts {
                events: vec![],
                logs: vec![],
                snapshots: [(5u64, snap(5, &[1], None))].into_iter().collect(),
                diffs: vec![],
                frame_index: vec![],
                clock: vec![],
                frames: BTreeMap::new(),
            },
        );
        let timeline = Timeline::from_archive(&archive);
        assert!(timeline.tree_at(4).is_none(), "before the first snapshot");
        assert!(timeline.tree_at(5).is_some());
    }

    #[test]
    fn ticks_are_the_sorted_frame_union() {
        let archive = TraceArchive::from_parts(
            "memory".into(),
            empty_meta(),
            crate::archive::ArchiveParts {
                events: vec![
                    ArchiveEvent {
                        seq: 1,
                        elapsed_ns: 0,
                        frame_seq: 7,
                        kind: "click".into(),
                        detail: "click".into(),
                    },
                    ArchiveEvent {
                        seq: 2,
                        elapsed_ns: 10,
                        frame_seq: 8,
                        kind: "clock".into(),
                        detail: "FastForward".into(),
                    },
                ],
                logs: vec![],
                snapshots: [(8u64, snap(8, &[1], None))].into_iter().collect(),
                diffs: vec![],
                frame_index: vec![
                    guiwright_trace::schema::FrameIndexEntry {
                        seq: 7,
                        hash: 0xAA,
                        offset: 0,
                        len: 0,
                    },
                    guiwright_trace::schema::FrameIndexEntry {
                        seq: 9,
                        hash: 0xAB,
                        offset: 0,
                        len: 0,
                    },
                ],
                clock: vec![],
                frames: BTreeMap::new(),
            },
        );
        let timeline = Timeline::from_archive(&archive);
        let seqs: Vec<u64> = timeline.ticks.iter().map(|t| t.frame_seq).collect();
        assert_eq!(seqs, vec![7, 8, 9], "union, sorted, deduped");
        assert_eq!(timeline.ticks[0].events.len(), 1);
        assert_eq!(timeline.ticks[0].hash, Some(0xAA));
        assert_eq!(
            timeline.ticks[1].events.len(),
            1,
            "the clock event rides frame 8"
        );
        assert_eq!(timeline.ticks[2].events.len(), 0, "frame 9 carries none");
    }
}
