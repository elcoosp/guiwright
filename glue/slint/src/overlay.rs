//! The slint inspector overlay (§4.4 "top-level overlay window", §13, m84).
//!
//! Slint's software renderer exposes no adapter-level composition slot —
//! no egui painter layer, no masonry/freya overlay stack — so the §13
//! overlay plan renders in a **borderless second `.slint` component
//! window** registered in the same `TestingBackend` thread-local window
//! registry as the app window. The overlay mirrors the app window's
//! geometry (the §9 window-relative rects land 1:1 in logical px at the
//! testing backend's scale factor 1), draws the hover outline + role/
//! TestId badges as real slint elements, and is captured through the same
//! `Window::take_snapshot` raster path the app window uses. Compositing
//! blends that snapshot onto the presented frame at capture time with
//! straight-alpha source-over — the software renderer's snapshot keeps a
//! fully transparent background where nothing painted, so only the drawn
//! overlay moves the pixels.
//!
//! The plan stays *data* ([`OverlayPlan`], §15.4): showing it never
//! triggers an extra app pass. Every draw records into the paint log —
//! the §13 draw-call evidence the pixel-diff tests assert against (the
//! same log shape the egui/masonry/freya overlays keep).

use slint::{ModelRc, SharedString, VecModel};

use guiwright_driver::adapter::AdapterError;
use guiwright_inspect::overlay::{Badge, OverlayPlan, OverlayRect};

/// Badge chip metrics (logical px; the same numbers the freya chips ride —
/// the 11px caption, 6.5px per character, a 16px-tall chip).
const BADGE_CHAR_W: f64 = 6.5;

slint::slint! {
    // The 1.19 language edition pin (the compiler's deprecation notice):
    // behavior changes up to 1.19 apply, so the window's geometry/color
    // semantics are pinned to the release train the glue documents (§4.4
    // caveats: semver-exempt internals → pinned per release train). The
    // attribute must lead the macro body — it is a file-level pin.
    #[edition = "1.19"]
    export struct GwOverlayOutline {
        x: length,
        y: length,
        width: length,
        height: length,
    }

    export struct GwOverlayBadge {
        x: length,
        y: length,
        width: length,
        text: string,
    }

    // The §4.4 overlay window: frameless and always-on-top by intent —
    // the testing backend's `MinimalSoftwareWindow` is a no-window-manager
    // adapter, so no compositor stands behind these hints. The
    // transparent background keeps every unpainted pixel at alpha 0 in
    // the snapshot (the software renderer's premultiplied clear), which
    // is what makes the source-over blend a no-op outside the drawn
    // overlay.
    export component GwOverlayWindow inherits Window {
        no-frame: true;
        always-on-top: true;
        background: transparent;
        in property <[GwOverlayOutline]> outlines;
        in property <[GwOverlayBadge]> badges;

        for outline in outlines: Rectangle {
            x: outline.x;
            y: outline.y;
            width: outline.width;
            height: outline.height;
            background: rgba(79, 156, 249, 0.094);
            border-width: 1.5px;
            border-color: #4f9cf9;
        }

        for badge in badges: Rectangle {
            x: badge.x - badge.width / 2;
            y: badge.y - 8px;
            width: badge.width;
            height: 16px;
            background: rgba(28, 31, 36, 0.86);
            border-width: 1px;
            border-color: #4f9cf9;
            Text {
                width: 100%;
                height: 100%;
                vertical-alignment: center;
                horizontal-alignment: center;
                color: #dfe3e8;
                font-size: 11px;
                overflow: elide;
                text: badge.text;
            }
        }
    }
}

/// Badge chip width for a caption (the freya chip's metrics: 6.5px per
/// character + 8px padding).
fn badge_width(text: &str) -> f64 {
    BADGE_CHAR_W * text.chars().count() as f64 + 8.0
}

/// The §13 draw-call evidence for one plan (the freya log format — the
/// pixel-diff tests assert the paint, this log names it).
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

