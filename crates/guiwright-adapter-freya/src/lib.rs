//! Guiwright [`FrameworkAdapter`] support for Freya (architecture §4.3,
//! M1). Freya is Level-1: `freya_testing::TestingRunner` is a complete
//! sync, windowless harness whose `AccessibilityTree` commits real
//! AccessKit `TreeUpdate`s — the engine here handles the pieces the
//! harness leaves open, framework-free:
//!
//! * [`ids`] — the `AccessibilityId` ↔ accesskit `NodeId` bridge (§4.3
//!   "stable selectors require apps to set `.a11y_id(AccessibilityId(n))`";
//!   auto ids are a process counter, so the bridge is the single place
//!   that owns the mapping).
//! * [`keys`] — key sequences with modifiers **and explicit `KeyUp`** (§4.3
//!   gap: "`press_key` sends `KeyDown` only — we drive raw
//!   `PlatformEvent::Keyboard` with modifiers+KeyUp").
//! * [`buffer`] — frame-quantized event ordering (§4.3: "input is
//!   frame-quantized by ragnarok's `EventBuffer` — respect
//!   press→sync→move→release ordering").
//! * [`platform`] — the [`InputEvent`] → `PlatformEvent` vocabulary.
//!
//! The `freya` feature adds [`runner`]: the thin `TestingRunner` glue.
//! It is feature-gated because the framework stack (software Skia) must
//! not leak into builds that do not ask for it (§18).
//!
//! [`FrameworkAdapter`]: guiwright_driver::adapter::FrameworkAdapter
//! [`InputEvent`]: guiwright_driver::adapter::InputEvent

#![forbid(unsafe_code)]

pub mod buffer;
pub mod ids;
pub mod keys;
pub mod platform;

#[cfg(feature = "freya")]
pub mod runner;

pub use buffer::{PendingEvent, SyncEventBuffer, click_sequence};
pub use ids::AccessibilityMap;
pub use keys::{KeyEvent, KeyPhase, key_sequence};
pub use platform::translate;
