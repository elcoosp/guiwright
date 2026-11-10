//! **Contribution 2 (§4.7)** — the tiny-skia headless `PaintState`.
//!
//! Floem's headless `PaintState` never initializes
//! (`window/handle.rs:313-322` — it needs a GPU surface), so headless
//! screenshots are impossible today (§4.7 "pixels — the gap"; §11 floem
//! row "pixels via contributed tiny-skia `PaintState`"). The contribution:
//! an **offscreen tiny-skia surface** (`tiny-skia` is already compiled
//! into floem; `FLOEM_FORCE_TINY_SKIA=1` proves the renderer path) plus an
//! explicit [`PaintOp`] list the renderer routes from `render_frame`.
//!
//! Upstream patch shape (see `docs/m3-floem-contributions.md`): when the
//! window is headless, initialize `PaintState` with a
//! [`PaintState::new`] surface instead of a GPU-backed one, and translate
//! the scene into [`PaintOp`]s at present time — the same dispatch the
//! live inspector `Capture` path uses (`Renderer::finish() →
//! Option<ImageBrush>`).
//!
//! This module owns the surface + the op contract + determinism:
//! rasterization is pure CPU (no GPU, no fonts), the frame digest is the
//! §15.5 strided xxh3 (identical to the egui adapter's), and PNG export
//! goes through tiny-skia's own encoder (lossless, §10).

use std::hash::Hasher;

use twox_hash::xxhash3_64::Hasher as Xxh3;

/// A paint operation — the subset of floem's scene vocabulary the
/// headless surface rasterizes for M3. Extensible upstream without a
/// semver break (new variants, §7 append-only philosophy).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintOp {
    /// Axis-aligned solid fill.
    SolidRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
    },
    /// Rounded fill (floem `border_radius`).
    RoundedRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: Color,
    },
}

/// Straight-alpha RGBA8 color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PaintError {
    #[error("surface of {0}x{1} exceeds the pixel guard")]
    TooLarge(u32, u32),
    #[error("png encode error: {0}")]
    Png(String),
}

/// Max headless surface (matches the §12 trace ring guard scale).
const MAX_SURFACE: u32 = 8192;

/// The offscreen CPU surface (the headless `PaintState`).
#[derive(Debug)]
pub struct PaintState {
    pixmap: tiny_skia::Pixmap,
}

