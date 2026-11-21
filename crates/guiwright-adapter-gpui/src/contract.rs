//! The downstream app glue contract (§4.5, §5.3, M2 spike doc §3).
//!
//! This crate ships **framework-free** (the `guiwright-adapter-slint`
//! precedent, §18): gpui 0.2.2 is on crates.io, but its Linux platform
//! stack needs system libraries (xkbcommon, wayland) a default build must
//! never require. The app-side wiring is small and fully specified here +
//! in `docs/m2-gpui-remote-spike.md` §3.
//!
//! # Launch-mode wiring (§5.3 GPUI row, spike doc §3)
//!
//! ```toml
//! # app Cargo.toml
//! [features]
//! gw-gpui = ["dep:guiwright-driver", "guiwright-driver/gpui", "gpui/test-support"]
//! ```
//!
//! ```ignore
//! // app main (remote mode, §5.3): GUIWRIGHT_DRIVER=uds:///tmp/app.gwp ./app
//! #[cfg(feature = "gw-gpui")]
//! fn guiwright_boot(cx: &mut gpui::App) {
//!     if let Ok(url) = std::env::var("GUIWRIGHT_DRIVER") {
//!         let seams = guiwright::app_seams();          // #[seam] output
//!         guiwright_driver::server::GwpServerHandle::spawn_with_seams(
//!             uds_path_from(url),
//!             || GpuiAdapter::attach(cx),              // this contract
//!             seams,
//!         )
//!         .expect("guiwright remote mode");
//!     }
//! }
//! ```
//!
//! # The adapter binding (the spike doc §2 contract)
//!
//! The M2 wire test (`clock_and_mock_domains_over_the_wire`) is the
//! launch-mode contract: replace `spawn_with_seams(.., || MockAdapter…)`
//! with the `GpuiAdapter` constructor and the wire conversation stays
//! byte-identical. The engine modules here are exactly what the glue
//! maps:
//!
//! | `FrameworkAdapter` method | GPUI mapping (§4.5) | Engine module |
//! |---|---|---|
//! | `meta().caps` | hybrid row (below) | [`GpuiCaps`] |
//! | `pump` | foreground-executor round, `dispatch_after(0)` waker | [`crate::clock`] |
//! | `tree_update` | `PlatformWindow::a11y_tree_update` while active | [`crate::a11y`] |
//! | `act` | a11y `ActionRequest` → synthesized mouse events (§8.4) | — |
//! | `input` | `Window::dispatch_event(PlatformInput)` | [`crate::input`] |
//! | `last_frame`/`render_frame` | `test-support` `rendered_frame` / headless readback | [`crate::frames`] |
//! | `clock` | seeded virtual dispatcher rounds | [`crate::clock`] |
//! | `run_in` | foreground executor + root entity context | [`crate::run_in`] |
//!
//! Open items recorded for the glue (spike doc §3, none block the
//! protocol): `ActionRequest` synthesis pacing (§9 table row), frame
//! hashes via [`crate::frames`] (§15.5), `AdapterMeta.framework_version`
//! reports the GPUI **platform** crate versions (§18 churn mitigation).
//!
//! # The canonical binding proof (`examples/downstream-glue/gpui`)
//!
//! The binding as **compilable, runnable code**:
//! `examples/downstream-glue/gpui` (its own workspace, dependency-free)
//! implements [`FrameworkAdapter`] over a gpui-shaped **mirror** — the
//! exact surface this contract's table maps, scripted
//! (`Window::dispatch_event(PlatformInput)`, per-frame
//! `a11y_tree_update`, `Entity` reads, `VisualTestContext`, the seeded
//! virtual dispatcher) — through this crate's engine modules:
//! [`crate::a11y::A11yHybrid`] with immediate activation,
//! [`crate::input::translate`] dispatch steps, [`crate::clock::plan`]
//! dispatcher rounds, [`crate::frames::FrameDigest`]'s strided xxh3 and
//! [`crate::run_in::RunInPlan`] foreground seams. Its integration test
//! drives a real `Session` end to end (resolve → click → `expect` → seam
//! → test-support raster screenshot) and the `compile_fail` doc test
//! pins the dispatch shape (a non-gpui window cannot bind). The mirror
//! keeps the seams honest: `SetValue` reports `Unsupported` with the
//! scheduler-blocker evidence below and `framework_version` names the
//! platform crates. `glue/gpui` remains the REAL-graph proof
//! (CI-exercised); this example is the anywhere-compilable one.
//!
//! # Upstream blockers (m80, file-level evidence)
//!
//! * **Fill** — the real `KeyDown` path panics inside zed's test
//!   scheduler (`scheduler/src/test_scheduler.rs:232`,
//!   `state.next_session_id.0 += 1` overflow, reproducible with three
//!   real `Keystroke` dispatches after a real focus click). The glue
//!   gates `SetValue` with `AdapterError::Unsupported` carrying the
//!   evidence; the real sequence (focus click + `Keystroke`s through
//!   `dispatch_event` into the focused `on_key_down`) is specified and
//!   waits on the upstream scheduler fix.
//! * **Pixels** — `Window::render_to_image` bails without a
//!   `HeadlessRenderer` and `TestPlatform::new` ships no factory
//!   (`platform/test/platform.rs` passes `None`); `render_frame`
//!   reports `Unsupported` with that evidence, and `last_frame` stays
//!   the hash-only §15.5 row until a `PlatformHeadlessRenderer` over
//!   lavapipe/llvmpipe can be configured.

