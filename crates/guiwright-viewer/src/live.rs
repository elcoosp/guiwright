//! The viewer live mode (architecture §12 "live", the §16 v0.6 row, M18):
//! watch a **running** GWP session instead of replaying a finished
//! `gwtrace/v1` archive.
//!
//! Two halves:
//!
//! * [`LiveTimeline`] — the sync accumulation model. Frame ticks, events,
//!   logs and wire tree dumps land in [`ArchiveParts`]-shaped buffers and
//!   [`LiveTimeline::timeline`] hands back the same scrubber
//!   [`Timeline`](crate::timeline::Timeline) the archive path builds —
//!   snapshot+diffs resolution included, one code path for both modes
//!   (§12 "tree inspector at any point, resolved from snapshot+diffs").
//! * [`LiveFeed`] — the async half: any [`guiwright_proto::ClientConn`]
//!   (the §7 client seam) attaches to a running session, subscribes to
//!   the M12 event vocabulary, fetches a full [`ResponseV1::TreeDump`]
//!   on attach and after every `TreeChanged`, and stamps frames from
//!   `FramePresented`. [`LiveFeed::pump`] applies one event per call and
//!   reports what changed; a quiet window is a `None`, never an error.
//!
//! The wire dump rows rebuild their parent links from the DFS order +
//! children counts ([`SnapshotNodeV1::children`]' documented contract),
//! then ride as [`TreeNode`]s — the viewer's minimum vocabulary, shared
//! with the archive.
//!
//! ```ignore
//! let (handle, connect) = GwpServerHandle::spawn_inprocess(make_adapter, seams);
//! let client = guiwright_proto::DuplexClient::connect(&connect).await?;
//! let mut feed = LiveFeed::new(client);
//! let info = feed.attach("viewer-live/0.1").await?;   // handshake + subscribe + initial dump
//! let change = feed.pump(Duration::from_secs(1)).await?;   // one applied event (None = quiet)
//! let view = feed.tree_at(feed.current_frame());           // resolved tree, archive semantics
//! handle.shutdown();
//! ```

use std::collections::BTreeMap;
use std::time::Duration;

use guiwright_proto::ClientConn;
use guiwright_proto::GwpError;
use guiwright_proto::schema::{
    AdapterInfoV1, EventKindV1, EventV1, GwpEvent, NodeKeyV1, RequestV1, ResponseV1,
    SnapshotNodeV1, SnapshotV1,
};
use guiwright_trace::schema::{FrameIndexEntry, TreeNode, TreeSnapshot};

use crate::ViewerError;
use crate::archive::{ArchiveEvent, ArchiveParts};
use crate::timeline::{Timeline, TreeView};

/// What [`LiveFeed::pump`] applied — one event per call, in the session's
/// own order. `TreeChanged` carries the frame the post-sync dump landed
/// on; `FramePresented` carries the presented frame's seq + hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveChange {
    /// The session's tree changed; the feed auto-synced a fresh dump.
    TreeChanged { frame: u64 },
    /// A frame was presented (the §10 visual-change hash rides along).
    FramePresented { frame: u64, hash: u64 },
    /// A log record streamed past (stored into the timeline's log buffer).
    Log,
}

/// The attach report: the session's adapter facts as the `Welcome`
/// carried them (§7).
#[derive(Clone, Debug)]
pub struct LiveInfo {
    pub adapter: AdapterInfoV1,
}

/// The sync accumulation model (§12): the live half of the archive
/// timeline. Records are kept in the archive's own vocabulary
/// ([`ArchiveEvent`], [`TreeSnapshot`], [`FrameIndexEntry`]) so
/// [`LiveTimeline::timeline`] can hand the exact scrubber model the
/// archive path builds.
///
/// The model is deliberately dumb about ordering: the live feed stamps
/// records against its current frame (the last `FramePresented`, or the
/// frame of the first dump), which is the same "records land on the
/// frame they happened on" discipline the trace recorder uses.
#[derive(Debug)]
pub struct LiveTimeline {
    test: String,
    events: Vec<ArchiveEvent>,
    logs: Vec<String>,
    snapshots: BTreeMap<u64, TreeSnapshot>,
    frame_index: Vec<FrameIndexEntry>,
    current_frame: u64,
    event_seq: u64,
}

