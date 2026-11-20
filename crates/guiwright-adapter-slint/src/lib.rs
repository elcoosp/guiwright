//! Guiwright Slint adapter engine (architecture §4.4, §17 row 5).
//!
//! Recon verdict: the testing backend is **complete but semver-exempt**
//! (per-release maintenance) and the AccessKit door exists only for live
//! apps (winit backend). Guiwright therefore drives the *testing*
//! backend — introspection (Level-2 logical tree) + `mock_elapsed_time`
//! clock. This crate ships the **framework-free half** — the engine the
//! glue feeds — plus the testing build helper (§11); the framework-facing
//! glue lives in the repo's slint-only workspace `glue/slint`, bound to
//! the real `i-slint-backend-testing` `TestingBackend` since m78.
//!
//! * [`ids`] — the `(component, item)` identity bridge (§6.1).
//! * [`logical`] — the Level-2 engine: introspection pass → synthesized
//!   AccessKit update (minted ids, ABA guards, test ids).
//! * [`clock`] — the `mock_elapsed_time` plan (§14 slint row).
//! * [`build`] — the testing build helper (§11: headless backend + debug
//!   info).
//!
//! # Glue contract (the `glue/slint` workspace)
//!
//! The glue binds the engine in one small module (slint's testing
//! backend is semver-exempt, §17 row 5 — this is the only per-release
//! surface):
//!
//! ```ignore
//! // glue/slint Cargo.toml (its own workspace root — see the header
//! // comment there and the constraint below):
//! //   slint = { version = "=1.19.0", default-features = false,
//! //             features = ["std", "compat-1-2", "renderer-software"] }
//! //   i-slint-backend-testing = { version = "=1.19.0",
//! //             default-features = false, features = ["renderer-software"] }
//!
//! /// Boot (§11 slint row): the testing backend as the thread's platform —
//! /// mock time + the software rasterizer for headless `take_snapshot`.
//! slint::platform::set_platform(Box::new(TestingBackend::new(
//!     TestingBackendOptions { mock_time: true, threading: false,
//!                             renderer_name: Some("software".into()) })));
//!
//! /// Introspection pass → the engine (walk the testing backend's
//! /// `ElementHandle` tree into [`IntrospectionItem`]s, then rebuild the
//! /// diff).
//! fn rebuild_diff(builder: &mut SlintTreeBuilder, root: &IntrospectionItem) {
//!     builder.rebuild(root);
//! }
//!
//! /// Clock: execute the plan against the testing backend (§14 slint row).
//! fn run_clock_plan(steps: &[SlintClockStep]) {
//!     for step in steps {
//!         if let SlintClockStep::MockElapsedTime { ms } = *step {
//!             i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
//!         }
//!     }
//! }
//! ```
//!
//! # Upstream constraint (recorded at M2, re-verified at m78)
//!
//! Cargo resolves *optional* dependencies into the lockfile, and the
//! `links = "skia"` uniqueness check applies at resolution time. freya's
//! testing stack (freya-skia-bindings) and slint's facade graph both
//! claim `links = "skia"`, so **no single workspace can depend on both
//! freya-testing and the slint facade**. Verified against slint 1.19.0
//! (2026-11): the facade depends on `i-slint-backend-selector`
//! unconditionally; the selector's desktop graph carries
//! `i-slint-backend-winit` (optional) whose own optional
//! `i-slint-renderer-skia` → `skia-safe` → `skia-bindings` chain lands in
//! the lockfile **with every feature combination** (`default-features =
//! false` included) — `cargo tree -i i-slint-renderer-skia` prints
//! "nothing to print" (nothing activated it) while `Cargo.lock` still
//! carries `skia-bindings 0.157.0`, and the resolver rejects the graph
//! the moment freya's `freya-skia-bindings` joins it. The testing backend
//! crate itself (with `renderer-software`) is skia-free — the facade's
//! unconditional selector edge is what drags skia in — and apps that
//! compile real `.slint` code (the `slint!` macro / `slint-build` both
//! generate facade-bound code) cannot escape it.
//!
//! The glue therefore compiles in the slint-only `glue/slint` workspace
//! (its own root, `resolver = "2"`), which path-depends on this engine,
//! `guiwright-driver` and `guiwright-core`; the main workspace keeps
//! carrying freya's skia entry and never slint's. CI runtime-tests the
//! glue (`cargo test --manifest-path glue/slint/Cargo.toml`). Revisit
//! only if slint makes the winit/skia chain truly optional in the
//! lockfile or freya moves off pinned skia.

#![forbid(unsafe_code)]

pub mod build;
pub mod clock;
pub mod ids;
pub mod logical;

pub use ids::SlintId;
pub use logical::{IntrospectionItem, SlintTreeBuilder, role_of_kind};
