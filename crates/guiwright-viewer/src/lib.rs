//! Guiwright trace viewer (architecture §12 "Viewer" + §5.2
//! `guiwright-viewer`).
//!
//! * [`archive`] — [`archive::TraceArchive`] reads a `gwtrace/v1` zip:
//!   `meta.json`, `events.ndjson`, `logs.ndjson`, `trees/<seq>.bin`
//!   (postcard+LZ4), `trees/diff.bin` (sequential LZ4 blobs), `index.bin`
//!   and `clock.bin`.
//! * [`timeline`] — [`timeline::Timeline`] merges the archive into a
//!   scrubber model: one tick per presented frame, the UI tree resolved
//!   at any point from the nearest snapshot + diffs (§12 "tree inspector
//!   at any point, resolved from snapshot+diffs").
//! * [`html`] — the self-contained `trace.html` emitter: timeline
//!   scrubber, tree inspector, event/log views and the virtual-clock
//!   ledger in one file (no server, no external assets, §12). The live
//!   half (`render_live_html`) renders the same scrubber from a
//!   running session's accumulated model (M23 — §12's recorded
//!   follow-on).
//! * [`live`] — the M18 live mode: [`live::LiveFeed`] attaches to a
//!   running GWP session over any [`guiwright_proto::ClientConn`] door
//!   and [`live::LiveTimeline`] accumulates the same scrubber model the
//!   archive path builds (§12 "live", the §16 v0.6 row).
//!
//! ```ignore
//! let archive = TraceArchive::open("guiwright-login_flow-1.zip")?;
//! let timeline = Timeline::from_archive(&archive);
//! let tree = timeline.tree_at(frame_seq);      // resolved view
//! std::fs::write("trace.html", viewer::render_html(&archive, &timeline))?;
//! ```

#![forbid(unsafe_code)]

pub mod archive;
pub mod html;
pub mod live;
pub mod timeline;

pub use archive::{TraceArchive, ViewerError};
pub use html::{render_html, render_live_html};
pub use live::{LiveChange, LiveFeed, LiveInfo, LiveTimeline};
pub use timeline::{Timeline, TimelineTick, TreeView, TreeViewNode};
