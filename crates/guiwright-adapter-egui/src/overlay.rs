//! The egui inspector overlay (§13 "Overlay: adapter-level composition —
//! egui: painter layer after the pass").
//!
//! The plan is *data* ([`OverlayPlan`]); rendering it never triggers an
//! extra pass — the adapter paints it inside the current pass's layer,
//! honoring the render-once invariant (§15.4). Call this at the end of
//! the app's update, with the context that owns the pass.

use egui::{Color32, FontId, Pos2, Rect, Shape, Stroke};

use guiwright_inspect::overlay::{Badge, OverlayPlan, OverlayRect};

/// Outline color (kittest-ish debug blue).
const OUTLINE: Color32 = Color32::from_rgb(0x4f, 0x9c, 0xf9);
/// Badge background (dim ink).
fn badge_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(0x1c, 0x1f, 0x24, 220)
}
/// Badge text.
const BADGE_FG: Color32 = Color32::from_rgb(0xdf, 0xe3, 0xe8);

/// Paint the overlay plan into the current egui pass (§13). The painter
/// comes from `ctx`'s top layer, so badges float above the app's widgets
/// without disturbing layout.
pub fn paint_overlay(ctx: &egui::Context, plan: &OverlayPlan) {
    if plan.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("guiwright-inspect-overlay"),
    ));

    if let Some(rect) = &plan.hover {
        let hover = to_egui_rect(rect);
        painter.rect_filled(
            hover,
            2.0,
            Color32::from_rgba_unmultiplied(0x4f, 0x9c, 0xf9, 24),
        );
        painter.rect_stroke(
            hover,
            2.0,
            Stroke::new(1.5, OUTLINE),
            egui::StrokeKind::Outside,
        );
    }

    for badge in &plan.badges {
        paint_badge(&painter, badge);
    }
}

fn paint_badge(painter: &egui::Painter, badge: &Badge) {
    let font = FontId::monospace(11.0);
    let galley = painter.layout_no_wrap(badge.text.clone(), font, BADGE_FG);
    let anchor = Pos2::new(badge.at.0 as f32, badge.at.1 as f32);
    let margin = 3.0;
    let bg = Rect::from_min_size(
        Pos2::new(
            anchor.x - galley.size().x / 2.0 - margin,
            anchor.y - galley.size().y / 2.0,
        ),
        galley.size() + 2.0 * egui::Vec2::splat(margin),
    );
    painter.rect_filled(bg, 3.0, badge_bg());
    painter.rect_stroke(bg, 3.0, Stroke::new(1.0, OUTLINE), egui::StrokeKind::Middle);
    painter.galley(
        Pos2::new(bg.min.x + margin, bg.min.y + margin),
        galley,
        BADGE_FG,
    );
}

/// Convert the plan's physical px rect to an egui rect.
fn to_egui_rect(rect: &OverlayRect) -> Rect {
    Rect::from_min_max(
        Pos2::new(rect.x0 as f32, rect.y0 as f32),
        Pos2::new(rect.x1 as f32, rect.y1 as f32),
    )
}

/// Convenience: shapes-only form for adapters that batch their own painter
/// output (returns the shapes instead of submitting them).
pub fn overlay_shapes(plan: &OverlayPlan) -> Vec<Shape> {
    let mut shapes = Vec::new();
    if let Some(rect) = &plan.hover {
        shapes.push(Shape::rect_stroke(
            to_egui_rect(rect),
            2.0,
            Stroke::new(1.5, OUTLINE),
            egui::StrokeKind::Outside,
        ));
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_inspect::overlay::Badge;

    #[test]
    fn empty_plan_paints_nothing() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        // No panic and no shapes on an empty plan.
        paint_overlay(&ctx, &OverlayPlan::default());
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        assert_eq!(overlay_shapes(&OverlayPlan::default()).len(), 0);
    }

    #[test]
    fn plan_shapes_convert_rects() {
        let plan = OverlayPlan {
            hover: Some(OverlayRect::new(10.0, 20.0, 90.0, 60.0)),
            badges: vec![Badge {
                text: "Button “Save”".into(),
                at: (50.0, 40.0),
            }],
        };
        let ctx = egui::Context::default();
        // Full paint inside a real pass (fonts load there, §15.4).
        ctx.begin_pass(egui::RawInput::default());
        paint_overlay(&ctx, &plan);
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let shapes = overlay_shapes(&plan);
        assert_eq!(shapes.len(), 1, "outline shape only; badges need galley");
    }
}
