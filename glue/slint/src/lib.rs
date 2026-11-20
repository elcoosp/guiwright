//! Guiwright Slint harness glue (architecture §4.4 door A, §17 row 5,
//! m78).
//!
//! This crate is the **framework half** of the slint adapter: it binds
//! the framework-free engine (`guiwright-adapter-slint` — introspection
//! model, `(component, item)` id bridge, role registry, clock plan) to
//! the real `i-slint-backend-testing` `TestingBackend`, the way m76 bound
//! the iced engine to `iced_test::Emulator`. It lives in its **own
//! workspace** (`glue/slint`, excluded from the main one) because the
//! slint facade's dependency graph resolves skia-bindings (`links =
//! "skia"`) while freya's testing stack claims the same links key in the
//! main workspace — the constraint recorded since M2, re-verified against
//! slint 1.19.0 at m78 (see the engine crate's docs for the evidence).
//!
//! * [`harness`] — [`slint::platform::set_platform`] installs the
//!   testing backend (mock time + software rasterizer),
//!   `Window::dispatch_event` injects real `WindowEvent`s,
//!   `Window::take_snapshot` renders headless pixels, and
//!   `mock_elapsed_time` advances the first-class virtual clock — all
//!   behind the [`FrameworkAdapter`] trait the session drives.
//!
//! # Consumer walkthrough
//!
//! ```ignore
//! cargo run --manifest-path glue/slint/Cargo.toml --example counter_form
//! ```
//!
//! The runtime tests (`tests/slint_embedded.rs`) drive the real
//! `counter_form.slint` app — stock `Button` + `LineEdit` + `Text` —
//! through [`Session`]: resolve by role/name with real bounds, click
//! increments, fill types, snapshots stay stable.
//!
//! [`FrameworkAdapter`]: guiwright_driver::adapter::FrameworkAdapter
//! [`Session`]: guiwright_driver::session::Session

pub mod harness;