impl LiveTimeline {
    /// A model labeled `test` (what `TraceMeta.test` would be for the
    /// archive of the same session).
    pub fn new(test: impl Into<String>) -> Self {
        Self {
            test: test.into(),
            events: Vec::new(),
            logs: Vec::new(),
            snapshots: BTreeMap::new(),
            frame_index: Vec::new(),
            current_frame: 0,
            event_seq: 0,
        }
    }

    /// The label [`LiveTimeline::timeline`] bakes into the scrubber model.
    pub fn set_test(&mut self, test: impl Into<String>) {
        self.test = test.into();
    }

    /// The frame the next record will stamp against (the last recorded
    /// frame, or 0 before the first one).
    pub fn current_frame(&self) -> u64 {
        self.current_frame
    }

    /// Record a presented frame (the §10 visual-change hash): it joins
    /// the scrubber axis and becomes the stamp for subsequent records.
    pub fn record_frame(&mut self, seq: u64, hash: u64) {
        self.frame_index.push(FrameIndexEntry {
            seq,
            hash,
            // The live path stores no PNG payloads; the index row still
            // carries the frame on the axis (offset/len stay zero).
            offset: 0,
            len: 0,
        });
        self.current_frame = self.current_frame.max(seq);
    }

    /// Record a session event (the §12 `events.ndjson` vocabulary —
    /// click, clock, action details) stamped at the current frame.
    pub fn record_event(&mut self, kind: impl Into<String>, detail: impl Into<String>) {
        self.event_seq += 1;
        self.events.push(ArchiveEvent {
            seq: self.event_seq,
            elapsed_ns: 0,
            frame_seq: self.current_frame,
            kind: kind.into(),
            detail: detail.into(),
        });
    }

    /// Record a log line (the §12 `logs.ndjson` vocabulary).
    pub fn record_log(&mut self, line: impl Into<String>) {
        self.logs.push(line.into());
    }

    /// Record a wire tree dump at the current frame. The dump's DFS rows
    /// rebuild parent links from the documented
    /// [`SnapshotNodeV1::children`] contract (depth-first order + per-row
    /// child counts), and the rows land as [`TreeNode`]s — the same
    /// payload shape the archive's `trees/<seq>.bin` stores.
    ///
    /// Returns the frame the snapshot landed on.
    pub fn record_snapshot(&mut self, rows: &[SnapshotNodeV1], focus: Option<&NodeKeyV1>) -> u64 {
        let nodes = rows_to_nodes(rows);
        let focus_id = focus.map(node_key_id);
        self.snapshots.insert(
            self.current_frame,
            TreeSnapshot {
                frame_seq: self.current_frame,
                nodes,
                focus: focus_id,
            },
        );
        self.current_frame
    }

    /// How many wire dumps have landed (one per attach + per sync).
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// The scrubber model over everything recorded so far. Rebuilt per
    /// call — live volumes are small (one dump per change), and the
    /// archive path owns the tick-union + resolution logic; cache the
    /// result if you drive it across a large tree.
    pub fn timeline(&self) -> Timeline {
        Timeline::from_parts(
            self.test.clone(),
            ArchiveParts {
                events: self.events.clone(),
                logs: self.logs.clone(),
                snapshots: self.snapshots.clone(),
                diffs: Vec::new(),
                frame_index: self.frame_index.clone(),
                clock: Vec::new(),
                frames: BTreeMap::new(),
            },
        )
    }

    /// The resolved tree view at `frame` (archive semantics: the nearest
    /// snapshot at or before the frame).
    pub fn tree_at(&self, frame: u64) -> Option<TreeView> {
        self.timeline().tree_at(frame)
    }
}

