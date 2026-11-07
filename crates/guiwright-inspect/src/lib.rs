//! Guiwright inspector — codegen + overlay (architecture §13).
//!
//! * [`codegen`] — the interaction → kit-code recorder. Every recorded
//!   action emits code against the **minimal stable selector** chosen by a
//!   cost model: `TestId` > role+accessible-name > label > text (with
//!   `.first()` narrowing on ambiguity, §13). Strict-mode uniqueness is
//!   checked live — ambiguity becomes a warning while recording.
//! * [`overlay`] — the adapter-level overlay plan (hover outlines +
//!   role/TestId badges) drawn by the framework adapters at present time.
//!
//! Recording happens at the adapter tap, **not** by hooking app callbacks
//! — codegen works on unmodified apps (§13).

#![forbid(unsafe_code)]

pub mod codegen;
pub mod overlay;

pub use codegen::{CodegenError, CodegenRecorder, NodeFacts, RecordedLine};
pub use overlay::{Badge, OverlayPlan, OverlayRect};