impl PaintState {
    pub fn new(width: u32, height: u32) -> Result<Self, PaintError> {
        if width == 0 || height == 0 || width > MAX_SURFACE || height > MAX_SURFACE {
            return Err(PaintError::TooLarge(width, height));
        }
        let pixmap =
            tiny_skia::Pixmap::new(width, height).ok_or(PaintError::TooLarge(width, height))?;
        Ok(Self { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// Clear + rasterize one frame (the `render_frame` entry point).
    pub fn paint(&mut self, ops: &[PaintOp]) {
        self.pixmap.fill(tiny_skia::Color::TRANSPARENT);
        for op in ops {
            match *op {
                PaintOp::SolidRect { x, y, w, h, color } => {
                    let rect = tiny_skia::Rect::from_xywh(x, y, w, h);
                    if let Some(rect) = rect {
                        self.pixmap.fill_rect(
                            rect,
                            &color_to_paint(color),
                            tiny_skia::Transform::identity(),
                            None,
                        );
                    }
                }
                PaintOp::RoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => {
                    let rect = tiny_skia::Rect::from_xywh(x, y, w, h);
                    if let Some(rect) = rect {
                        let r = radius.min(w / 2.0).min(h / 2.0);
                        if let Some(path) = round_rect_path(rect, r) {
                            self.pixmap.fill_path(
                                &path,
                                &color_to_paint(color),
                                tiny_skia::FillRule::Winding,
                                tiny_skia::Transform::identity(),
                                None,
                            );
                        }
                    }
                }
            }
        }
    }

    /// The §15.5 strided-pixel xxh3 digest — the frame-presented hash that
    /// powers visual-change detection without shipping pixels (§10).
    pub fn strided_hash(&self) -> u64 {
        let px = self.pixmap.data();
        let stride = 4 * 16; // every 16th pixel
        let mut hasher = Xxh3::default();
        let mut i = 0;
        while i < px.len() {
            hasher.write(&px[i..i + 4]);
            i += stride;
        }
        hasher.finish()
    }

    /// Raw RGBA8 (premultiplied per tiny-skia convention — the same
    /// bytes `FrameRef.pixels` carries, §6.1).
    pub fn rgba(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// Lossless PNG (§10 "Output: PNG (lossless)") via tiny-skia's encoder.
    pub fn png(&self) -> Result<Vec<u8>, PaintError> {
        self.pixmap
            .encode_png()
            .map_err(|e| PaintError::Png(e.to_string()))
    }
}

fn color_to_paint(color: Color) -> tiny_skia::Paint<'static> {
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(color.to_skia());
    paint.anti_alias = false;
    paint
}

/// A rounded-rect path (tiny-skia-path 0.12 has no `push_round_rect`).
fn round_rect_path(rect: tiny_skia::Rect, radius: f32) -> Option<tiny_skia::Path> {
    let rad = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let (left, top) = (rect.left(), rect.top());
    let (width, height) = (rect.width(), rect.height());
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(left + rad, top);
    pb.line_to(left + width - rad, top);
    pb.quad_to(left + width, top, left + width, top + rad);
    pb.line_to(left + width, top + height - rad);
    pb.quad_to(left + width, top + height, left + width - rad, top + height);
    pb.line_to(left + rad, top + height);
    pb.quad_to(left, top + height, left, top + height - rad);
    pb.line_to(left, top + rad);
    pb.quad_to(left, top, left + rad, top);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_solid_and_rounded_rects() {
        let mut state = PaintState::new(64, 48).expect("surface");
        state.paint(&[
            PaintOp::SolidRect {
                x: 8.0,
                y: 8.0,
                w: 32.0,
                h: 16.0,
                color: Color::rgba(255, 0, 0, 255),
            },
            PaintOp::RoundedRect {
                x: 8.0,
                y: 32.0,
                w: 32.0,
                h: 8.0,
                radius: 4.0,
                color: Color::rgba(0, 0, 255, 255),
            },
        ]);

        let px = |x: u32, y: u32| -> [u8; 4] {
            let i = ((y * 64 + x) * 4) as usize;
            state.rgba()[i..i + 4].try_into().expect("rgba")
        };
        // Premultiplied straight alpha (a=255 → rgb unchanged).
        assert_eq!(px(10, 10), [255, 0, 0, 255], "inside the solid rect");
        assert_eq!(px(0, 0), [0, 0, 0, 0], "outside any op is transparent");
        assert_eq!(px(12, 35), [0, 0, 255, 255], "inside the rounded rect");
        // A rounded corner (x=8+0.5, y=32+0.5) is NOT filled.
        assert_eq!(px(8, 32), [0, 0, 0, 0], "rounded corner cut");
    }

    #[test]
    fn strided_hash_is_content_sensitive_and_stable() {
        let mut a = PaintState::new(64, 64).expect("surface");
        a.paint(&[PaintOp::SolidRect {
            x: 0.0,
            y: 0.0,
            w: 64.0,
            h: 64.0,
            color: Color::rgba(255, 0, 0, 255),
        }]);
        let mut b = PaintState::new(64, 64).expect("surface");
        b.paint(&[PaintOp::SolidRect {
            x: 0.0,
            y: 0.0,
            w: 64.0,
            h: 64.0,
            color: Color::rgba(0, 0, 255, 255),
        }]);
        assert_ne!(a.strided_hash(), b.strided_hash(), "content changes hash");
        // Same ops → same hash (deterministic CPU raster, §10).
        let mut again = PaintState::new(64, 64).expect("surface");
        again.paint(&[PaintOp::SolidRect {
            x: 0.0,
            y: 0.0,
            w: 64.0,
            h: 64.0,
            color: Color::rgba(255, 0, 0, 255),
        }]);
        assert_eq!(a.strided_hash(), again.strided_hash());
    }

    #[test]
    fn png_export_is_lossless_and_decodable() {
        let mut state = PaintState::new(16, 16).expect("surface");
        state.paint(&[PaintOp::SolidRect {
            x: 0.0,
            y: 0.0,
            w: 16.0,
            h: 16.0,
            color: Color::rgba(10, 20, 30, 255),
        }]);
        let png = state.png().expect("png");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let decoded = tiny_skia::Pixmap::decode_png(&png).expect("decode");
        assert_eq!((decoded.width(), decoded.height()), (16, 16));
    }

    #[test]
    fn degenerate_surfaces_are_rejected() {
        assert!(matches!(
            PaintState::new(0, 10),
            Err(PaintError::TooLarge(0, 10))
        ));
        assert!(matches!(
            PaintState::new(1, 0),
            Err(PaintError::TooLarge(1, 0))
        ));
        assert!(matches!(
            PaintState::new(MAX_SURFACE + 1, 4),
            Err(PaintError::TooLarge(_, 4))
        ));
    }
}
