//! The in-app host (architecture §5.2 `guiwright-driver`).
//!
//! * [`adapter`] — the [`adapter::FrameworkAdapter`] trait: implemented once
//!   per framework, all methods execute on the UI thread at frame boundaries
//!   (§5.1 rendezvous discipline).
//! * [`session`] — owns the adapter + tree service, runs the event-driven
//!   wait loop (§15.3), schedules semantic actions (§8.4).
//! * [`clock`] — the virtual clock service (§14): frame-sized stepping,
//!   pause/resume semantics, the `clock.bin` ledger.
//! * [`seams`] — state seams (§14 mock domain): app-registered hooks
//!   executed inside the adapter's runner context via `run_in`.
//! * [`server`] — serves GWP over UDS, TCP, WebSocket (M15) and the
//!   in-process embedded door (M16, the [`server::Transport`] seam) by
//!   marshalling commands to the session thread (commands are queued and
//!   applied inside the next frame, §5.1).
//! * [`events`] — the §7 server-push event channel (M12): the session
//!   thread's subscriber registry + fan-out, flushed between responses.
//! * [`log_tail`] — the event channel's log half (the log-tail milestone):
//!   the per-session `tracing` tap whose records stream as
//!   [`guiwright_proto::schema::EventV1::Log`] frames to `Log` subscribers
//!   when the server opts in with [`server::ServerOptions::log_tail`].
//! * [`gesture`] — gesture synthesis (§9, M7): the wire's high-level
//!   `Gesture` vocabulary → raw `InputEvent` sequences, paced per adapter.
//! * [`mock`] — a scriptable adapter used by tests and benches (feature
//!   `mock`).

#![forbid(unsafe_code)]

pub mod adapter;
pub mod clock;
pub mod events;
pub mod gesture;
pub mod log_tail;
pub mod seams;
pub mod server;
pub mod session;

#[cfg(feature = "mock")]
pub mod mock;

pub use adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline,
    FrameStamp, FrameworkAdapter, GesturePacing, InputEvent, LogicalTreeDiff, PointerButton,
    PumpOutcome, SemAction, WindowInfo,
};
pub use clock::{ClockLedgerEntry, ClockReport, ClockService, FRAME_STEP, LedgerCommand};
pub use gesture::{
    CodegenSurface, Gesture, codegen_call, codegen_event_call, codegen_line, synthesize,
};
pub use log_tail::{LOG_TAIL_CAPACITY, LogTail, LogTap};
pub use seams::{SeamFn, SeamRegistry, SeamTask};
pub use server::{
    BoxServeFuture, GwpServerHandle, ServerOptions, Transport, spawn_session_server,
    spawn_session_server_with,
};
pub use session::{ActionOpts, GestureOutcome, Session, SnapshotNode, TreeDump, WireShot};
