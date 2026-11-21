//! The downstream floem glue **reference example** (architecture §4.7,
//! §17 row 6) — the compilable binding proof the
//! `guiwright-adapter-floem` contract points to.
//!
//! The engine crate (`guiwright-adapter-floem`) ships framework-free: the
//! real harness binding lives downstream. The canonical downstream home
//! is the repo's `glue/floem` workspace (the real `lapce/floem` graph);
//! this example is its dependency-free twin — the floem API surface the
//! binding touches (`TestRoot` boot, `HeadlessHarness::new_with_size`,
//! `ViewId::get_visual_rect` / `debug_name` / `children`,
//! `dispatch_event` with real hit-testing) is **mirrored** in
//! [`mirror`] with the exact names and signatures the real harness rides,
//! and [`adapter`] implements [`FrameworkAdapter`] over it through the
//! engine's modules:
//!
//! | `FrameworkAdapter` method | Engine module the binding drives |
//! |---|---|
//! | `meta().caps` | [`guiwright_adapter_floem::FloemAdapterContract`] (the §6.2 row as data) |
//! | `pump` | one harness pass → [`guiwright_driver::adapter::FrameStamp`] |
//! | `logical_snapshot` | the `ViewId` walk → [`guiwright_adapter_floem::ViewReport`] → [`guiwright_adapter_floem::FloemLogicalTree::rebuild`] (Level-2, §6.1) |
//! | `act` | minted `NodeId` → handle facts → spatial input at the visual rect (§8.4) |
//! | `input` | [`guiwright_adapter_floem::input::translate`] → `dispatch_event` per step |
//! | `render_frame` | [`guiwright_adapter_floem::paint`]: `PaintState` + `PaintOp`s over real view geometry (§4.7 contribution 2) |
//! | `clock` | [`guiwright_adapter_floem::clock::plan`] → harness ticks (§14) |
//!
//! Everything else the real harness documents as a seam is carried over
//! verbatim: text/value facts ride the app's shared state (floem exposes
//! no text getters at the pinned revision), `SetValue` reports
//! `Unsupported` carrying the headless-IME fill-blocker evidence
//! (`contract.rs`, m80), and the disabled-state dispatch surface is
//! absent. The integration test (`tests/bind.rs`) boots the mirror app
//! through a real `Session` — resolve → click → `expect` — and the
//! `compile_fail` doc test below pins the shape: a harness without the
//! floem surface cannot bind.
//!
//! # The overlay binding is signature-enforced too (m86)
//!
//! The §13 route (the 7/7 overlay count's floem leg): `show_overlay`
//! hands the plan to the engine's [`guiwright_adapter_floem::OverlayHost`]
//! — the mock of the real present-path hook — and `render_frame` extends
//! its scene with the host's ops. When the real framework API matches
//! the mock signature, the binding compiles and routes unchanged; a
//! surface WITHOUT the present signature cannot bind:
//!
//! ```compile_fail
//! use guiwright_adapter_floem::OverlayHost;
//! use guiwright_inspect::overlay::OverlayPlan;
//!
//! struct Placeholder;
//!
//! fn present(host: &mut Placeholder, plan: &OverlayPlan) {
//!     // E0599: `present` is the overlay host's method — a surface
//!     // without the overlay-host signature cannot bind the §13 route,
//!     // which is exactly the contract the mirror pins.
//!     host.present(plan);
//! }
//! ```
//!
//! # The binding is signature-enforced (compile-fail proof)
//!
//! ```compile_fail
//! use guiwright_adapter_floem::FloemLogicalTree;
//! use guiwright_downstream_glue_floem::mirror::HeadlessHarness;
//!
//! // The §4.7 binding folds ViewReports out of the floem-shaped harness.
//! // A placeholder that is NOT a `HeadlessHarness` cannot bind: the
//! // introspection call sites below fail with E0599 (`root_id` /
//! // `get_visual_rect` / `dispatch_event` are the harness's methods),
//! // which is exactly the contract the example pins.
//! struct Placeholder;
//!
//! fn walk(harness: &mut Placeholder, engine: &mut FloemLogicalTree) {
//!     let root = harness.root_id();
//!     let rect = root.get_visual_rect();
//!     harness.dispatch_event(guiwright_downstream_glue_floem::mirror::Event::PointerLeave);
//!     let _ = (root, rect, engine);
//! }
//! ```

#![forbid(unsafe_code)]

pub mod adapter;
pub mod mirror;

pub use adapter::FloemDownstreamGlue;
pub use mirror::{HeadlessHarness, TestRoot, ViewId};
