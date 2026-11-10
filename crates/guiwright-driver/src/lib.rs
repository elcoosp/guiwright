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
//! * [`server`] — serves GWP over UDS by marshalling commands to the session
//!   thread (commands are queued and applied inside the next frame, §5.1).
//! * [`gesture`] — gesture synthesis (§9, M7): the wire's high-level
//!   `Gesture` vocabulary → raw `InputEvent` sequences, paced per adapter.
//! * [`mock`] — a scriptable adapter used by tests and benches (feature
//!   `mock`).

#![forbid(unsafe_code)]

pub mod adapter;
pub mod clock;
pub mod gesture;
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
pub use seams::{SeamFn, SeamRegistry, SeamTask};
pub use server::{GwpServerHandle, spawn_session_server};
pub use session::{ActionOpts, GestureOutcome, Session};
