//! Guiwright Floem harness glue (architecture §4.7, §17 row 6, m80).
//!
//! This crate is the **framework half** of the floem adapter: it binds
//! the framework-free engine (`guiwright-adapter-floem` — test-id
//! registry, role registry, logical-tree engine, tiny-skia
//! `PaintState`, input vocabulary, clock plan) to the real
//! `lapce/floem` `HeadlessHarness`, the way m78 bound the slint engine
//! to `i-slint-backend-testing` and m76 bound iced to `iced_test`.
//! It lives in its **own workspace** (`glue/floem`, excluded from the
//! main one) by the recorded floem constraint (crate docs of
//! `guiwright-adapter-floem`): floem 0.2.0 predates the harness, so the
//! binding rides the pinned git revision, and the main workspace must
//! never carry floem's Linux graph (§5.3).
//!
//! * [`harness`] — [`TestRoot`] boot, the real [`ViewId`] walk,
//!   `dispatch_event` input with real hit-testing, the engine's
//!   tiny-skia `PaintState` pixels, and the harness-timer clock plan —
//!   all behind the [`FrameworkAdapter`] trait the session drives.
//!
//! # Consumer walkthrough
//!
//! ```ignore
//! cargo run --manifest-path glue/floem/Cargo.toml --example counter_form
//! ```
//!
//! The runtime tests (`tests/floem_embedded.rs`) drive the real floem
//! counter+form app — stock `button` + `text_input` + derived labels —
//! through [`Session`]: resolve by role/name with real bounds, click
//! increments, fill types, snapshots stay stable.
//!
//! [`FrameworkAdapter`]: guiwright_driver::adapter::FrameworkAdapter
//! [`Session`]: guiwright_driver::session::Session

pub mod harness;
