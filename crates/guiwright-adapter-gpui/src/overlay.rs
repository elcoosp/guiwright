//! **The gpui inspector overlay host (§13) — the mock binding.**
//!
//! The real gpui surface the §13 overlay routes through is the
//! test-support painted-quad scene: `Window::paint_quad(quad)` records
//! a [`Quad`](gpui) into the window's paint list and
//! `Window::painted_quads()` reads it back (the same scene digest the
//! glue's frame hashes ride, §15.5). The `test-support` feature lives
//! only on zed's git source and its platform stack needs system
//! libraries a default build must never require (`contract.rs`), so the
//! binding proof ships the **mock types that exactly mirror the real
//! APIs** (the plan's fallback for upstream blockers, m86):
//!
//! * [`PaintedQuad`] mirrors the quad record the real scene carries —
//!   bounds, fill color, corner radius — plain data the digest and the
//!   scripted raster both consume.
//! * [`OverlayHost`] mirrors the paint route — `paint_quads(&plan)` is
//!   the `paint_quad` call sequence the glue's `show_overlay` routes,
//!   `painted_quads()` is the scene readback, every draw lands in the
//!   §13 draw-call log.
//! * [`composite`] rasterizes the quads into a scripted RGBA surface —
//!   the stand-in for the test-support raster (`rendered_frame`), so
//!   the mirror example proves the §13 pixel-diff route end to end.
//!
//! The binding contract: **when the real framework API matches the mock
//! signatures** (a test-support build with a headless raster), the glue
//! swaps the mock for the real call sites unchanged — same arguments,
//! same quad list, same log. The `compile_fail` doc test pins the
//! negative side (a window without the paint surface cannot bind).

use std::hash::Hasher;

use guiwright_inspect::overlay::OverlayPlan;
use twox_hash::XxHash3_64;

/// Outline color — the kittest-ish debug blue the egui overlay paints.
pub const OUTLINE: [u8; 4] = [0x4f, 0x9c, 0xf9, 255];
/// Hover fill — a translucent blue wash under the outline.
pub const HOVER_WASH: [u8; 4] = [0x4f, 0x9c, 0xf9, 24];
/// Badge chip background (dim ink) — the chip rides the paint log, the
/// pixel diff proves the paint (the freya overlay's metric).
pub const BADGE_BG: [u8; 4] = [0x1c, 0x1f, 0x24, 220];
/// Outline stroke width (px).
const OUTLINE_PX: f64 = 2.0;
/// Badge chip metrics (the freya overlay's chip shape).
const BADGE_WIDTH: f64 = 64.0;
const BADGE_HEIGHT: f64 = 16.0;
const BADGE_RADIUS: f32 = 4.0;

/// One recorded quad — the mock of the real painted-quad record (bounds
/// + fill + corner radius, straight-alpha RGBA8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintedQuad {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub color: [u8; 4],
    pub radius: f32,
}

impl PaintedQuad {
    /// Fold the quad into the §15.5 frame digest — the hash-only path
    /// the pixel-less adapters ride (the overlay layer joins the scene
    /// digest, so "wait for visual change" sees the overlay).
    pub fn fold_into(&self, hasher: &mut XxHash3_64) {
        for value in [self.x0, self.y0, self.x1, self.y1] {
            hasher.write(&value.to_bits().to_le_bytes());
        }
        hasher.write(&self.color);
        hasher.write(&self.radius.to_bits().to_le_bytes());
    }
}

/// The mock overlay host — the paint route the §13 overlay goes through,
/// mirroring the real test-support surface one-to-one (`paint_quad` →
/// scene → `painted_quads` + the draw-call log).
#[derive(Debug, Default)]
pub struct OverlayHost {
    quads: Vec<PaintedQuad>,
    log: Vec<String>,
}

