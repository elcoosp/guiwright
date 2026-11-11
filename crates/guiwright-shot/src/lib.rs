//! Screenshot capture codec, YIQ diff and the baseline/bless flow
//! (architecture §10 "Frames, screenshots, video, visual regression").
//!
//! This crate is framework-free: adapters hand it RGBA8 buffers, the kit
//! calls it from its screenshot surfaces (`App::screenshot` for the
//! window, `Locator::screenshot` for one element). Three pieces:
//!
//! * [`codec`] — PNG encode/decode (lossless, §10 "Output: PNG (lossless)").
//! * [`diff`] — the per-pixel **weighted squared YIQ distance** with a
//!   `max_failed_pixels` budget (the kittest/dify semantics §10 adopts).
//! * [`baseline`] — baselines stored per platform+scale+renderer at
//!   `tests/screenshots/<name>/<os>-<arch>-<renderer>.png` (§10; iced's
//!   renderer-suffix lesson), the `GUIWRIGHT_BLESS` flow (`1` updates
//!   failing, `force` updates all) and the failing-run artifacts
//!   (`<name>.new.png` + `<name>.diff.png`).
//!
//! [`config`] parses `Guiwright.toml` (§10: "config in `Guiwright.toml`
//! with per-OS overrides — we inherit kittest.toml's shape").

#![forbid(unsafe_code)]

pub mod baseline;
pub mod codec;
pub mod config;
pub mod diff;

/// An RGBA8 image buffer (row-major, 4 bytes per pixel, straight alpha).
///
/// Diffing compares the RGB channels (§10 metric: weighted squared YIQ
/// distance); straight-alpha compositing onto an opaque background is the
/// adapter's job before handing pixels over — premultiplied RGB already
/// encodes alpha, so the metric ignores the A channel by design.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Image {
    /// Solid-color image (tests + deterministic fixtures).
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        Self {
            width,
            height,
            rgba: rgba.repeat((width * height) as usize),
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Dimensions must match for diffing (§10: same snapshot name implies
    /// same viewport; a size change is always a mismatch).
    pub fn same_shape(&self, other: &Image) -> bool {
        self.width == other.width && self.height == other.height
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ShotError {
    #[error(
        "screenshot size mismatch: baseline is {baseline_w}x{baseline_h}, actual {actual_w}x{actual_h}"
    )]
    SizeMismatch {
        baseline_w: u32,
        baseline_h: u32,
        actual_w: u32,
        actual_h: u32,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("png codec error: {0}")]
    Png(String),
    #[error("config error: {0}")]
    Config(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_image_has_exact_bytes() {
        let img = Image::solid(2, 2, [10, 20, 30, 255]);
        assert_eq!(img.rgba.len(), 16);
        assert_eq!(img.pixel(1, 0), [10, 20, 30, 255]);
        assert_eq!(img.pixel(0, 1), [10, 20, 30, 255]);
    }

    #[test]
    fn shape_check() {
        let a = Image::solid(4, 4, [0, 0, 0, 255]);
        let b = Image::solid(4, 4, [255, 255, 255, 255]);
        let c = Image::solid(2, 8, [0, 0, 0, 255]);
        assert!(a.same_shape(&b));
        assert!(!a.same_shape(&c));
    }
}

/// Crop a sub-rect (clip-aware element screenshots, §10 "crop from the
/// window frame by the node's clip-aware rect — never re-render").
/// Coordinates are clamped to the image; an empty rect is an error.
pub fn crop(image: &Image, x0: u32, y0: u32, x1: u32, y1: u32) -> Result<Image, ShotError> {
    let x0 = x0.min(image.width);
    let y0 = y0.min(image.height);
    let x1 = x1.clamp(x0, image.width);
    let y1 = y1.clamp(y0, image.height);
    let w = x1 - x0;
    let h = y1 - y0;
    if w == 0 || h == 0 {
        return Err(ShotError::Config(format!(
            "empty crop rect ({x0},{y0})-({x1},{y1})"
        )));
    }
    let mut rgba = Vec::with_capacity((w * h) as usize * 4);
    for row in y0..y1 {
        let start = ((row * image.width + x0) * 4) as usize;
        let end = start + (w * 4) as usize;
        rgba.extend_from_slice(&image.rgba[start..end]);
    }
    Ok(Image {
        width: w,
        height: h,
        rgba,
    })
}

#[cfg(test)]
mod crop_tests {
    use super::*;

    #[test]
    fn crop_takes_the_requested_region() {
        // 3x2 image with distinct pixels: (x, y) → [x*10+y, 0, 0, 255]
        let mut img = Image {
            width: 3,
            height: 2,
            rgba: Vec::new(),
        };
        for y in 0..2u32 {
            for x in 0..3u32 {
                img.rgba.extend_from_slice(&[(x * 10 + y) as u8, 0, 0, 255]);
            }
        }
        let region = crop(&img, 1, 0, 3, 2).expect("crop");
        assert_eq!(region.width, 2);
        assert_eq!(region.height, 2);
        assert_eq!(region.pixel(0, 0), [10, 0, 0, 255]);
        assert_eq!(region.pixel(1, 1), [21, 0, 0, 255]);
    }

    #[test]
    fn crop_clamps_and_rejects_empty() {
        let img = Image::solid(4, 4, [1, 2, 3, 255]);
        assert!(crop(&img, 2, 2, 2, 4).is_err(), "zero width");
        assert!(crop(&img, 0, 0, 0, 0).is_err(), "zero height");
        let clamped = crop(&img, 2, 2, 99, 99).expect("clamped to image");
        assert_eq!((clamped.width, clamped.height), (2, 2));
    }
}
