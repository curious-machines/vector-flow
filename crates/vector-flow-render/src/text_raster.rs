use std::sync::Arc;

use ab_glyph::{Font, FontArc};

use vector_flow_core::types::{ImageData, TextInstance};

use crate::batch::{CollectedText, ImageDrawBatch};

fn hash_text(text: &TextInstance, size_bucket: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.text.hash(&mut hasher);
    text.style.font_family.hash(&mut hasher);
    text.style.font_path.hash(&mut hasher);
    text.style.font_weight.hash(&mut hasher);
    (text.style.font_style as u8).hash(&mut hasher);
    (text.style.alignment as u8).hash(&mut hasher);
    size_bucket.hash(&mut hasher);
    // Hash color
    (text.color.r.to_bits()).hash(&mut hasher);
    (text.color.g.to_bits()).hash(&mut hasher);
    (text.color.b.to_bits()).hash(&mut hasher);
    (text.color.a.to_bits()).hash(&mut hasher);
    hasher.finish()
}

/// Quantize a pixel size into discrete buckets to avoid re-rasterizing on every
/// tiny zoom change. Returns a bucket ID.
fn size_bucket(pixel_size: f32) -> u32 {
    // Bucket at powers of 1.25: each bucket is ~25% larger than the previous.
    // This gives us re-rasterization roughly every 25% zoom change.
    if pixel_size <= 1.0 {
        return 0;
    }
    (pixel_size.ln() / 1.25_f32.ln()).round() as u32
}

/// Rasterize text into an RGBA pixel buffer.
/// Returns (width, height, pixels) or None if text is empty.
pub fn rasterize_text(text: &TextInstance, scale_factor: f32) -> Option<(u32, u32, Vec<u8>)> {
    let layout = &text.layout;
    if layout.glyphs.is_empty() {
        return None;
    }

    let font = FontArc::try_from_vec(layout.font_data.as_ref().clone()).ok()?;

    let (layout_w, layout_h) = layout.bounds;
    let pixel_w = ((layout_w * scale_factor).ceil() as u32).max(1);
    let pixel_h = ((layout_h * scale_factor).ceil() as u32).max(1);

    // Clamp to reasonable size to avoid OOM
    let max_dim = 4096;
    let pixel_w = pixel_w.min(max_dim);
    let pixel_h = pixel_h.min(max_dim);

    let mut pixels = vec![0u8; (pixel_w * pixel_h * 4) as usize];

    let r = (text.color.r * 255.0).round() as u8;
    let g = (text.color.g * 255.0).round() as u8;
    let b = (text.color.b * 255.0).round() as u8;

    for pg in &layout.glyphs {
        let scale = ab_glyph::PxScale::from(pg.size * scale_factor);
        let glyph_id = ab_glyph::GlyphId(pg.glyph_id);
        let glyph = glyph_id.with_scale_and_position(
            scale,
            ab_glyph::point(pg.x * scale_factor, pg.y * scale_factor),
        );

        if let Some(outlined) = Font::outline_glyph(&font, glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|px, py, coverage: f32| {
                let x = px as i32 + bounds.min.x as i32;
                let y = py as i32 + bounds.min.y as i32;
                if x >= 0 && x < pixel_w as i32 && y >= 0 && y < pixel_h as i32 {
                    let idx = ((y as u32 * pixel_w + x as u32) * 4) as usize;
                    let alpha = (coverage * text.color.a * 255.0).round() as u8;
                    // Simple alpha blending (source over)
                    let dst_a = pixels[idx + 3];
                    if dst_a == 0 {
                        pixels[idx] = r;
                        pixels[idx + 1] = g;
                        pixels[idx + 2] = b;
                        pixels[idx + 3] = alpha;
                    } else {
                        // Composite
                        let sa = alpha as f32 / 255.0;
                        let da = dst_a as f32 / 255.0;
                        let out_a = sa + da * (1.0 - sa);
                        if out_a > 0.0 {
                            let blend = |s: u8, d: u8| -> u8 {
                                ((s as f32 * sa + d as f32 * da * (1.0 - sa)) / out_a)
                                    .round() as u8
                            };
                            pixels[idx] = blend(r, pixels[idx]);
                            pixels[idx + 1] = blend(g, pixels[idx + 1]);
                            pixels[idx + 2] = blend(b, pixels[idx + 2]);
                            pixels[idx + 3] = (out_a * 255.0).round() as u8;
                        }
                    }
                }
            });
        }
    }

    Some((pixel_w, pixel_h, pixels))
}

