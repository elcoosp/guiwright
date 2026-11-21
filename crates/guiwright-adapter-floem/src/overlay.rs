//! **The floem inspector overlay host (§13) — the mock binding.**
//!
//! The real floem surface the §13 overlay routes through is the
//! headless `WindowHandle`'s present path: post-paint draw hooks
//! translate into the scene vocabulary the renderer rasterizes
//! (`contract.rs` upstream patch 2 — the same dispatch the live
//! inspector `Capture` path uses). Floem 0.2.0 predates that hook, so
//! this module ships the **mock types that exactly mirror the real
//! APIs** the binding touches (the plan's fallback for upstream
//! blockers, m86):
//!
//! * [`OverlayHost`] mirrors the real present-path hook —
//!   `present(&plan)` is the `paint ops` call sequence the glue's
//!   `show_overlay` routes, one [`PaintOp`] list per present, recorded
//!   into the §13 draw-call log.
//! * The ops ride [`crate::paint::PaintOp`] — the same vocabulary the
//!   tiny-skia [`crate::paint::PaintState`] rasterizes, so the real
//!   glue's `render_frame` extends its scene with `present`'s output
//!   and the overlay composites onto the presented pixels (the mirror
//!   example proves the route end to end).
//!
//! The binding contract: **when the real framework API matches the mock
//! signature** (floem ships the headless present hook), the glue swaps
//! the mock for the real call site unchanged — same arguments, same op
//! list, same log. The `compile_fail` doc test pins the negative side
//! (a surface without the present signature cannot bind).

use guiwright_inspect::overlay::OverlayPlan;

use crate::paint::{Color, PaintOp};

/// Outline color — the kittest-ish debug blue the egui overlay paints.
const OUTLINE: Color = Color::rgba(0x4f, 0x9c, 0xf9, 255);
/// Hover fill — a translucent blue wash under the outline.
const HOVER_WASH: Color = Color::rgba(0x4f, 0x9c, 0xf9, 24);
/// Badge chip background (dim ink) — the chip rides the paint log, the
/// pixel diff proves the paint (the freya overlay's metric).
const BADGE_BG: Color = Color::rgba(0x1c, 0x1f, 0x24, 220);
/// Outline stroke width (px).
const OUTLINE_PX: f32 = 2.0;
/// Badge chip metrics (the freya overlay's chip shape).
const BADGE_WIDTH: f32 = 64.0;
const BADGE_HEIGHT: f32 = 16.0;
const BADGE_RADIUS: f32 = 4.0;

/// The mock overlay host — the present-path hook the §13 overlay routes
/// through, mirroring the real floem surface one-to-one (post-paint →
/// paint ops + the draw-call log).
///
/// The host is stateless w.r.t. pixels (the surface rasterizes the ops
/// it returns) and keeps only the §13 draw-call evidence.
#[derive(Debug, Default)]
pub struct OverlayHost {
    log: Vec<String>,
}

