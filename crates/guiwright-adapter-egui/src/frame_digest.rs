//! Frame digest: xxh3 over the tessellated scene (architecture §10
//! "scene-digest where available" + §15.5 hash-dedup) — the cheap
//! visual-change signal that powers `FramePresented` without pixel
//! readback at M0.

use egui::epaint::Primitive;
use std::hash::Hasher;
use twox_hash::xxhash3_64::Hasher as Xxh3;

/// Digest the tessellated primitives of one pass. Geometry-only on purpose:
/// vertex positions/uvs/colors fully determine the painted output once the
/// font atlas is warm (textures change once, at startup).
pub fn digest(primitives: &[egui::ClippedPrimitive]) -> u64 {
    let mut hasher = Xxh3::default();
    for prim in primitives {
        hasher.write_u32(prim.clip_rect.min.x.to_bits());
        hasher.write_u32(prim.clip_rect.min.y.to_bits());
        hasher.write_u32(prim.clip_rect.max.x.to_bits());
        hasher.write_u32(prim.clip_rect.max.y.to_bits());
        match &prim.primitive {
            Primitive::Mesh(mesh) => {
                hasher.write_u64(mesh.vertices.len() as u64);
                for v in &mesh.vertices {
                    hasher.write_u32(v.pos.x.to_bits());
                    hasher.write_u32(v.pos.y.to_bits());
                    hasher.write_u32(v.uv.x.to_bits());
                    hasher.write_u32(v.uv.y.to_bits());
                    let [r, g, b, a] = v.color.to_array();
                    hasher.write_u8(r);
                    hasher.write_u8(g);
                    hasher.write_u8(b);
                    hasher.write_u8(a);
                }
                hasher.write_u64(mesh.indices.len() as u64);
            }
            // Paint callbacks are opaque; their presence still changes the
            // scene (counted once) — pixel capture lands in M1.
            Primitive::Callback(_) => {
                hasher.write_u8(0xFF);
            }
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Rect, Vec2};

    /// Render a tiny UI and digest it.
    fn digest_of(app: impl FnOnce(&mut egui::Ui)) -> u64 {
        let ctx = Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(400.0))),
            time: Some(1.0),
            ..Default::default()
        };
        let mut opt_app = Some(app);
        let output = ctx.run_ui(input, |ui| {
            if let Some(app) = opt_app.take() {
                app(ui);
            }
        });
        let mut output = output;
        let shapes = std::mem::take(&mut output.shapes);
        let prims = ctx.tessellate(shapes, output.pixels_per_point);
        // Hash-only pipeline: explicitly drop unapplied texture deltas
        // (a real renderer would apply them to its atlas).
        output.drop_without_applying_deltas();
        digest(&prims)
    }

    #[test]
    fn static_ui_digests_equal_across_passes() {
        let a = digest_of(|ui| {
            ui.label("Hello");
            let _ = ui.button("Save");
        });
        let b = digest_of(|ui| {
            ui.label("Hello");
            let _ = ui.button("Save");
        });
        assert_eq!(a, b, "identical UI must hash identically");
    }

    #[test]
    fn changed_ui_changes_digest() {
        let a = digest_of(|ui| {
            ui.label("Hello");
        });
        let b = digest_of(|ui| {
            ui.label("Hello, Ada");
        });
        assert_ne!(a, b, "different content must hash differently");
    }

    #[test]
    fn empty_ui_digest_is_stable() {
        let a = digest_of(|_ui| {});
        let b = digest_of(|_ui| {});
        assert_eq!(a, b);
    }
}
