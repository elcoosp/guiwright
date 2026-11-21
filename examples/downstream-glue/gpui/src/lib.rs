//! The downstream gpui glue **reference example** (architecture §4.5,
//! §17 row 7) — the compilable binding proof the
//! `guiwright-adapter-gpui` contract points to.
//!
//! The engine crate (`guiwright-adapter-gpui`) ships framework-free: the
//! app-side wiring lives downstream. The canonical downstream home is the
//! repo's `glue/gpui` workspace (the real gpui/test-support graph); this
//! example is its dependency-free twin — the gpui API surface the
//! binding touches (`Window::dispatch_event(PlatformInput)` as the
//! canonical input seam, per-frame `a11y_tree_update` while active, the
//! `Entity` reads the fact readers close over, `VisualTestContext`
//! dispatch, the seeded virtual dispatcher rounds) is **mirrored** in
//! [`mirror`], and [`adapter`] implements [`FrameworkAdapter`] over it
//! through the engine's modules:
//!
//! | `FrameworkAdapter` method | GPUI mapping (§4.5) | Engine module |
//! |---|---|---|
//! | `meta().caps` | hybrid row via `caps_for(test_support = true)` | [`guiwright_adapter_gpui::GpuiAdapterContract`] |
//! | `pump` | foreground-executor round, `dispatch_after(0)` waker | [`guiwright_adapter_gpui::clock`] |
//! | `tree_update` | `a11y_tree_update` while active, through the activation model | [`guiwright_adapter_gpui::A11yHybrid`] |
//! | `act` | a11y node → synthesized mouse events at its bounds (§8.4) | [`guiwright_adapter_gpui::A11yHybrid::bounds_of`] |
//! | `input` | `Window::dispatch_event(PlatformInput)` per step | [`guiwright_adapter_gpui::input::translate`] |
//! | `last_frame`/`render_frame` | test-support `rendered_frame` + strided pixel digest | [`guiwright_adapter_gpui::frames`] |
//! | `clock` | seeded virtual dispatcher rounds | [`guiwright_adapter_gpui::clock`] |
//! | `run_in` | foreground executor + root entity context | [`guiwright_adapter_gpui::RunInPlan`] |
//!
//! The documented seams carry over verbatim: `SetValue` reports
//! `Unsupported` with the zed test-scheduler fill-blocker evidence
//! (`contract.rs`, m80), pixels stay test-support-gated (the mirror
//! models `test_support = true`, the row the §11 GPUI line promises), and
//! `framework_version` names the platform crate versions (§18 churn
//! mitigation). The integration test (`tests/bind.rs`) boots the mirror
//! app through a real `Session` — resolve → click → `expect` → seam — and
//! the `compile_fail` doc test below pins the shape: a window without the
//! gpui dispatch surface cannot bind.
//!
//! # The overlay binding is signature-enforced too (m86)
//!
//! The §13 route (the 7/7 overlay count's gpui leg): `show_overlay`
//! hands the plan to the engine's [`guiwright_adapter_gpui::OverlayHost`]
//! — the mock of the real test-support paint route (`paint_quad` →
//! `painted_quads`) — and `render_pixels` composites the host's quads
//! onto the scripted raster. When the real framework API matches the
//! mock signatures, the binding compiles and routes unchanged; a window
//! WITHOUT the paint surface cannot bind:
//!
//! ```compile_fail
//! use guiwright_adapter_gpui::OverlayHost;
//! use guiwright_inspect::overlay::OverlayPlan;
//!
//! struct Placeholder;
//!
//! fn paint(host: &mut Placeholder, plan: &OverlayPlan) {
//!     // E0599: `paint_quads` is the overlay host's method — a surface
//!     // without the painted-quad signature cannot bind the §13 route,
//!     // which is exactly the contract the mirror pins.
//!     host.paint_quads(plan);
//! }
//! ```
//!
//! # The binding is signature-enforced (compile-fail proof)
//!
//! ```compile_fail
//! use guiwright_adapter_gpui::frames::FrameDigest;
//! use guiwright_downstream_glue_gpui::mirror::Window;
//!
//! // The §4.5 binding dispatches `PlatformInput` through the real window
//! // seam. A placeholder that is NOT a gpui-shaped `Window` cannot bind:
//! // the dispatch call below fails with E0599 (`dispatch_event` is the
//! // window's method), which is exactly the contract the example pins.
//! struct Placeholder;
//!
//! fn dispatch(window: &mut Placeholder, input: guiwright_downstream_glue_gpui::mirror::PlatformInput) {
//!     window.dispatch_event(input);
//!     let _ = FrameDigest::of(&[0_u8]);
//! }
//! ```

#![forbid(unsafe_code)]

pub mod adapter;
pub mod mirror;

pub use adapter::GpuiDownstreamGlue;
pub use mirror::{Entity, PlatformInput, VisualTestContext, Window};
