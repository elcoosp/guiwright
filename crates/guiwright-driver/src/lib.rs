//! The in-app host (architecture §5.2 `guiwright-driver`).
//!
//! * [`adapter`] — the [`adapter::FrameworkAdapter`] trait: implemented once
//!   per framework, all methods execute on the UI thread at frame boundaries
//!   (§5.1 rendezvous discipline).
//! * [`session`] — owns the adapter + tree service, runs the event-driven
//!   wait loop (§15.3), schedules semantic actions (§8.4).
//! * [`server`] — serves GWP over UDS by marshalling commands to the session
//!   thread (commands are queued and applied inside the next frame, §5.1).
//! * [`mock`] — a scriptable adapter used by tests and benches (feature
//!   `mock`).

#![forbid(unsafe_code)]

pub mod adapter;
pub mod server;
pub mod session;

#[cfg(feature = "mock")]
pub mod mock;

pub use adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline,
    FrameStamp, FrameworkAdapter, InputEvent, LogicalTreeDiff, PointerButton, PumpOutcome,
    SemAction,
};
pub use server::spawn_session_server;
pub use session::{ActionOpts, Session};
