//! The masonry inspector overlay (§13 "masonry/freya: overlay layer —
//! both have native overlay/layer stacks").
//!
//! Masonry composites through the `VisualLayerPlan` layer stack; the
//! overlay is the adapter-level composition *on top of the app's scene*.
//! This module converts the framework-free [`OverlayPlan`] into
//! [`OverlayDrawOp`]s — the exact list the harness renders at the top
//! layer at present time (draw-at-present, no extra pass, §15.4). The
//! vello-scene attachment rides the adapter integration pass; the plan →
//! op conversion is the stable surface.

use guiwright_inspect::overlay::{Badge, OverlayPlan};

/// One overlay draw op, window-relative physical px (§9).
#[derive(Clone, Debug, PartialEq)]
pub enum OverlayDrawOp {
    /// Filled + outlined hover rectangle.
    HoverOutline { rect: [f64; 4] },
    /// Badge text centered at a point.
    Badge { text: String, at: [f64; 2] },
}

/// Convert the plan to draw ops (document order).
pub fn overlay_draw_ops(plan: &OverlayPlan) -> Vec<OverlayDrawOp> {
    let mut ops = Vec::new();
    if let Some(rect) = &plan.hover {
        ops.push(OverlayDrawOp::HoverOutline {
            rect: [rect.x0, rect.y0, rect.x1, rect.y1],
        });
    }
    for Badge { text, at } in &plan.badges {
        ops.push(OverlayDrawOp::Badge {
            text: text.clone(),
            at: [at.0, at.1],
        });
    }
    ops
}

/// The overlay layer id inside the `VisualLayerPlan` (top-most overlay).
pub const OVERLAY_LAYER_ID: &str = "guiwright-inspect-overlay";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_converts_to_draw_ops_in_order() {
        let plan = OverlayPlan {
            hover: Some(guiwright_inspect::overlay::OverlayRect::new(
                10.0, 20.0, 90.0, 60.0,
            )),
            badges: vec![
                Badge {
                    text: "Button #save".into(),
                    at: (50.0, 40.0),
                },
                Badge {
                    text: "TextInput".into(),
                    at: (100.0, 40.0),
                },
            ],
        };
        let ops = overlay_draw_ops(&plan);
        assert_eq!(ops.len(), 3);
        let [x0, y0, x1, y1] = match &ops[0] {
            OverlayDrawOp::HoverOutline { rect } => *rect,
            OverlayDrawOp::Badge { .. } => panic!("expected HoverOutline, got Badge"),
        };
        assert_eq!((x0, y0, x1, y1), (10.0, 20.0, 90.0, 60.0));
        assert!(matches!(&ops[1], OverlayDrawOp::Badge { text, .. } if text.contains("#save")));
        assert!(matches!(ops[2], OverlayDrawOp::Badge { .. }));
    }

    #[test]
    fn empty_plan_produces_no_ops() {
        assert_eq!(overlay_draw_ops(&OverlayPlan::default()).len(), 0);
    }
}