impl OverlayHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Paint one overlay plan: the `paint_quad` call sequence for the
    /// §13 rows — hover wash, four outline edges, one chip per badge —
    /// recorded into the host's scene (the readback) and the log.
    /// Returns the quads painted this call (what the glue's window
    /// route would hand the renderer).
    pub fn paint_quads(&mut self, plan: &OverlayPlan) -> Vec<PaintedQuad> {
        let mut painted = Vec::new();
        if let Some(hover) = &plan.hover {
            let (x0, y0) = (hover.x0, hover.y0);
            let (x1, y1) = (hover.x1, hover.y1);
            let wash = PaintedQuad {
                x0,
                y0,
                x1,
                y1,
                color: HOVER_WASH,
                radius: 0.0,
            };
            self.quads.push(wash);
            painted.push(wash);
            for edge in outline_edges(x0, y0, x1, y1) {
                self.quads.push(edge);
                painted.push(edge);
            }
            self.log
                .push(format!("outline {x0:.0},{y0:.0} → {x1:.0},{y1:.0}"));
        }
        for badge in &plan.badges {
            let quad = PaintedQuad {
                x0: badge.at.0,
                y0: badge.at.1,
                x1: badge.at.0 + BADGE_WIDTH,
                y1: badge.at.1 + BADGE_HEIGHT,
                color: BADGE_BG,
                radius: BADGE_RADIUS,
            };
            self.quads.push(quad);
            painted.push(quad);
            self.log.push(format!(
                "badge '{}' @ {:.0},{:.0}",
                badge.text, badge.at.0, badge.at.1
            ));
        }
        painted
    }

    /// The scene readback (the `painted_quads()` mirror) — every quad
    /// the host painted since the last clear.
    pub fn painted_quads(&self) -> &[PaintedQuad] {
        &self.quads
    }

    /// The §13 draw-call evidence so far (one line per draw).
    pub fn paint_log(&self) -> &[String] {
        &self.log
    }

    /// Drain the log (the adapters' `take_paint_log` shape).
    pub fn take_paint_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// Clear the scene + log (the `DisableOverlay` reset — the next
    /// frame is the bare app again).
    pub fn clear(&mut self) {
        self.quads.clear();
        self.log.clear();
    }

    /// Whether anything is in the scene (the empty-plan no-op the
    /// session's refresh loop hits on sessions without a plan).
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// The scene digest (§15.5): the strided fold over the quad list —
    /// the hash-only adapters fold this into their frame stamp so the
    /// overlay registers as a visual change.
    pub fn scene_digest(&self) -> u64 {
        let mut hasher = XxHash3_64::default();
        for quad in &self.quads {
            quad.fold_into(&mut hasher);
        }
        hasher.finish()
    }
}

/// The four outline edges (the wash inside them stays translucent).
fn outline_edges(x0: f64, y0: f64, x1: f64, y1: f64) -> [PaintedQuad; 4] {
    let top = PaintedQuad {
        x0,
        y0,
        x1,
        y1: y0 + OUTLINE_PX,
        color: OUTLINE,
        radius: 0.0,
    };
    let bottom = PaintedQuad {
        x0,
        y0: y1 - OUTLINE_PX,
        x1,
        y1,
        color: OUTLINE,
        radius: 0.0,
    };
    let left = PaintedQuad {
        x0,
        y0,
        x1: x0 + OUTLINE_PX,
        y1,
        color: OUTLINE,
        radius: 0.0,
    };
    let right = PaintedQuad {
        x0: x1 - OUTLINE_PX,
        y0,
        x1,
        y1,
        color: OUTLINE,
        radius: 0.0,
    };
    [top, bottom, left, right]
}

/// Rasterize the plan's quads into a scripted RGBA8 surface — the
/// straight-alpha source-over blend the test-support raster stands in
/// for (the mirror's `render_pixels` calls this after the app's own
/// paint, so the overlay composites at present time, §15.4).
pub fn composite(plan: &OverlayPlan, pixels: &mut [u8], width: u32, height: u32) {
    let mut host = OverlayHost::new();
    let quads = host.paint_quads(plan);
    for quad in quads {
        blend_quad(pixels, width, height, &quad);
    }
}

/// One quad, source-over: `out = src·a + dst·(1 − a)` per channel, the
/// same composition the pixel-painter adapters give.
fn blend_quad(pixels: &mut [u8], width: u32, height: u32, quad: &PaintedQuad) {
    let x0 = clamp_px(quad.x0, width);
    let y0 = clamp_px(quad.y0, height);
    let x1 = clamp_px(quad.x1, width);
    let y1 = clamp_px(quad.y1, height);
    let [sr, sg, sb, sa] = quad.color;
    let alpha = f64::from(sa) / 255.0;
    for y in y0..y1.min(height) {
        for x in x0..x1.min(width) {
            // Rounded corners cut like the paint path does.
            if quad.radius > 0.0 && !inside_radius(quad, x, y) {
                continue;
            }
            let i = ((y * width + x) * 4) as usize;
            let (dr, dg, db) = (pixels[i], pixels[i + 1], pixels[i + 2]);
            pixels[i] = blend_channel(sr, dr, alpha);
            pixels[i + 1] = blend_channel(sg, dg, alpha);
            pixels[i + 2] = blend_channel(sb, db, alpha);
            pixels[i + 3] = 255;
        }
    }
}

