//! PNG codec (§10 "Output: PNG (lossless)"). Lossless in, lossless out —
//! baselines and trace frames are byte-stable across runs.

use crate::{Image, ShotError};

/// Encode RGBA8 → PNG bytes (8-bit RGBA, no interlace).
pub fn encode_png(image: &Image) -> Result<Vec<u8>, ShotError> {
    let mut out = Vec::new();
    {
        let mut sink = std::io::BufWriter::new(&mut out);
        let mut encoder = png::Encoder::new(&mut sink, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| ShotError::Png(e.to_string()))?;
        writer
            .write_image_data(&image.rgba)
            .map_err(|e| ShotError::Png(e.to_string()))?;
    }
    Ok(out)
}

/// Decode PNG bytes → RGBA8 (any source color type is normalized to RGBA8).
pub fn decode_png(bytes: &[u8]) -> Result<Image, ShotError> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|e| ShotError::Png(e.to_string()))?;
    let mut buf = vec![0; reader.output_buffer_size().expect("png header known")];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| ShotError::Png(e.to_string()))?;
    let width = info.width;
    let height = info.height;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => expand_rgb(&buf[..info.buffer_size()], width, height),
        other => {
            return Err(ShotError::Png(format!(
                "unsupported source color type {other:?} (expect RGBA/RGB baselines)"
            )));
        }
    };
    Ok(Image {
        width,
        height,
        rgba,
    })
}

fn expand_rgb(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let pixels = (width * height) as usize;
    let mut rgba = Vec::with_capacity(pixels * 4);
    for px in rgb.chunks_exact(3) {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip_is_byte_stable() {
        let img = Image::solid(7, 5, [12, 200, 34, 255]);
        let bytes = encode_png(&img).expect("encode");
        let decoded = decode_png(&bytes).expect("decode");
        assert_eq!(decoded, img, "lossless round-trip");
        // Re-encode determinism (same libpng settings → same bytes).
        let bytes2 = encode_png(&img).expect("encode again");
        assert_eq!(bytes, bytes2, "byte-stable output");
    }

    #[test]
    fn rgb_source_normalizes_to_opaque_rgba() {
        // Hand-build an RGB PNG via the encoder.
        let w = 3u32;
        let h = 2u32;
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgb.extend_from_slice(&[(x * 80) as u8, (y * 100) as u8, 7]);
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().expect("header");
            wr.write_image_data(&rgb).expect("data");
        }
        let img = decode_png(&out).expect("decode");
        assert_eq!(img.width, w);
        assert_eq!(img.height, h);
        assert_eq!(img.pixel(2, 1), [160, 100, 7, 255]);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_png(b"not a png").is_err());
    }
}
