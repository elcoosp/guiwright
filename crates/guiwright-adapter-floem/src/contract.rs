//! The downstream glue contract + upstream patch checklist (§4.7).
//!
//! This crate ships **framework-free** (the `guiwright-adapter-slint`
//! precedent, §18): floem 0.2.0 on crates.io predates the
//! `HeadlessHarness` (§4.7 recon cites `src/headless.rs:101` at
//! `lapce/floem@1351ffb`, post-0.2.0 main), so the harness binding
//! compiles downstream until the harness releases. The binding itself is
//! small and fully specified here — [`FloemAdapterContract`] is the data
//! shape a downstream glue implements, and the upstream patches are
//! enumerated below (design details: `docs/m3-floem-contributions.md`).
//!
//! # Upstream patch 1 — `test_id(...)` decorator (§4.7 contribution 1a)
//!
//! A decorator view + a global registry, mirroring the `debug_name` slot:
//!
//! ```ignore
//! // floem (upstream): src/views/test_id.rs
//! pub fn test_id(id: impl Into<String>, child: impl IntoView) -> TestId
//! // registration: TEST_IDS.with(|r| r.register(id, view_id))
//! ```
//!
//! The registry semantics this crate requires live in
//! [`crate::ids::TestIdRegistry`] (strict-mode duplicates, ABA
//! generations, removal hooks) — the upstream map is the thin half.
//!
//! # Upstream patch 2 — tiny-skia headless `PaintState` (§4.7 contribution 2)
//!
//! When the window is headless, initialize `PaintState` with
//! [`crate::paint::PaintState`] instead of a GPU surface, and route
//! `render_frame` through [`crate::paint::PaintOp`]s:
//!
//! ```ignore
//! // floem (upstream): src/window/handle.rs, PaintState::new
//! if headless {
//!     PaintState::TinySkia(guiwright_paint::PaintState::new(w, h)?)
//! } else { /* existing GPU path */ }
//! // present: translate the scene into PaintOps (same dispatch the
//! // inspector `Capture` path uses, `Renderer::finish()`)
//! ```
//!
//! # The downstream binding (once the harness releases)
//!
//! ```ignore
//! pub struct FloemAdapter {
//!     meta: AdapterMeta,               // caps.logical_snapshot = true
//!     harness: floem::HeadlessHarness, // §4.7 Embed
//!     tree: FloemLogicalTree,
//!     test_ids: TestIdRegistry,
//!     queued: VecDeque<floem::Event>,  // input() rendezvous queue (§5.1)
//! }
//!
//! impl FrameworkAdapter for FloemAdapter {
//!     // pump: one harness pass → FrameStamp (scene hash from the
//!     // PaintState digest, §15.5)
//!     // tree_update: None (Level-2)
//!     // logical_snapshot: walk the view tree into ViewReports
//!     //   (get_visual_rect + registries) → FloemLogicalTree::rebuild
//!     // act: SemAction → spatial input at the node's visual rect
//!     //   (§8.4; the harness's click/keyboard helpers, §4.7)
//!     // input: translate() → dispatch_event per step
//!     // clock: clock::plan() → harness ticks (§14)
//! }
//! ```

/// The capability + routing table a downstream floem glue implements
/// (the §6.2 matrix row, floem column). Kept as data so the contract is
/// testable and the glue cannot silently drift from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloemAdapterContract {
    /// Level-2: no AccessKit of its own — [`crate::logical`] synthesizes
    /// the update.
    pub tree_update: bool,
    /// Geometry + role/label polling (§4.7).
    pub logical_snapshot: bool,
    /// `dispatch_event` headless / `route_event` live (§4.7).
    pub input: bool,
    /// No GPU surface in headless mode until patch 2 lands.
    pub last_frame: bool,
    /// `true` **with** the tiny-skia `PaintState` patch (§4.7 "the
    /// contribution"); `false` without it.
    pub render_frame_with_paint_state_patch: bool,
    /// Harness-timer ticks with virtual time (§4.7, §14).
    pub clock: bool,
}

impl Default for FloemAdapterContract {
    fn default() -> Self {
        Self {
            tree_update: false,
            logical_snapshot: true,
            input: true,
            last_frame: false,
            render_frame_with_paint_state_patch: true,
            clock: true,
        }
    }
}

impl FloemAdapterContract {
    /// The caps the adapter advertises **without** the upstream patches
    /// (what a glue built against floem 0.2.0 can honestly report).
    pub fn caps_pre_patches(self) -> guiwright_driver::adapter::Caps {
        guiwright_driver::adapter::Caps {
            tree_update: self.tree_update,
            logical_snapshot: self.logical_snapshot,
            input: self.input,
            last_frame: self.last_frame,
            render_frame: false,
            clock: self.clock,
        }
    }

    /// The caps **with** the tiny-skia `PaintState` patch (§4.7
    /// contribution 2) — the row §11's floem line promises.
    pub fn caps_with_paint_state_patch(self) -> guiwright_driver::adapter::Caps {
        guiwright_driver::adapter::Caps {
            render_frame: self.render_frame_with_paint_state_patch,
            ..self.caps_pre_patches()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_matches_the_62_matrix_floem_column() {
        let contract = FloemAdapterContract::default();
        // §6.2 floem: tree_update ❌ → logical_snapshot; input ✅;
        // last_frame ⚠️ capture path; render_frame ❌ → contribute
        // tiny-skia; clock ⚠️ harness timers.
        assert!(!contract.tree_update);
        assert!(contract.logical_snapshot);
        assert!(contract.input);
        assert!(!contract.last_frame);
        assert!(!contract.caps_pre_patches().render_frame);
        assert!(contract.caps_with_paint_state_patch().render_frame);
        assert!(contract.clock);
    }
}