/// Convert collected text instances into ImageDrawBatches for rendering via the image pipeline.
/// `zoom` is the current camera zoom level, `pixels_per_point` is the display scale factor.
pub fn prepare_text_batches(
    texts: &[CollectedText],
    zoom: f32,
    pixels_per_point: f32,
) -> Vec<ImageDrawBatch> {
    use crate::batch::affine2_to_mat4;
    use crate::vertex::ImageVertex;

    let scale_factor = (zoom * pixels_per_point).max(0.25);

    texts
        .iter()
        .filter_map(|ct| {
            let text = &ct.text;
            let (pixel_w, pixel_h, pixels) = rasterize_text(text, scale_factor)?;

            let (layout_w, layout_h) = text.layout.bounds;

            // The image represents the text block at its layout size.
            // Scale the quad to match the layout dimensions in world space.
            let hw = layout_w / 2.0;
            let hh = layout_h / 2.0;

            // UV y is NOT flipped for text — rasterized top-to-bottom, rendered top-to-bottom.
            // But canvas Y is up, so we need to handle the coordinate system:
            // The text is rasterized with Y-down. We render the quad with the image pipeline
            // which expects Y-flipped UVs for images. For text we want top of texture = top of text
            // visually, which in our Y-up canvas means the top of the quad is at +hh.
            // Image pipeline already flips UV Y (uv: 0,1 at bottom, 0,0 at top).
            // So we use the same UV convention as images.
            let vertices = [
                ImageVertex { position: [-hw, -hh], uv: [0.0, 1.0] },
                ImageVertex { position: [ hw, -hh], uv: [1.0, 1.0] },
                ImageVertex { position: [ hw,  hh], uv: [1.0, 0.0] },
                ImageVertex { position: [-hw,  hh], uv: [0.0, 0.0] },
            ];
            let indices = [0u32, 1, 2, 0, 2, 3];

            // The text transform positions the text in world space.
            // We need to offset by half the layout size since the quad is centered at origin
            // but the text layout starts at (0, 0).
            let center_offset = glam::Affine2::from_translation(glam::Vec2::new(hw, -hh));
            let transform = affine2_to_mat4(&(text.transform * center_offset));

            let tint = if ct.dimmed {
                [0.3, 0.3, 0.3, 0.5]
            } else {
                [1.0, 1.0, 1.0, text.opacity]
            };

            // Create a unique source_path for the texture cache
            let source_path = format!(
                "__text_{}_{}_{}",
                hash_text(text, size_bucket(text.style.font_size as f32 * scale_factor)),
                pixel_w,
                pixel_h,
            );

            Some(ImageDrawBatch {
                image: Arc::new(ImageData {
                    width: pixel_w,
                    height: pixel_h,
                    pixels,
                    source_path,
                }),
                vertices,
                indices,
                transform,
                color: tint,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::ScaleFont;
    use glam::Affine2;
    use vector_flow_core::types::{Color, TextLayout, TextStyle, PositionedGlyph};

    fn make_test_text() -> Arc<TextInstance> {
        // Load the bundled font for testing
        let font_data: &[u8] = include_bytes!(
            "../../vector-flow-compute/src/fonts/NotoSans-Regular.ttf"
        );
        let font = FontArc::try_from_slice(font_data).unwrap();
        let scale = ab_glyph::PxScale::from(24.0);
        let scaled = font.as_scaled(scale);

        let mut glyphs = Vec::new();
        let mut x = 0.0f32;
        for ch in "Hi".chars() {
            let glyph_id = font.glyph_id(ch);
            glyphs.push(PositionedGlyph {
                glyph_id: glyph_id.0,
                x,
                y: scaled.ascent(),
                size: 24.0,
            });
            x += scaled.h_advance(glyph_id);
        }

        Arc::new(TextInstance {
            text: "Hi".into(),
            style: TextStyle::default(),
            color: Color::WHITE,
            transform: Affine2::IDENTITY,
            opacity: 1.0,
            layout: Arc::new(TextLayout {
                bounds: (x, scaled.ascent() - scaled.descent()),
                glyphs,
                font_data: Arc::new(font_data.to_vec()),
                font_index: 0,
            }),
        })
    }

    #[test]
    fn rasterize_simple_text() {
        let text = make_test_text();
        let result = rasterize_text(&text, 1.0);
        assert!(result.is_some());
        let (w, h, pixels) = result.unwrap();
        assert!(w > 0);
        assert!(h > 0);
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        // Should have some non-zero pixels
        assert!(pixels.iter().any(|&p| p > 0));
    }

    #[test]
    fn size_bucket_quantization() {
        let b1 = size_bucket(24.0);
        let b2 = size_bucket(25.0);
        // Close sizes should be in the same bucket
        assert_eq!(b1, b2);
        // Very different sizes should differ
        let b3 = size_bucket(100.0);
        assert_ne!(b1, b3);
    }
}
