//! Guiwright [`FrameworkAdapter`] engine for Floem (architecture §4.7, M3).
//!
//! Floem ranks 6th in the §17 maturity table: a great headless event
//! harness (`HeadlessHarness` + DOM-like event router), solid public
//! geometry (`ViewId::get_visual_rect`, `element_id_at`) — but **no
//! accessibility, no test ids, and no headless pixels**. §4.7 scopes two
//! contributions to close the gaps; this crate implements both sides:
//!
//! * [`ids`] — **contribution 1a, the `test_id(...)` decorator**: a
//!   first-class registry (global test-id → `ViewId` map, mirroring
//!   floem's `debug_name` slot) with strict-mode duplicate detection and
//!   ABA generation guards. The upstreamable patch is a small decorator
//!   view + a global registry — `docs/m3-floem-contributions.md` carries
//!   the patch design; this module is the driver-side engine it feeds.
//! * [`roles`] — **contribution 1b, role metadata on core views**:
//!   button/text-input/checkbox map to AccessKit-compatible [`Role`]s so
//!   the logical tree carries semantics without AccessKit.
//! * [`paint`] — **contribution 2, tiny-skia headless `PaintState`**:
//!   floem's `PaintState` never initializes without a GPU surface
//!   (`window/handle.rs:313-322`), so headless screenshots are impossible
//!   today. The contribution is an offscreen tiny-skia surface
//!   (`tiny-skia` + softbuffer are already compiled in;
//!   `FLOEM_FORCE_TINY_SKIA=1` proves the renderer path) fed by an
//!   explicit [`paint::PaintOp`] list routed from `render_frame`. This
//!   module is the surface + op contract + tests; the upstream patch
//!   routes floem's display list into it.
//! * [`logical`] — the Level-2 tree engine: [`logical::ViewReport`]s in
//!   (geometry from `get_visual_rect`, roles from [`roles`], ids from
//!   [`ids`]), a synthesized AccessKit-shaped
//!   [`LogicalTreeDiff`](guiwright_driver::adapter::LogicalTreeDiff) out —
//!   selectors, actionability and `expect` run unchanged (§6.1).
//! * [`input`] — the input vocabulary the harness executes through the
//!   real event pipeline (`dispatch_event` headless, `route_event` live,
//!   §4.7; the winit floem builds against is the **fork**, never
//!   crates.io winit).
//! * [`clock`] — the harness-timer clock plan (§4.7 "pumps
//!   `Event::Window(UpdatePhase)` ticks with virtual time"); stepping
//!   lives in the driver's clock service (§14).
//! * [`contract`] — the downstream glue contract + upstream patch
//!   checklist. This crate ships **framework-free** (the
//!   `guiwright-adapter-slint` precedent, §18): floem 0.2.0 on crates.io
//!   predates the `HeadlessHarness` (§4.7 recon cites `src/headless.rs:101`
//!   at `lapce/floem@1351ffb`, post-0.2.0 main), so the glue compiles
//!   downstream until the harness releases — then it lands behind a
//!   `floem` feature exactly like iced's and freya's.
//!
//! [`FrameworkAdapter`]: guiwright_driver::adapter::FrameworkAdapter
//! [`Role`]: accesskit::Role

#![forbid(unsafe_code)]

pub mod clock;
pub mod contract;
pub mod ids;
pub mod input;
pub mod logical;
pub mod paint;
pub mod roles;

pub use contract::FloemAdapterContract;
pub use ids::{TestIdError, TestIdRegistry};
pub use logical::{FloemLogicalTree, ViewReport};
pub use paint::{PaintError, PaintOp, PaintState};
pub use roles::RoleRegistry;
