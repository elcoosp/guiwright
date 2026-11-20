//! Guiwright GPUI harness glue (architecture §4.5, §17 row 7, m80).
//!
//! This crate is the **framework half** of the gpui adapter: it binds
//! the framework-free engine (`guiwright-adapter-gpui` — a11y-hybrid
//! activation model, `PlatformInput` input steps, test-support
//! dispatcher clock plan, foreground-executor seam runner, §15.5 frame
//! digest) to the real gpui test-support platform, the way m78 bound
//! the slint engine to `i-slint-backend-testing` and m80 bound floem to
//! its `HeadlessHarness`. It lives in its **own workspace**
//! (`glue/gpui`, excluded from the main one) by the recorded gpui
//! constraint (crate docs of `guiwright-adapter-gpui`): the
//! `test-support` feature exists only on the git source and the Linux
//! platform stack needs system libraries (xkbcommon, wayland) the main
//! workspace must never require (§5.3).
//!
//! * [`app`] — the real counter+form entity: `div()` buttons with real
//!   `on_click` listeners over real entity state, a focused text field
//!   with a real `on_key_down` handler, and gpui's own `role(...)`
//!   plus `debug_selector` DSL carrying the roles and the walked
//!   geometry.
//! * [`harness`] — the [`GpuiAdapter`]: `TestAppContext::build` over a
//!   seeded `TestDispatcher`, `Window::dispatch_event(PlatformInput)`
//!   input, the synthesized-diff fallback over framework-recorded
//!   bounds through the engine's `A11yHybrid`, dispatcher clock
//!   rounds, and the foreground-entity seam runner — all behind the
//!   [`FrameworkAdapter`] trait the session drives.
//!
//! # Consumer walkthrough
//!
//! ```ignore
//! cargo run --manifest-path glue/gpui/Cargo.toml --example counter_form
//! ```
//!
//! The runtime tests (`tests/gpui_embedded.rs`) drive the real gpui
//! counter+form app through [`Session`]: resolve by role/name with
//! real bounds, click increments, fill types, digests follow content.
//!
//! [`FrameworkAdapter`]: guiwright_driver::adapter::FrameworkAdapter
//! [`Session`]: guiwright_driver::session::Session

pub mod app;
pub mod harness;