/// Straight-alpha source-over: blend one overlay snapshot (`rgba8`,
/// straight alpha as the software renderer's public `take_snapshot`
/// exposes) onto the presented frame (opaque). Transparent overlay
/// pixels leave the frame untouched — the overlay window's transparent
/// background paints nothing, so only drawn outlines/badges move bytes.
pub fn composite_over(over: &[u8], base: &mut [u8]) {
    let n = over.len().min(base.len());
    for (o, b) in over[..n].chunks_exact(4).zip(base[..n].chunks_exact_mut(4)) {
        let alpha = u32::from(o[3]);
        if alpha == 0 {
            continue;
        }
        if alpha == 255 {
            b[..3].copy_from_slice(&o[..3]);
            continue;
        }
        let inv = 255 - alpha;
        for (oc, bc) in o[..3].iter().zip(b[..3].iter_mut()) {
            *bc = ((u32::from(*oc) * alpha + u32::from(*bc) * inv) / 255) as u8;
        }
        b[3] = 255;
    }
}

/// The overlay host: owns the second window (created lazily on the first
/// plan, dropped when the session clears), the geometry mirror, and the
/// paint log.
#[derive(Default)]
pub struct OverlayHost {
    window: Option<GwOverlayWindow>,
    /// The last plan, kept so a resize can re-mirror the geometry without
    /// a new plan from the session.
    plan: Option<OverlayPlan>,
    paint_log: Vec<String>,
}

impl std::fmt::Debug for OverlayHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayHost")
            .field("shown", &self.window.is_some())
            .field("plan", &self.plan)
            .field("paint_log_len", &self.paint_log.len())
            .finish()
    }
}

impl OverlayHost {
    /// Show (or refresh) the overlay for `plan`, mirroring the app
    /// window's logical `size`. An empty plan is still shown — the
    /// session refreshed while nothing hoverable is outlined — the window
    /// simply paints nothing.
    pub fn show(&mut self, plan: &OverlayPlan, size: (f64, f64)) -> Result<(), AdapterError> {
        if self.window.is_none() {
            let window = GwOverlayWindow::new()
                .map_err(|err| AdapterError::Internal(format!("slint overlay window: {err}")))?;
            window
                .show()
                .map_err(|err| AdapterError::Internal(format!("slint overlay show: {err}")))?;
            self.window = Some(window);
        }
        let window = self.window.as_ref().expect("overlay window shown");
        self.plan = Some(plan.clone());
        window
            .window()
            .set_size(slint::LogicalSize::new(size.0 as f32, size.1 as f32));
        window.set_outlines(ModelRc::new(VecModel::from(outline_rows(plan))));
        window.set_badges(ModelRc::new(VecModel::from(badge_rows(plan))));
        self.paint_log.extend(paint_log(plan));
        Ok(())
    }

    /// End the overlay session: the window leaves the registry and the
    /// paint log drains (nothing is drawn after this).
    pub fn clear(&mut self) {
        self.window = None;
        self.plan = None;
        self.paint_log.clear();
    }

    /// Re-mirror the app window's size (a `render_frame` resize while the
    /// session is on) with the stored plan.
    pub fn mirror(&mut self, size: (f64, f64)) -> Result<(), AdapterError> {
        if self.window.is_some()
            && let Some(plan) = self.plan.clone()
        {
            return self.show(&plan, size);
        }
        Ok(())
    }

    /// Blend the overlay window's snapshot onto the presented frame. A
    /// failed capture leaves the frame untouched (the overlay silently
    /// absent — the pixel-diff evidence catches it either way, as with
    /// the freya composite).
    pub fn composite(&mut self, base: &mut [u8]) {
        let Some(window) = &self.window else {
            return;
        };
        let Ok(snapshot) = window.window().take_snapshot() else {
            return;
        };
        composite_over(snapshot.as_bytes(), base);
    }

