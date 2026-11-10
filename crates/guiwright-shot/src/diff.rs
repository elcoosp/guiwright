//! The per-pixel visual metric (§10): pixelwise **weighted squared YIQ
//! distance** (the `dify` semantics kittest adopted, §10 "adopt
//! `egui_kittest`/`dify` semantics) plus a `max_failed_pixels` budget.
//!
//! RGB → YIQ (NTSC), then the weighted squared channel distance
//! `0.5053·Δy² + 0.299·Δi² + 0.1957·Δq²`. A pixel fails when its distance
//! exceeds `threshold` (default [`DEFAULT_THRESHOLD`] = 0.6, kittest's
//! checked-in value). The comparison passes when the number of failed
//! pixels is ≤ `max_failed_pixels` (an absolute count — kittest README:
//! "an absolute pixel count, not a fraction of the image").

use crate::Image;

/// Default per-pixel tolerance (weighted squared YIQ distance) — kittest's
/// checked-in `[linux]/[macos]/[windows]` value (§10 inherits the shape).
pub const DEFAULT_THRESHOLD: f32 = 0.6;

/// YIQ channel weights for the squared distance (dify semantics).
const W_Y: f32 = 0.5053;
const W_I: f32 = 0.299;
const W_Q: f32 = 0.1957;

/// RGB (0-255) → YIQ components (scaled to 0-1 domain first).
fn rgb_to_yiq(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let (red, green, blue) = (
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
    );
    let y = 0.299 * red + 0.587 * green + 0.114 * blue;
    let i = 0.5959 * red - 0.2746 * green - 0.3213 * blue;
    let q = 0.2115 * red - 0.5227 * green + 0.3112 * blue;
    (y, i, q)
}

/// Weighted squared YIQ distance between two pixels, normalized to `[0, 1]`
/// (the dify form: `sqrt(0.5053·Δy² + 0.299·Δi² + 0.1957·Δq²)` on 0-1 YIQ —
/// the weights sum to 1, so 1.0 is the maximum perceptual distance and
/// kittest-style thresholds like 0.6 are fractions of that maximum).
pub fn pixel_distance(a: [u8; 4], b: [u8; 4]) -> f32 {
    let (y1, i1, q1) = rgb_to_yiq(a[0], a[1], a[2]);
    let (y2, i2, q2) = rgb_to_yiq(b[0], b[1], b[2]);
    let (dy, di, dq) = (y1 - y2, i1 - i2, q1 - q2);
    (W_Y * dy * dy + W_I * di * di + W_Q * dq * dq).sqrt()
}

/// Outcome of comparing actual pixels against a baseline (§10).
#[derive(Clone, Debug, PartialEq)]
pub struct DiffReport {
    /// Pixels whose distance exceeded the threshold.
    pub failed_pixels: usize,
    /// Largest pixel distance seen (for messages and gradations).
    pub max_distance: f32,
    /// Total compared pixels (width × height).
    pub total_pixels: usize,
    /// The distance at every pixel (row-major) — diff-image rendering and
    /// debugging; allocation-light enough for test-sized viewports.
    pub distances: Vec<f32>,
}

impl DiffReport {
    /// Budget check: §10 "`max_failed_pixels` budget".
    pub fn passes(&self, max_failed_pixels: usize) -> bool {
        self.failed_pixels <= max_failed_pixels
    }

    /// Human summary used in assertion errors.
    pub fn summarize(&self, threshold: f32, max_failed_pixels: usize) -> String {
        format!(
            "{} of {} pixels differ beyond threshold {threshold} (max distance {:.4}, budget {max_failed_pixels})",
            self.failed_pixels, self.total_pixels, self.max_distance
        )
    }
}

/// Compare two same-shape images pixelwise (§10).
///
/// Panics are avoided: a size mismatch is returned as `None` — the caller
/// (baseline flow) converts it into [`crate::ShotError::SizeMismatch`].
pub fn diff(baseline: &Image, actual: &Image, threshold: f32) -> Option<DiffReport> {
    if !baseline.same_shape(actual) {
        return None;
    }
    let total = (baseline.width * baseline.height) as usize;
    let mut distances = Vec::with_capacity(total);
    let mut failed = 0usize;
    let mut max = 0.0f32;
    for (a, b) in baseline
        .rgba
        .chunks_exact(4)
        .zip(actual.rgba.chunks_exact(4))
    {
        let pa = [a[0], a[1], a[2], a[3]];
        let pb = [b[0], b[1], b[2], b[3]];
        let d = pixel_distance(pa, pb);
        if d > threshold {
            failed += 1;
        }
        max = max.max(d);
        distances.push(d);
    }
    Some(DiffReport {
        failed_pixels: failed,
        max_distance: max,
        total_pixels: total,
        distances,
    })
}