impl OverlayHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Present one overlay plan: translate the §13 rows into the
    /// [`PaintOp`] list the tiny-skia [`crate::paint::PaintState`]
    /// rasterizes **on top of** the app's own scene (later ops
    /// overpaint — the source-over composition the other adapters'
    /// present-time composites give). Every draw records into the log.
    pub fn present(&mut self, plan: &OverlayPlan) -> Vec<PaintOp> {
        let mut ops = Vec::new();
        if let Some(hover) = &plan.hover {
            let (x0, y0) = (hover.x0 as f32, hover.y0 as f32);
            let (x1, y1) = (hover.x1 as f32, hover.y1 as f32);
            let (w, h) = ((x1 - x0).max(0.0), (y1 - y0).max(0.0));
            // The translucent wash under the outline (one full fill).
            ops.push(PaintOp::SolidRect {
                x: x0,
                y: y0,
                w,
                h,
                color: HOVER_WASH,
            });
            // The outline: four edge fills (the op vocabulary is fills —
            // the same edges the pixel painters stroke).
            ops.push(PaintOp::SolidRect {
                x: x0,
                y: y0,
                w,
                h: OUTLINE_PX,
                color: OUTLINE,
            });
            ops.push(PaintOp::SolidRect {
                x: x0,
                y: y1 - OUTLINE_PX,
                w,
                h: OUTLINE_PX,
                color: OUTLINE,
            });
            ops.push(PaintOp::SolidRect {
                x: x0,
                y: y0,
                w: OUTLINE_PX,
                h,
                color: OUTLINE,
            });
            ops.push(PaintOp::SolidRect {
                x: x1 - OUTLINE_PX,
                y: y0,
                w: OUTLINE_PX,
                h,
                color: OUTLINE,
            });
            self.log.push(format!(
                "outline {x0:.0},{y0:.0} → {x1:.0},{y1:.0} ({w:.0}x{h:.0})"
            ));
        }
        for badge in &plan.badges {
            // The chip: a rounded fill at the badge anchor (text rides
            // the log — fonts stay out of the headless raster by
            // design, the pixel diff proves the paint).
            ops.push(PaintOp::RoundedRect {
                x: badge.at.0 as f32,
                y: badge.at.1 as f32,
                w: BADGE_WIDTH,
                h: BADGE_HEIGHT,
                radius: BADGE_RADIUS,
                color: BADGE_BG,
            });
            self.log.push(format!(
                "badge '{}' @ {:.0},{:.0}",
                badge.text, badge.at.0, badge.at.1
            ));
        }
        ops
    }

    /// The §13 draw-call evidence so far (one line per draw).
    pub fn paint_log(&self) -> &[String] {
        &self.log
    }

    /// Drain the log (the adapters' `take_paint_log` shape).
    pub fn take_paint_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// Whether anything was ever presented (the empty-plan no-op the
    /// session's refresh loop hits on sessions without a plan).
    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// Clear the evidence (the `DisableOverlay` reset — the host keeps
    /// no pixel state, so only the log resets).
    pub fn clear(&mut self) {
        self.log.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_inspect::overlay::{Badge, OverlayRect};

    fn hover_plan() -> OverlayPlan {
        OverlayPlan {
            hover: Some(OverlayRect::new(16.0, 16.0, 152.0, 64.0)),
            badges: vec![Badge {
                text: "button".into(),
                at: (16.0, 0.0),
            }],
        }
    }

    #[test]
    fn present_translates_the_plan_into_paint_ops() {
        let mut host = OverlayHost::new();
        let ops = host.present(&hover_plan());
        // 1 wash + 4 outline edges + 1 badge chip.
        assert_eq!(ops.len(), 6, "wash + four edges + chip: {ops:?}");
        let PaintOp::SolidRect { color, .. } = ops[0] else {
            panic!("the wash rides first: {ops:?}")
        };
        assert_eq!(color, HOVER_WASH, "translucent blue wash");
        let PaintOp::SolidRect { color, .. } = ops[1] else {
            panic!("the top edge is an outline fill: {ops:?}")
        };
        assert_eq!(color, OUTLINE, "kittest-ish debug blue");
        let PaintOp::RoundedRect { color, .. } = ops[5] else {
            panic!("the chip is the rounded fill: {ops:?}")
        };
        assert_eq!(color, BADGE_BG, "dim ink chip");
    }

    #[test]
    fn the_draw_log_names_the_draws() {
        let mut host = OverlayHost::new();
        host.present(&hover_plan());
        let log = host.paint_log();
        assert_eq!(log.len(), 2, "one outline + one badge: {log:?}");
        assert!(log[0].starts_with("outline 16,16"), "{log:?}");
        assert!(log[1].contains("badge 'button'"), "{log:?}");
        assert!(!host.is_empty());
        let taken = host.take_paint_log();
        assert_eq!(taken.len(), 2);
        assert!(host.is_empty(), "take drains the log");
    }

    #[test]
    fn the_empty_plan_present_no_ops() {
        let mut host = OverlayHost::new();
        let ops = host.present(&OverlayPlan::default());
        assert!(ops.is_empty(), "no rows → no ops: {ops:?}");
        assert!(host.is_empty(), "no draws → no log");
    }

    #[test]
    fn the_ops_rasterize_onto_a_real_surface() {
        // The route proof: present's ops land on the tiny-skia
        // PaintState and MOVE pixels — the exact composition the
        // glue's render_frame performs.
        let mut host = OverlayHost::new();
        let mut state = crate::paint::PaintState::new(200, 100).expect("surface");
        state.paint(&[PaintOp::SolidRect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
            color: Color::rgba(24, 24, 28, 255),
        }]);
        let before = state.strided_hash();
        let ops = host.present(&hover_plan());
        state.paint(&ops);
        assert_ne!(
            state.strided_hash(),
            before,
            "the overlay ops moved the pixels"
        );
    }
}