/// The live feed over any connected GWP client (§7's [`ClientConn`]
/// seam, M18): attach, subscribe, sync, pump. One feed, one connection —
/// the same §11 discipline the driver's connections follow.
#[derive(Debug)]
pub struct LiveFeed<C: ClientConn> {
    client: C,
    timeline: LiveTimeline,
}

impl<C: ClientConn> LiveFeed<C> {
    /// Wrap a connected (not yet handshaked) client.
    pub fn new(client: C) -> Self {
        Self {
            client,
            timeline: LiveTimeline::new(String::new()),
        }
    }

    /// Handshake as `agent`, subscribe to the live vocabulary
    /// (`TreeChanged` + `FramePresented` — the M12 kinds the timeline
    /// consumes), and fetch the initial dump so [`LiveFeed::tree_at`]
    /// answers right after attach. The dump stamps at the feed's origin
    /// frame (0) until the first `FramePresented` advances the axis.
    pub async fn attach(&mut self, agent: &str) -> Result<LiveInfo, ViewerError> {
        let adapter = self.client.handshake(agent).await?;
        self.client
            .subscribe(&[EventKindV1::TreeChanged, EventKindV1::FramePresented])
            .await?;
        self.timeline.set_test(agent);
        self.sync_tree().await?;
        Ok(LiveInfo { adapter })
    }

    /// Fetch a full dump (`Snapshot::Full` → `TreeDump`) and record it at
    /// the current frame. Returns the frame the dump landed on.
    pub async fn sync_tree(&mut self) -> Result<u64, ViewerError> {
        let response = self
            .client
            .request(RequestV1::Snapshot(SnapshotV1::Full))
            .await?;
        let ResponseV1::TreeDump { focus, nodes, .. } = response else {
            return Err(GwpError::Protocol(format!(
                "live sync expected a TreeDump response, got {}",
                response_kind(&response)
            ))
            .into());
        };
        let frame = self.timeline.record_snapshot(&nodes, focus.as_ref());
        Ok(frame)
    }

    /// Wait up to `timeout` for the next pushed event and apply it:
    /// `TreeChanged` triggers an auto-sync (a fresh dump at the current
    /// frame), `FramePresented` advances the axis, `Log` joins the log
    /// buffer. A quiet window answers `Ok(None)` — pacing is the
    /// caller's, exactly like the wire client's `next_event`.
    pub async fn pump(&mut self, timeout: Duration) -> Result<Option<LiveChange>, ViewerError> {
        let event = match tokio::time::timeout(timeout, self.client.next_event()).await {
            Ok(Ok(event)) => event,
            Ok(Err(err)) => return Err(err.into()),
            // The window closed with nothing pushed: a quiet door, not a
            // failure (the feed may outlive several quiet windows).
            Err(_elapsed) => return Ok(None),
        };
        let GwpEvent::V1(event) = event;
        let change = match event {
            EventV1::TreeChanged { .. } => {
                let frame = self.sync_tree().await?;
                LiveChange::TreeChanged { frame }
            }
            EventV1::FramePresented { seq, hash } => {
                self.timeline.record_frame(seq, hash);
                LiveChange::FramePresented { frame: seq, hash }
            }
            EventV1::Log { record } => {
                self.timeline.record_log(record);
                LiveChange::Log
            }
        };
        Ok(Some(change))
    }

    /// The scrubber model over everything the feed has seen so far (the
    /// same model [`Timeline::from_archive`](crate::Timeline::from_archive)
    /// builds for a finished session's archive).
    pub fn timeline(&self) -> Timeline {
        self.timeline.timeline()
    }

    /// The resolved tree view at `frame` (archive semantics).
    pub fn tree_at(&self, frame: u64) -> Option<TreeView> {
        self.timeline.tree_at(frame)
    }

    /// The frame the next record will stamp against.
    pub fn current_frame(&self) -> u64 {
        self.timeline.current_frame()
    }

    /// How many dumps have landed.
    pub fn snapshot_count(&self) -> usize {
        self.timeline.snapshot_count()
    }