/// The §6.2 capability row, GPUI column — kept as data so the glue cannot
/// silently drift from it (the floem contract pattern).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuiCaps {
    /// Level-1: unconditional AccessKit while active (§4.5 "AccessKit is
    /// unconditional").
    pub tree_update_when_active: bool,
    /// Hybrid: the logical engine is the *fallback* when a window is
    /// inactive (§6.1 hybrid).
    pub logical_snapshot_fallback: bool,
    pub input: bool,
    /// `test-support` `rendered_frame` only (§4.5).
    pub last_frame_with_test_support: bool,
    /// `WindowingRequest::Headless` + `PlatformHeadlessRenderer` (§4.5,
    /// §11 "needs `test-support`; Linux headless is native").
    pub render_frame_with_test_support: bool,
    /// ⚠️ test-support dispatcher (§6.2 GPUI row).
    pub clock: bool,
    /// State seams via the foreground executor (§14, spike §2).
    pub run_in: bool,
}

impl Default for GpuiCaps {
    fn default() -> Self {
        Self {
            tree_update_when_active: true,
            logical_snapshot_fallback: true,
            input: true,
            last_frame_with_test_support: true,
            render_frame_with_test_support: true,
            clock: true,
            run_in: true,
        }
    }
}

/// The adapter contract (the row the glue implements).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuiAdapterContract {
    pub caps: GpuiCaps,
}

impl GpuiAdapterContract {
    /// Caps advertised to a **remote** client (§7 `Welcome.caps`): the
    /// test-support rows depend on the app's feature graph — the glue
    /// passes `test_support` from `cfg`/feature detection at attach.
    pub fn caps_for(self, test_support: bool) -> guiwright_driver::adapter::Caps {
        guiwright_driver::adapter::Caps {
            tree_update: self.caps.tree_update_when_active,
            logical_snapshot: self.caps.logical_snapshot_fallback,
            input: self.caps.input,
            last_frame: test_support && self.caps.last_frame_with_test_support,
            render_frame: test_support && self.caps.render_frame_with_test_support,
            clock: self.caps.clock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_depend_on_the_test_support_feature_graph() {
        let contract = GpuiAdapterContract::default();
        // §11 GPUI row: "needs test-support; Linux headless is native".
        let with = contract.caps_for(true);
        assert!(with.tree_update);
        assert!(with.render_frame);
        assert!(with.clock);
        let without = contract.caps_for(false);
        assert!(!without.last_frame, "rendered_frame is test-support-gated");
        assert!(
            !without.render_frame,
            "headless readback is test-support-gated"
        );
        assert!(without.tree_update, "a11y is unconditional (§4.5)");
        assert!(without.clock, "the dispatcher plan maps regardless");
    }
}
