//! The §15.5 strided-pixel frame digest (§4.5 "GPUI has no scene digest —
//! use the §15.5 strided-pixel xxh3 via `test-support`'s
//! `rendered_frame`").
//!
//! Identical policy to the egui adapter's pixel digest: hash a strided
//! sample of the RGBA buffer (every 16th pixel), not the whole frame —
//! "wait for visual change" without shipping pixels (§10), and the dedup
//! signal the video recorder consumes (§10 M3).

use std::hash::Hasher;

use twox_hash::xxhash3_64::Hasher as Xxh3;

/// Stride: every 16th pixel (4 bytes each) — the §15.5 sample.
const STRIDE_PIXELS: usize = 16;

/// xxh3 over a strided sample of a premultiplied RGBA8 buffer.
pub fn strided_xxh3(rgba: &[u8]) -> u64 {
    let stride = 4 * STRIDE_PIXELS;
    let mut hasher = Xxh3::default();
    let mut i = 0;
    while i < rgba.len() {
        hasher.write(&rgba[i..(i + 4).min(rgba.len())]);
        i += stride;
    }
    hasher.finish()
}

/// The frame digest surface the glue calls with `test-support`'s
/// `rendered_frame` bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameDigest(pub u64);

impl FrameDigest {
    pub fn of(rgba: &[u8]) -> Self {
        Self(strided_xxh3(rgba))
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(w: usize, h: usize, rgba: [u8; 4]) -> Vec<u8> {
        rgba.repeat(w * h)
    }

    #[test]
    fn content_sensitive_and_stable() {
        let red = buffer(64, 64, [255, 0, 0, 255]);
        let blue = buffer(64, 64, [0, 0, 255, 255]);
        assert_ne!(strided_xxh3(&red), strided_xxh3(&blue));
        assert_eq!(strided_xxh3(&red), strided_xxh3(&red), "deterministic");
    }

    #[test]
    fn strided_sample_skips_most_pixels_but_still_sees_change() {
        // Change deep in the frame (not at offset 0): the stride must
        // catch it.
        let mut base = buffer(128, 128, [10, 20, 30, 255]);
        let mut touched = base.clone();
        let idx = (16 * 4) + 1; // the 16th pixel, green channel
        touched[idx] = 200;
        assert_ne!(strided_xxh3(&base), strided_xxh3(&touched));
        // A change one pixel off-stride is invisible by design (documented
        // tolerance of the sample, §15.5).
        let mut off_stride = base.clone();
        off_stride[4] = 200; // pixel 1 — never sampled
        assert_eq!(strided_xxh3(&base), strided_xxh3(&off_stride));
        let _ = &mut base;
    }

    #[test]
    fn digest_wraps_the_hash() {
        let frame = buffer(8, 8, [1, 2, 3, 255]);
        let d = FrameDigest::of(&frame);
        assert_eq!(d.value(), strided_xxh3(&frame));
    }
}