    /// Drain the paint log; `None` when nothing was drawn since the last
    /// take (or after a `clear`).
    pub fn take_paint_log(&mut self) -> Option<Vec<String>> {
        if self.paint_log.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.paint_log))
        }
    }
}

/// One hover-outline row per outlined rect (§13 "hover outlines"; the
/// plan carries at most the hovered node's rect today).
fn outline_rows(plan: &OverlayPlan) -> Vec<GwOverlayOutline> {
    plan.hover.iter().map(outline_row).collect()
}

fn outline_row(rect: &OverlayRect) -> GwOverlayOutline {
    GwOverlayOutline {
        x: rect.x0 as f32,
        y: rect.y0 as f32,
        width: ((rect.x1 - rect.x0).max(1.0)) as f32,
        height: ((rect.y1 - rect.y0).max(1.0)) as f32,
    }
}

/// One chip row per badge (§13 "badges"): a filled rect + caption pinned
/// at the node's laid-out center.
fn badge_rows(plan: &OverlayPlan) -> Vec<GwOverlayBadge> {
    plan.badges
        .iter()
        .map(|badge: &Badge| GwOverlayBadge {
            x: badge.at.0 as f32,
            y: badge.at.1 as f32,
            width: badge_width(&badge.text) as f32,
            text: SharedString::from(badge.text.as_str()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_width_matches_the_chip_metrics() {
        assert!((badge_width("Button") - (BADGE_CHAR_W * 6.0 + 8.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn plan_rows_carry_the_rects_and_chips() {
        let plan = OverlayPlan {
            hover: Some(OverlayRect::new(10.0, 20.0, 90.0, 60.0)),
            badges: vec![Badge {
                text: "Button #increment".into(),
                at: (50.0, 40.0),
            }],
        };
        let outlines = outline_rows(&plan);
        assert_eq!(outlines.len(), 1);
        assert_eq!(
            (
                outlines[0].x,
                outlines[0].y,
                outlines[0].width,
                outlines[0].height
            ),
            (10.0, 20.0, 80.0, 40.0)
        );
        let badges = badge_rows(&plan);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].text, "Button #increment");
        assert_eq!(badges[0].x, 50.0);
        assert!(badges[0].width > 0.0);
    }

    #[test]
    fn paint_log_names_the_outline_and_badges() {
        let plan = OverlayPlan {
            hover: Some(OverlayRect::new(10.0, 20.0, 90.0, 60.0)),
            badges: vec![Badge {
                text: "Button #increment".into(),
                at: (50.0, 40.0),
            }],
        };
        let log = paint_log(&plan);
        assert_eq!(log[0], "outline 10,20→90,60");
        assert!(log[1].starts_with("badge \"Button #increment\""));
    }

    /// The blend contract: transparent overlay bytes pass the frame
    /// through; translucent ones mix; opaque ones replace. The base
    /// frames are opaque (the app snapshot's alpha is 255).
    #[test]
    fn composite_over_blends_by_alpha() {
        // Fully transparent overlay: the frame survives byte-for-byte.
        let mut base = vec![64u8, 64, 64, 255, 64, 64, 64, 255];
        composite_over(&[0, 0, 0, 0, 0, 0, 0, 0], &mut base);
        assert_eq!(base, vec![64u8, 64, 64, 255, 64, 64, 64, 255]);

        // Opaque overlay pixel: replaces the pixel wholesale.
        let mut base = vec![64u8, 64, 64, 255, 64, 64, 64, 255];
        composite_over(&[10, 20, 30, 255], &mut base[4..]);
        assert_eq!(&base[4..8], &[10, 20, 30, 255]);

        // Half-transparent overlay pixel: the midpoint mix.
        let mut base = vec![64u8, 64, 64, 255];
        composite_over(&[0, 0, 0, 128], &mut base);
        assert_eq!(base[0], 31, "64 * (255-128) / 255 = 31");
        assert_eq!(base[3], 255, "the presented frame stays opaque");
    }
}