/// A clamped pixel coordinate: negative values floor to 0, values past
/// the surface clamp to it, and the guarded float→int step rides the
/// §10 pixel-ramp rule (round-then-truncate, the workspace's documented
/// conversion).
fn clamp_px(value: f64, upper: u32) -> u32 {
    let clamped = value.max(0.0).min(f64::from(upper));
    u32::try_from(clamped.round() as i64).unwrap_or(0)
}

fn inside_radius(quad: &PaintedQuad, x: u32, y: u32) -> bool {
    let r = f64::from(quad.radius);
    let (px, py) = (f64::from(x), f64::from(y));
    let near_left = px < quad.x0 + r;
    let near_right = px > quad.x1 - r;
    let near_top = py < quad.y0 + r;
    let near_bottom = py > quad.y1 - r;
    let (cx, cy) = match (near_left, near_right, near_top, near_bottom) {
        (true, _, true, _) => (quad.x0 + r, quad.y0 + r),
        (false, true, true, _) => (quad.x1 - r, quad.y0 + r),
        (true, _, false, true) => (quad.x0 + r, quad.y1 - r),
        (false, true, false, true) => (quad.x1 - r, quad.y1 - r),
        _ => return true,
    };
    let (dx, dy) = (px - cx, py - cy);
    dx * dx + dy * dy <= r * r
}

fn blend_channel(src: u8, dst: u8, alpha: f64) -> u8 {
    let value = f64::from(src) * alpha + f64::from(dst) * (1.0 - alpha);
    let clamped = value.round().max(0.0).min(f64::from(u8::MAX));
    u8::try_from(clamped as i64).unwrap_or(0)
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
    fn paint_quads_records_the_plan_into_the_scene() {
        let mut host = OverlayHost::new();
        let painted = host.paint_quads(&hover_plan());
        // 1 wash + 4 edges + 1 chip.
        assert_eq!(painted.len(), 6, "wash + four edges + chip: {painted:?}");
        assert_eq!(host.painted_quads().len(), 6, "the scene readback");
        assert_eq!(painted[0].color, HOVER_WASH, "translucent blue wash");
        assert_eq!(painted[1].color, OUTLINE, "kittest-ish debug blue");
        assert!(painted[5].radius > 0.0, "the chip is rounded");
        assert!(!host.is_empty());
        assert_eq!(host.scene_digest(), host.scene_digest(), "digest stable");
    }

    #[test]
    fn the_draw_log_names_the_draws() {
        let mut host = OverlayHost::new();
        host.paint_quads(&hover_plan());
        let log = host.paint_log();
        assert_eq!(log.len(), 2, "one outline + one badge: {log:?}");
        assert!(log[0].starts_with("outline 16,16"), "{log:?}");
        assert!(log[1].contains("badge 'button'"), "{log:?}");
        let taken = host.take_paint_log();
        assert_eq!(taken.len(), 2);
        assert!(host.paint_log().is_empty(), "take drains the log");
        host.clear();
        assert!(host.is_empty(), "clear empties the scene");
    }

    #[test]
    fn the_empty_plan_paints_nothing() {
        let mut host = OverlayHost::new();
        let painted = host.paint_quads(&OverlayPlan::default());
        assert!(painted.is_empty(), "no rows → no quads: {painted:?}");
        assert!(host.is_empty(), "no draws → no scene");
    }

    #[test]
    fn the_scene_digest_moves_with_the_plan() {
        let mut host = OverlayHost::new();
        let bare = host.scene_digest();
        host.paint_quads(&hover_plan());
        assert_ne!(host.scene_digest(), bare, "the overlay moved the digest");
        host.clear();
        assert_eq!(host.scene_digest(), bare, "clear restores the bare scene");
    }

    #[test]
    fn composite_moves_pixels_and_restores_after_clear() {
        let width = 200_u32;
        let height = 100_u32;
        let base = vec![24_u8; (width * height * 4) as usize];

        let mut empty_plan = base.clone();
        composite(&OverlayPlan::default(), &mut empty_plan, width, height);
        assert_eq!(empty_plan, base, "the empty plan composites nothing");

        let mut overlaid = base.clone();
        composite(&hover_plan(), &mut overlaid, width, height);
        let diff = base
            .iter()
            .zip(overlaid.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(diff > 0, "the overlay painted onto the scene: {diff} bytes");
    }
}
