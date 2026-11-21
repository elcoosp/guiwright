//! The freya inspector overlay (§13 "Overlay: adapter-level composition —
//! masonry/freya: overlay layer").
//!
//! The plan is *data* ([`OverlayPlan`]); rendering it never triggers an
//! extra pass — the adapter composites it **at present time** onto the
//! frame the runner's software-Skia pipeline just rasterized (§15.4): the
//! same Skia engine the app draws with paints the hover outline + role
//! badges into the frame's pixels. The accessibility tree stays untouched
//! (overlay-layer elements would pollute it — every freya element enters
//! the tree, so the composition is a draw pass, not a widget layer).
//!
//! Every draw records into the adapter's paint log — the §13 draw-call
//! evidence the pixel-diff tests assert against (M83).

use skia_safe::surfaces;
use skia_safe::{Color, Font, FontMgr, FontStyle, ImageInfo, Paint, PaintStyle, Rect};

use guiwright_inspect::overlay::{Badge, OverlayPlan, OverlayRect};

/// Outline color (the kittest-ish debug blue the egui overlay paints).
fn outline_color() -> Color {
    Color::from_argb(255, 0x4f, 0x9c, 0xf9)
}
/// Hover fill (translucent blue wash under the outline).
fn hover_fill_color() -> Color {
    Color::from_argb(24, 0x4f, 0x9c, 0xf9)
}
/// Badge background (dim ink).
fn badge_bg() -> Color {
    Color::from_argb(220, 0x1c, 0x1f, 0x24)
}
/// Badge text.
fn badge_fg() -> Color {
    Color::from_argb(255, 0xdf, 0xe3, 0xe8)
}
/// Badge chip metrics (logical px; the chip rides the paint log — the
/// pixel diff proves the paint).
const BADGE_FONT_SIZE: f32 = 11.0;
const BADGE_CHAR_W: f32 = 6.5;
const BADGE_HEIGHT: f32 = 16.0;

fn skia_rect(rect: &OverlayRect) -> Rect {
    Rect::new(
        rect.x0 as f32,
        rect.y0 as f32,
        rect.x1 as f32,
        rect.y1 as f32,
    )
}

/// The §13 draw-call evidence for one plan (outline + badge entries).
pub fn paint_log(plan: &OverlayPlan) -> Vec<String> {
    let mut log = Vec::new();
    if let Some(rect) = &plan.hover {
        log.push(format!(
            "outline {:.0},{:.0}→{:.0},{:.0}",
            rect.x0, rect.y0, rect.x1, rect.y1
        ));
    }
    for badge in &plan.badges {
        log.push(format!(
            "badge {:?} @ {:.0},{:.0}",
            badge.text, badge.at.0, badge.at.1
        ));
    }
    log
}

/// Composite the plan onto the frame's pixels — one Skia raster surface
/// over the app's own render, the overlay painted with the framework's
/// engine, then read back in place (§13 present-time composition).
pub fn composite(plan: &OverlayPlan, rgba: &mut [u8], width: u32, height: u32) {
    if plan.is_empty() {
        return;
    }
    let info = ImageInfo::new_n32_premul((width as i32, height as i32), None);
    let Some(mut surface) = surfaces::raster_n32_premul((width as i32, height as i32)) else {
        return;
    };
    let row_bytes = (width as usize) * 4;
    if !surface
        .canvas()
        .write_pixels(&info, rgba, row_bytes, (0, 0))
    {
        return;
    }
    let canvas = surface.canvas();

    if let Some(rect) = &plan.hover {
        let rect = skia_rect(rect);
        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_color(hover_fill_color());
        canvas.draw_rect(rect, &fill);
        let mut stroke = Paint::default();
        stroke.set_anti_alias(true);
        stroke.set_style(PaintStyle::Stroke);
        stroke.set_stroke_width(1.5);
        stroke.set_color(outline_color());
        canvas.draw_rect(rect, &stroke);
    }

    // The badge font rides the system font manager — the same default the
    // runner's own text stack uses (`FontMgr::default()` in
    // freya-engine's skia backend).
    let typeface = FontMgr::default().legacy_make_typeface(None, FontStyle::default());
    for badge in &plan.badges {
        paint_badge(canvas, &typeface, badge);
    }

    // A failed read-back leaves the frame pixels untouched (the overlay
    // silently absent) — the pixel-diff evidence catches it either way.
    let _ = canvas.read_pixels(&info, rgba, row_bytes, (0, 0));
}

/// One badge chip: a filled rect + caption, pinned at the node's
/// laid-out center. Returns the paint-log entry.
fn paint_badge(
    canvas: &skia_safe::Canvas,
    typeface: &Option<skia_safe::Typeface>,
    badge: &Badge,
) -> String {
    let width = BADGE_CHAR_W * badge.text.chars().count() as f32 + 8.0;
    let rect = Rect::new(
        (badge.at.0 - f64::from(width) / 2.0) as f32,
        (badge.at.1 - f64::from(BADGE_HEIGHT) / 2.0) as f32,
        (badge.at.0 + f64::from(width) / 2.0) as f32,
        (badge.at.1 + f64::from(BADGE_HEIGHT) / 2.0) as f32,
    );
    let mut bg = Paint::default();
    bg.set_anti_alias(true);
    bg.set_color(badge_bg());
    canvas.draw_rect(rect, &bg);
    let mut edge = Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(PaintStyle::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color(outline_color());
    canvas.draw_rect(rect, &edge);

    let mut fg = Paint::default();
    fg.set_anti_alias(true);
    fg.set_color(badge_fg());
    if let Some(typeface) = typeface {
        let font = Font::from_typeface(typeface.clone(), BADGE_FONT_SIZE);
        // Fixed baseline offset inside the chip — the caption is
        // single-line at a fixed size (the draw log carries the text).
        canvas.draw_str(
            &badge.text,
            (rect.left() + 4.0, rect.top() + 12.0),
            &font,
            &fg,
        );
    }
    format!(
        "badge {:?} @ {:.0},{:.0}",
        badge.text, badge.at.0, badge.at.1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The composite paints on a copy: the base frame survives unchanged
    /// when the plan is empty, and the hover outline alters it otherwise.
    #[test]
    fn composite_alters_the_frame_pixels() {
        let (width, height) = (40u32, 30u32);
        let base = vec![64u8; (width * height * 4) as usize];
        // Empty plan: the frame stays as the app rendered it.
        let mut frame = base.clone();
        composite(&OverlayPlan::default(), &mut frame, width, height);
        assert_eq!(frame, base, "empty plan composites nothing");

        // Hover outline: the frame pixels move.
        let plan = OverlayPlan {
            hover: Some(OverlayRect::new(4.0, 4.0, 36.0, 26.0)),
            badges: vec![Badge {
                text: "Button #save".into(),
                at: (20.0, 15.0),
            }],
        };
        let mut frame = base.clone();
        composite(&plan, &mut frame, width, height);
        let diff = base
            .iter()
            .zip(frame.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(diff > 0, "the overlay painted: {diff} bytes");
    }
}
