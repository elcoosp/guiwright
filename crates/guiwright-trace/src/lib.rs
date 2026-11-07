//! Guiwright trace — the `gwtrace/v1` zip recorder (architecture §12).
//!
//! * [`schema`] — the archive's records (versioned, forward-compatible).
//! * [`recorder`] — the UI-thread recorder + writer thread (bounded SPSC
//!   ring, drop-counted backpressure, 256 MB oldest-first frame eviction).
//! * [`layer`] — a `tracing` layer that captures framework + driver logs
//!   into `logs.ndjson`.
//!
//! The kit starts a recorder per test (§11 process-per-test), the driver
//! feeds it events at frame boundaries, and the viewer consumes the zip.

#![forbid(unsafe_code)]

pub mod layer;
pub mod recorder;
pub mod schema;
pub mod video;

pub use layer::TraceLogLayer;
pub use recorder::{
    DEFAULT_FRAME_BUDGET_BYTES, DEFAULT_RING_CAPACITY, TraceError, TraceFile, TraceOptions,
    TraceRecorder, TraceSink,
};
pub use schema::{
    AdapterRecord, ClockCommand, ClockJump, FrameIndexEntry, SCHEMA, TraceEvent, TraceMeta,
    TreeDiffRecord, TreeNode, TreeSnapshot,
};