    /// End the conversation politely (`Bye`): the server-side connection
    /// loop exits and the pipe/socket closes. The feed stays usable for
    /// reads of what it has recorded; the next pump fails honestly.
    pub async fn bye(&mut self) -> Result<(), ViewerError> {
        let response = self.client.request(RequestV1::Bye).await?;
        if !matches!(response, ResponseV1::Bye) {
            return Err(GwpError::Protocol(format!(
                "bye expected a Bye response, got {}",
                response_kind(&response)
            ))
            .into());
        }
        Ok(())
    }

    /// The accumulated live timeline (the sync model, detached from the
    /// wire — keep it after the session ends).
    pub fn live_timeline(&self) -> &LiveTimeline {
        &self.timeline
    }

    /// Give the client back (the feed owns it exclusively).
    pub fn into_client(self) -> C {
        self.client
    }
}

/// The wire node id vocabulary ([`NodeKeyV1`] → the trace schema's
/// `u64` ids): AccessKit node ids and Logical adapter ids both land in
/// the same column, exactly as [`TreeNode`] stores them.
fn node_key_id(key: &NodeKeyV1) -> u64 {
    match key {
        NodeKeyV1::AccessKit { node, .. } => *node,
        NodeKeyV1::Logical { id, .. } => *id,
    }
}

/// DFS rows → [`TreeNode`]s: parent links rebuilt from the per-row child
/// counts ([`SnapshotNodeV1::children`]' documented contract). A stack of
/// (id, remaining children) walks the depth-first order; a row whose
/// ancestors are all consumed starts a new window root (the dump is
/// depth-first per window root, §16 v0.4).
fn rows_to_nodes(rows: &[SnapshotNodeV1]) -> Vec<TreeNode> {
    let mut nodes = Vec::with_capacity(rows.len());
    // (id, remaining children to consume)
    let mut stack: Vec<(u64, u32)> = Vec::new();
    for row in rows {
        // Find the parent: the innermost open ancestor with children
        // left, or none once every open subtree is consumed.
        let mut parent = None;
        while let Some(&(id, remaining)) = stack.last() {
            if remaining > 0 {
                parent = Some(id);
                stack.last_mut().expect("just peeked").1 -= 1;
                break;
            }
            stack.pop();
        }
        nodes.push(TreeNode {
            id: node_key_id(&row.key),
            parent,
            role: row.role.clone(),
            label: row.name.clone(),
            value: row.value.clone(),
            bounds: row.bounds,
        });
        stack.push((node_key_id(&row.key), row.children));
    }
    nodes
}

/// The response's variant name, for wire-misuse errors that must never
/// embed a payload (a stray dump's `Debug` is the whole tree). A new
/// `ResponseV1` variant breaks this match on purpose — the wire
/// discipline is append-only, and so is this vocabulary.
fn response_kind(response: &ResponseV1) -> &'static str {
    match response {
        ResponseV1::Welcome { .. } => "Welcome",
        ResponseV1::Pong => "Pong",
        ResponseV1::TreeSnapshot { .. } => "TreeSnapshot",
        ResponseV1::Resolved { .. } => "Resolved",
        ResponseV1::ActionDone { .. } => "ActionDone",
        ResponseV1::InputQueued { .. } => "InputQueued",
        ResponseV1::ClockState { .. } => "ClockState",
        ResponseV1::MockDone { .. } => "MockDone",
        ResponseV1::Codegen { .. } => "Codegen",
        ResponseV1::Error { .. } => "Error",
        ResponseV1::Bye => "Bye",
        ResponseV1::VideoFile { .. } => "VideoFile",
        ResponseV1::Windows { .. } => "Windows",
        ResponseV1::WindowUsed { .. } => "WindowUsed",
        ResponseV1::GestureDone { .. } => "GestureDone",
        ResponseV1::Shot { .. } => "Shot",
        ResponseV1::TreeDump { .. } => "TreeDump",
        ResponseV1::Event(_) => "Event",
        ResponseV1::Subscribed { .. } => "Subscribed",
    }
}

