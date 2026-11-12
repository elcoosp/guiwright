//! Guiwright core — the framework-agnostic brain.
//!
//! This crate implements the pieces of the [Guiwright architecture] that do
//! not depend on any GUI framework (architecture §5.2 `guiwright-core`):
//!
//! * [`tree::TreeService`] — one incremental [`accesskit_consumer`] tree per
//!   window plus per-frame locator indexes, generation counters (ABA guards)
//!   and the single diff fan-out point (§15.2).
//! * [`selector::Selector`] — the Playwright-shaped selector model (§8.1):
//!   `getByRole`, `getByLabel`, `getByText`, `getByTestId`, `getByValue`,
//!   descendant chains and filters (`.first()`/`.nth()`/`.has_text()`/…).
//! * [`tree::TreeService::resolve`] — the resolution pipeline (§8.2): index
//!   lookup + filter application + strict-mode uniqueness policy.
//! * [`actionability`] — the anti-flakiness core (§8.3): attached / visible /
//!   stable / enabled / receives-events checks.
//! * [`wait`] — event-driven waiter state machines (§15.3 zero-polling
//!   auto-wait). Waiters are woken by tree diffs and frame-presented events;
//!   they never sleep-poll.
//! * [`expect`] — retry-until-timeout assertions (`expect(locator)
//!   .to_be_visible()`, §16 "Assertions with retry").
//!
//! The crate's runtime dependencies are the one universal substrate —
//! `accesskit` + `accesskit_consumer` (the §5.2 dependency rule) — plus
//! `thiserror` (the error vocabulary) and `tracing` (the dropped-diff /
//! pre-init warnings, §15.3). Nothing else: no framework, no runtime.
//!
//! [Guiwright architecture]: https://github.com/elcoosp/guiwright/blob/main/docs/architecture.md

#![forbid(unsafe_code)]

pub mod actionability;
pub mod error;
pub mod expect;
pub mod key;
pub mod selector;
/// Synthetic AccessKit fixtures shared by core, driver, adapter tests and
/// benches (kept public so integration tests reuse the same reality).
pub mod test_util;
pub mod tree;
pub mod wait;

pub use error::CoreError;
pub use key::{Generation, MAIN_WINDOW, NodeKey};
pub use selector::{Nth, Selector, TextMatch, TextPattern};
pub use tree::{FrameInfo, Resolved, TreeDiffSummary, TreeService, WindowScope};
pub use wait::{ChecksWaiter, StabilityTracker, WaitOutcome, Waiter, WaiterEvent};
