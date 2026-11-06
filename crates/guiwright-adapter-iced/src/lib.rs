//! Guiwright [`FrameworkAdapter`] engine for iced (architecture §4.6, M1).
//!
//! Iced has no AccessKit tree — Guiwright builds the semantic tree itself
//! (§4.6 "no a11y tree — nothing to piggyback; Guiwright builds its
//! logical tree from selector ops", "adds role semantics via a contributed
//! widget-kind registry"). This crate is that engine, framework-free:
//!
//! * [`logical`] — the tree state machine: consumes [`WidgetReport`]s,
//!   mints stable `NodeId`s per widget handle, and synthesizes an
//!   AccessKit-shaped [`LogicalTreeDiff`] so the consumer tree, selectors,
//!   actionability and `expect` run unchanged (driver §6.1 normalization).
//! * [`registry`] — the widget-kind → AccessKit [`Role`] registry
//!   (§4.6); apps contribute kinds at startup.
//! * [`actions`] — [`SemAction`] → [`WidgetOp`] routing per role.
//! * [`input`] — the input vocabulary the harness executes through the
//!   framework's real pipeline (`UserInterface::update`, §4.6).
//! * [`policy`] — iced Emulator determinism modes → pump budgets (§4.6
//!   "`Zen` = settle-to-quiescence, `Patient`, `Immediate`").
//!
//! The `iced` feature adds [`harness`]: the thin glue that binds the
//! engine to `iced_test`'s [`Emulator`]/[`Simulator`]. It is feature-gated
//! because the framework stack (iced_renderer → wgpu) must never leak into
//! builds that do not ask for it (§18 "adapters are thin & feature-gated";
//! §5.3 "nothing else leaks into release builds").
//!
//! [`FrameworkAdapter`]: guiwright_driver::adapter::FrameworkAdapter
//! [`LogicalTreeDiff`]: guiwright_driver::adapter::LogicalTreeDiff

#![forbid(unsafe_code)]

pub mod actions;
pub mod input;
pub mod logical;
pub mod policy;
pub mod registry;

#[cfg(feature = "iced")]
pub mod harness;

pub use actions::{WidgetOp, route};
pub use input::{InputOp, KeyOp, PointerOp, translate};
pub use logical::{LogicalTree, WidgetReport};
pub use policy::{Mode, PUMP_BUDGET_ZEN, pump_budget};
pub use registry::RoleRegistry;