/// Render a red-highlight diff image from a report (§10: failing runs emit
/// `<name>.diff.png`). White where pixels pass, scaled red where they fail.
pub fn diff_image(baseline: &Image, report: &DiffReport, threshold: f32) -> Image {
    let mut rgba = Vec::with_capacity(baseline.rgba.len());
    for d in &report.distances {
        if *d > threshold {
            let t = (d / threshold).min(4.0); // saturate past 4× the tolerance
            // Round + clamp in f32, convert through TryFrom — no truncating
            // or sign-losing `as` casts on the render path.
            let intensity = u8::try_from((128.0 * t).min(255.0).round() as i64).unwrap_or(u8::MAX);
            rgba.extend_from_slice(&[intensity, 0, 0, 255]);
        } else {
            rgba.extend_from_slice(&[255, 255, 255, 255]);
        }
    }
    Image {
        width: baseline.width,
        height: baseline.height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_pass() {
        let img = Image::solid(16, 16, [128, 64, 32, 255]);
        let report = diff(&img, &img, DEFAULT_THRESHOLD).expect("same shape");
        assert_eq!(report.failed_pixels, 0);
        assert_eq!(report.max_distance, 0.0);
        assert!(report.passes(0));
    }

    #[test]
    fn identical_luma_different_chroma_stays_under_threshold() {
        // Same luma (Y), different chroma — the YIQ metric exists to be
        // forgiving exactly here (anti-flakiness for AA fringes).
        let gray = Image::solid(8, 8, [128, 128, 128, 255]);
        let warm = Image::solid(8, 8, [138, 118, 128, 255]); // same Y approx
        let report = diff(&gray, &warm, DEFAULT_THRESHOLD).expect("same shape");
        assert!(
            report.passes(0),
            "chroma-only wiggle must pass: {}",
            report.summarize(DEFAULT_THRESHOLD, 0)
        );
    }

    #[test]
    fn black_vs_white_fails_hard() {
        let black = Image::solid(4, 4, [0, 0, 0, 255]);
        let white = Image::solid(4, 4, [255, 255, 255, 255]);
        let report = diff(&black, &white, DEFAULT_THRESHOLD).expect("same shape");
        assert_eq!(report.failed_pixels, 16);
        assert!(!report.passes(0));
        assert!(report.max_distance > 0.5);
    }

    #[test]
    fn budget_is_absolute_count() {
        let mut img = Image::solid(10, 10, [0, 0, 0, 255]);
        // paint one pixel white
        let i = (5 * 10 + 5) * 4;
        img.rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        let baseline = Image::solid(10, 10, [0, 0, 0, 255]);
        let report = diff(&baseline, &img, DEFAULT_THRESHOLD).expect("same shape");
        assert_eq!(report.failed_pixels, 1);
        assert!(report.passes(1), "1 failed pixel fits a budget of 1");
        assert!(!report.passes(0), "budget is absolute, not fractional");
    }

    #[test]
    fn size_mismatch_returns_none() {
        let a = Image::solid(4, 4, [0, 0, 0, 255]);
        let b = Image::solid(2, 8, [0, 0, 0, 255]);
        assert!(diff(&a, &b, DEFAULT_THRESHOLD).is_none());
    }

    #[test]
    fn diff_image_highlights_failures() {
        let baseline = Image::solid(2, 1, [0, 0, 0, 255]);
        let mut actual = baseline.clone();
        actual.rgba[0..4].copy_from_slice(&[255, 255, 255, 255]);
        let report = diff(&baseline, &actual, DEFAULT_THRESHOLD).expect("same shape");
        let rendered = diff_image(&baseline, &report, DEFAULT_THRESHOLD);
        assert!(rendered.pixel(0, 0)[0] > 0, "failed pixel tinted red");
        assert_eq!(
            rendered.pixel(1, 0),
            [255, 255, 255, 255],
            "pass stays white"
        );
    }
}
