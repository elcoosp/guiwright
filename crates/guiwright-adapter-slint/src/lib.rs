//! Guiwright Slint adapter engine (architecture §4.4, §17 row 5).
//!
//! Recon verdict: the testing backend is **complete but semver-exempt**
//! (per-release maintenance) and the AccessKit door exists only for live
//! apps (winit backend). Guiwright therefore drives the *testing*
//! backend — introspection (Level-2 logical tree) + `mock_elapsed_time`
//! clock. This crate ships the **framework-free half** — the engine the
//! glue feeds — plus the testing build helper (§11); the framework-facing
//! glue lives downstream because upstream slint cannot coexist with
//! freya in one workspace (see *Glue contract* below).
//!
//! * [`ids`] — the `(component, item)` identity bridge (§6.1).
//! * [`logical`] — the Level-2 engine: introspection pass → synthesized
//!   AccessKit update (minted ids, ABA guards, test ids).
//! * [`clock`] — the `mock_elapsed_time` plan (§14 slint row).
//! * [`build`] — the testing build helper (§11: headless backend + debug
//!   info).
//!
//! # Glue contract (downstream workspaces)
//!
//! A slint-only downstream workspace wires the glue in one small module
//! (slint's testing backend is semver-exempt, §17 row 5 — this is the
//! only per-release surface):
//!
//! ```ignore
//! // downstream Cargo.toml:
//! //   slint = { version = "…", default-features = false,
//! //             features = ["std", "compat-1-2", "renderer-software"] }
//! //   i-slint-backend-testing = { version = "…", default-features = false }
//!
//! /// Introspection pass → the engine (walk the testing backend's item
//! /// tree into [`IntrospectionItem`]s, then rebuild the diff).
//! fn rebuild_diff(builder: &mut SlintTreeBuilder, root: IntrospectionItem) { … }
//!
//! /// Clock: execute the plan against the testing backend (§14 slint row).
//! fn run_clock_plan(steps: &[SlintClockStep]) {
//!     for step in steps {
//!         if let SlintClockStep::MockElapsedTime { ms } = *step {
//!             i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
//!         }
//!     }
//! }
//!
//! /// Boot (§11 slint row): headless software backend + debug info.
//! i_slint_backend_testing::init_integration_test_with_mock_time();
//! ```
//!
//! # Upstream constraint (recorded at M2)
//!
//! Cargo resolves *optional* dependencies into the lockfile, and the
//! `links = "skia"` uniqueness check applies at resolution time. freya's
//! testing stack (freya-skia-bindings) and slint's renderer-skia chain
//! (i-slint-backend-winit → i-slint-renderer-skia → skia-bindings) both
//! claim `links = "skia"`, so **no single workspace can depend on both
//! freya-testing and slint** for any slint release with a skia renderer
//! in its graph (checked 1.14.1 and 1.18.1, 2026-11). The glue therefore
//! compiles in slint-only downstream workspaces; this workspace keeps
//! the engine framework-free and CI verifies exactly that. Revisit when
//! freya moves off pinned skia or the adapters ship per-framework.

#![forbid(unsafe_code)]

pub mod build;
pub mod clock;
pub mod ids;
pub mod logical;

pub use ids::SlintId;
pub use logical::{IntrospectionItem, SlintTreeBuilder, role_of_kind};