/// The wire rows → trace nodes conversion is pure — exercised with the
/// recorded-parts unit test (no tokio needed).
#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_proto::schema::NodeKeyV1;

    fn row(id: u64, role: &str, name: &str, children: u32) -> SnapshotNodeV1 {
        SnapshotNodeV1 {
            key: NodeKeyV1::AccessKit {
                tree: [0u8; 16],
                node: id,
            },
            role: role.into(),
            name: Some(name.into()),
            value: None,
            bounds: Some([0.0, 0.0, 10.0, 10.0]),
            children,
        }
    }

    #[test]
    fn dfs_rows_rebuild_parent_links_across_windows() {
        // Window 1 (2 children: Save[1 child], Cancel) + Window 2 (1 child).
        let rows = vec![
            row(1, "Window", "One", 2),
            row(2, "Button", "Save", 1),
            row(3, "Text", "label", 0),
            row(4, "Button", "Cancel", 0),
            row(5, "Window", "Two", 1),
            row(6, "Button", "Quit", 0),
        ];
        let nodes = rows_to_nodes(&rows);
        let parent_of = |id: u64| nodes.iter().find(|n| n.id == id).expect("row").parent;
        assert_eq!(parent_of(2), Some(1));
        assert_eq!(parent_of(3), Some(2), "the grandchild hangs off Save");
        assert_eq!(parent_of(4), Some(1), "Cancel is Save's sibling");
        assert_eq!(parent_of(5), None, "the second window root has no parent");
        assert_eq!(parent_of(6), Some(5));
        assert_eq!(nodes.len(), 6);
    }

    #[test]
    fn record_snapshot_stamps_at_the_current_frame() {
        let mut timeline = LiveTimeline::new("t");
        timeline.record_frame(4, 0x10);
        let rows = vec![row(1, "Window", "W", 0)];
        let frame = timeline.record_snapshot(&rows, None);
        assert_eq!(frame, 4);
        assert_eq!(timeline.snapshot_count(), 1);
        let view = timeline.tree_at(4).expect("the recorded frame resolves");
        assert_eq!(view.nodes.len(), 1);
        assert_eq!(view.snapshot_frame, Some(4));
    }

    #[test]
    fn sibling_windows_and_empty_stacks_never_misparent() {
        // A leaf row after a fully-consumed root: no parent, no panic.
        let rows = vec![row(1, "Window", "W", 1), row(2, "Button", "B", 0)];
        let nodes = rows_to_nodes(&rows);
        assert_eq!(nodes[1].parent, Some(1));
    }

    #[test]
    fn events_and_logs_stamp_against_the_current_frame() {
        let mut timeline = LiveTimeline::new("t");
        timeline.record_frame(2, 0x1);
        timeline.record_event("click", "at (1,1)");
        timeline.record_log("{\"message\":\"hello\"}");
        let scrubber = timeline.timeline();
        let tick = scrubber
            .ticks
            .iter()
            .find(|t| t.frame_seq == 2)
            .expect("frame 2 on the axis");
        assert_eq!(tick.events.len(), 1);
        assert_eq!(tick.events[0].kind, "click");
    }

    #[test]
    fn the_frame_axis_is_the_sorted_union() {
        let mut timeline = LiveTimeline::new("t");
        timeline.record_frame(5, 0x5);
        timeline.record_frame(2, 0x2);
        timeline.record_frame(5, 0x5);
        let scrubber = timeline.timeline();
        let seqs: Vec<u64> = scrubber.ticks.iter().map(|t| t.frame_seq).collect();
        assert_eq!(seqs, vec![2, 5], "sorted, deduped, no phantom frames");
    }

    #[test]
    fn response_kind_names_the_wire_vocabulary() {
        assert_eq!(response_kind(&ResponseV1::Pong), "Pong");
        assert_eq!(response_kind(&ResponseV1::Bye), "Bye");
        assert_eq!(
            response_kind(&ResponseV1::TreeDump {
                tree_seq: 1,
                node_count: 0,
                focus: None,
                nodes: Vec::new(),
            }),
            "TreeDump"
        );
    }
}
