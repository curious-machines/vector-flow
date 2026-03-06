use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::{Font, FontArc, ScaleFont};
use glam::{Affine2, Vec2};

use vector_flow_core::error::ComputeError;
use vector_flow_core::types::{
    FontStyle, PathData, PathVerb, Point, PositionedGlyph, TextAlignment, TextInstance,
    TextLayout, TextStyle,
};

// ---------------------------------------------------------------------------
// Bundled fonts
// ---------------------------------------------------------------------------

const NOTO_SANS_REGULAR: &[u8] = include_bytes!("../fonts/NotoSans-Regular.ttf");
const NOTO_SANS_BOLD: &[u8] = include_bytes!("../fonts/NotoSans-Bold.ttf");
const NOTO_SANS_ITALIC: &[u8] = include_bytes!("../fonts/NotoSans-Italic.ttf");
const NOTO_SANS_BOLD_ITALIC: &[u8] = include_bytes!("../fonts/NotoSans-BoldItalic.ttf");

/// Select bundled font bytes based on weight and style.
fn bundled_font(weight: u16, style: FontStyle) -> &'static [u8] {
    let bold = weight >= 600;
    let italic = matches!(style, FontStyle::Italic | FontStyle::Oblique);
    match (bold, italic) {
        (false, false) => NOTO_SANS_REGULAR,
        (true, false) => NOTO_SANS_BOLD,
        (false, true) => NOTO_SANS_ITALIC,
        (true, true) => NOTO_SANS_BOLD_ITALIC,
    }
}

// ---------------------------------------------------------------------------
// Font cache
// ---------------------------------------------------------------------------

pub struct FontCache {
    db: fontdb::Database,
    /// Cached font bytes keyed by resolved path or family+weight+style key.
    loaded: HashMap<String, Arc<Vec<u8>>>,
}

impl FontCache {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self {
            db,
            loaded: HashMap::new(),
        }
    }

    /// Resolve font bytes for the given style.
    /// Priority: font_path > font_family (system lookup) > bundled default.
    /// Returns (font_bytes, font_index).
    pub fn resolve_font(
        &mut self,
        style: &TextStyle,
        project_dir: &str,
    ) -> (Arc<Vec<u8>>, u32) {
        // 1. Explicit font path
        if !style.font_path.is_empty() {
            let resolved = super::resolve_path(&style.font_path, project_dir);
            if let Some(cached) = self.loaded.get(&resolved) {
                return (Arc::clone(cached), 0);
            }
            if let Ok(data) = std::fs::read(&resolved) {
                let arc = Arc::new(data);
                self.loaded.insert(resolved, Arc::clone(&arc));
                return (arc, 0);
            }
            // Fall through to family lookup if file not found
        }

        // 2. System font family lookup via fontdb
        if !style.font_family.is_empty() {
            let db_weight = fontdb::Weight(style.font_weight);
            let db_style = match style.font_style {
                FontStyle::Normal => fontdb::Style::Normal,
                FontStyle::Italic => fontdb::Style::Italic,
                FontStyle::Oblique => fontdb::Style::Oblique,
            };
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(&style.font_family)],
                weight: db_weight,
                style: db_style,
                stretch: fontdb::Stretch::Normal,
            };
            if let Some(face_id) = self.db.query(&query) {
                let cache_key = format!(
                    "fontdb:{}:{}:{:?}",
                    style.font_family, style.font_weight, style.font_style
                );
                if let Some(cached) = self.loaded.get(&cache_key) {
                    let face_info = self.db.face(face_id).unwrap();
                    return (Arc::clone(cached), face_info.index);
                }
                // Extract font data from fontdb
                let mut result_data: Option<(Arc<Vec<u8>>, u32)> = None;
                self.db.with_face_data(face_id, |data, index| {
                    let arc = Arc::new(data.to_vec());
                    result_data = Some((arc, index));
                });
                if let Some((data, index)) = result_data {
                    self.loaded.insert(cache_key, Arc::clone(&data));
                    return (data, index);
                }
            }
        }

        // 3. Bundled default
        let bytes = bundled_font(style.font_weight, style.font_style);
        let cache_key = format!("bundled:{}:{:?}", style.font_weight, style.font_style);
        if let Some(cached) = self.loaded.get(&cache_key) {
            return (Arc::clone(cached), 0);
        }
        let arc = Arc::new(bytes.to_vec());
        self.loaded.insert(cache_key, Arc::clone(&arc));
        (arc, 0)
    }
}

// ---------------------------------------------------------------------------
// Text layout
// ---------------------------------------------------------------------------

/// Lay out text into positioned glyphs.
#[allow(unused_assignments)] // prev_glyph is set in wrap branches, read in next iteration
pub fn layout_text(
    text: &str,
    style: &TextStyle,
    font_data: &[u8],
    font_index: u32,
) -> Result<TextLayout, ComputeError> {
    let font = FontArc::try_from_vec(font_data.to_vec())
        .map_err(|e| ComputeError::BackendError(format!("Font load error: {e}")))?;

    let scale = ab_glyph::PxScale::from(style.font_size as f32);
    let scaled = font.as_scaled(scale);

    let ascent = scaled.ascent();
    let descent = scaled.descent();
    let line_gap = scaled.line_gap();
    let line_step = (ascent - descent + line_gap) * style.line_height as f32;

    let box_w = if style.box_width > 0.0 {
        Some(style.box_width as f32)
    } else {
        None
    };
    let box_h = if style.box_height > 0.0 {
        Some(style.box_height as f32)
    } else {
        None
    };

    // Break text into lines, then lay out glyphs per line.
    let mut lines: Vec<Vec<PositionedGlyph>> = Vec::new();
    let mut line_widths: Vec<f32> = Vec::new();
    let mut current_line: Vec<PositionedGlyph> = Vec::new();
    let mut cursor_x: f32 = 0.0;
    let mut cursor_y: f32 = ascent; // baseline of first line
    let mut prev_glyph: Option<ab_glyph::GlyphId> = None;
    let mut max_width: f32 = 0.0;

    let letter_spacing = style.letter_spacing as f32;

    for ch in text.chars() {
        if ch == '\n' {
            line_widths.push(cursor_x);
            if cursor_x > max_width {
                max_width = cursor_x;
            }
            lines.push(std::mem::take(&mut current_line));
            cursor_x = 0.0;
            cursor_y += line_step;
            prev_glyph = None;

            if let Some(bh) = box_h {
                if cursor_y - ascent + line_step > bh {
                    break;
                }
            }
            continue;
        }

        let glyph_id = font.glyph_id(ch);

        // Kerning
        if let Some(prev) = prev_glyph {
            cursor_x += scaled.kern(prev, glyph_id);
        }

        let advance = scaled.h_advance(glyph_id);

        // Word wrap: if we have a box width and this glyph would exceed it
        if style.wrap {
            if let Some(bw) = box_w {
                if cursor_x + advance > bw && !current_line.is_empty() {
                    // Find the last space to break at
                    let break_at = find_word_break(&current_line, text);
                    if break_at > 0 && break_at < current_line.len() {
                        let remainder: Vec<PositionedGlyph> =
                            current_line.drain(break_at..).collect();
                        let w = current_line
                            .last()
                            .map(|g| g.x + scaled.h_advance(ab_glyph::GlyphId(g.glyph_id)))
                            .unwrap_or(0.0);
                        line_widths.push(w);
                        if w > max_width {
                            max_width = w;
                        }
                        lines.push(std::mem::take(&mut current_line));
                        cursor_y += line_step;

                        // Re-position remainder glyphs on new line
                        let offset_x = if let Some(first) = remainder.first() {
                            first.x
                        } else {
                            0.0
                        };
                        for g in &remainder {
                            current_line.push(PositionedGlyph {
                                glyph_id: g.glyph_id,
                                x: g.x - offset_x,
                                y: cursor_y,
                                size: g.size,
                            });
                        }
                        cursor_x = current_line
                            .last()
                            .map(|g| {
                                g.x + scaled.h_advance(ab_glyph::GlyphId(g.glyph_id))
                                    + letter_spacing
                            })
                            .unwrap_or(0.0);
                        prev_glyph = current_line
                            .last()
                            .map(|g| ab_glyph::GlyphId(g.glyph_id));
                    } else {
                        // No good break point; break at current position
                        line_widths.push(cursor_x);
                        if cursor_x > max_width {
                            max_width = cursor_x;
                        }
                        lines.push(std::mem::take(&mut current_line));
                        cursor_x = 0.0;
                        cursor_y += line_step;
                        prev_glyph = None;
                    }

                    if let Some(bh) = box_h {
                        if cursor_y - ascent + line_step > bh {
                            break;
                        }
                    }
                }
            }
        }

        // Skip space glyphs at start of wrapped line (trim leading spaces)
        if ch == ' ' && current_line.is_empty() && !lines.is_empty() {
            cursor_x = 0.0;
            prev_glyph = Some(glyph_id);
            continue;
        }

        current_line.push(PositionedGlyph {
            glyph_id: glyph_id.0,
            x: cursor_x,
            y: cursor_y,
            size: style.font_size as f32,
        });

        cursor_x += advance + letter_spacing;
        prev_glyph = Some(glyph_id);
    }

    // Final line
    if !current_line.is_empty() {
        line_widths.push(cursor_x);
        if cursor_x > max_width {
            max_width = cursor_x;
        }
        lines.push(current_line);
    }

    // Apply alignment
    let effective_width = box_w.unwrap_or(max_width);
    let mut all_glyphs: Vec<PositionedGlyph> = Vec::new();
    for (line_idx, line) in lines.into_iter().enumerate() {
        let lw = line_widths.get(line_idx).copied().unwrap_or(0.0);
        let offset_x = match style.alignment {
            TextAlignment::Left => 0.0,
            TextAlignment::Center => (effective_width - lw) / 2.0,
            TextAlignment::Right => effective_width - lw,
        };
        for mut g in line {
            g.x += offset_x;
            all_glyphs.push(g);
        }
    }

    let total_height = if line_widths.is_empty() {
        0.0
    } else {
        (line_widths.len() as f32 - 1.0) * line_step + (ascent - descent)
    };

    Ok(TextLayout {
        glyphs: all_glyphs,
        bounds: (effective_width, total_height),
        font_data: Arc::new(font_data.to_vec()),
        font_index,
    })
}

/// Find a word break point (index of first glyph after the last space).
fn find_word_break(glyphs: &[PositionedGlyph], _text: &str) -> usize {
    // Walk backwards looking for a space-like gap.
    // Since we don't store the character, we use glyph_id heuristic:
    // space glyph typically has glyph_id = font.glyph_id(' ').
    // For simplicity, just look for the last glyph that could be a break point.
    // We'll break after the last "space" glyph.
    // A simpler approach: check gaps between glyphs.
    // For now, break at the last glyph that starts a word (rough heuristic).
    for i in (1..glyphs.len()).rev() {
        // If there's a gap bigger than expected, that was a space
        let prev_end = glyphs[i - 1].x + glyphs[i - 1].size * 0.5; // rough
        if glyphs[i].x - prev_end > glyphs[i].size * 0.1 {
            return i;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Text to Path conversion
// ---------------------------------------------------------------------------

/// Extract glyph outlines from a TextInstance as PathData.
///
/// Builds the path in **unscaled font units** (large coordinate space, typically
/// 0–2048) so that downstream tessellation with a fixed tolerance (e.g. 0.5)
/// produces smooth curves. A uniform scale transform is then baked into the
/// path coordinates to bring it back to the layout's world-space size.
pub fn text_to_path(text_inst: &TextInstance) -> Result<(PathData, Affine2), ComputeError> {
    let layout = &text_inst.layout;

    // Empty font data means no text was laid out (e.g. default/unconnected input)
    if layout.font_data.is_empty() || layout.glyphs.is_empty() {
        return Ok((PathData::new(), Affine2::IDENTITY));
    }

    let font = FontArc::try_from_vec(layout.font_data.as_ref().clone())
        .map_err(|e| ComputeError::BackendError(format!("Font load error: {e}")))?;

    let height_unscaled = font.height_unscaled();
    let mut verbs: Vec<PathVerb> = Vec::new();

    // Build path in unscaled font units for tessellation quality.
    // Glyph layout positions (pg.x, pg.y) are in scaled (pixel) coords,
    // so we convert them back to font units for positioning.
    for pg in &layout.glyphs {
        let glyph_id = ab_glyph::GlyphId(pg.glyph_id);
        let scale_factor = pg.size / height_unscaled;
        // Convert layout position back to font-unit space
        let glyph_x = pg.x / scale_factor;
        let glyph_y = pg.y / scale_factor;

        if let Some(outline) = font.outline(glyph_id) {
            // Build contours from outline curves.
            // ab_glyph OutlineCurve: Line(from, to), Quad(from, ctrl, to), Cubic(from, c1, c2, to).
            // Within a contour, consecutive curves chain (prev.to == next.from).
            let mut last_x = f32::NAN;
            let mut last_y = f32::NAN;

            for curve in &outline.curves {
                let from = match curve {
                    ab_glyph::OutlineCurve::Line(p0, _) => p0,
                    ab_glyph::OutlineCurve::Quad(p0, _, _) => p0,
                    ab_glyph::OutlineCurve::Cubic(p0, _, _, _) => p0,
                };

                let fx = glyph_x + from.x;
                // Font Y is up, layout Y is down. Canvas Y is up → negate layout Y:
                let fy = from.y - glyph_y;

                // Start new contour if first curve or "from" doesn't match last endpoint
                if last_x.is_nan() || (fx - last_x).abs() > 0.1 || (fy - last_y).abs() > 0.1 {
                    if !last_x.is_nan() {
                        verbs.push(PathVerb::Close);
                    }
                    verbs.push(PathVerb::MoveTo(Point { x: fx, y: fy }));
                }

                match curve {
                    ab_glyph::OutlineCurve::Line(_, p1) => {
                        let x = glyph_x + p1.x;
                        let y = p1.y - glyph_y;
                        verbs.push(PathVerb::LineTo(Point { x, y }));
                        last_x = x;
                        last_y = y;
                    }
                    ab_glyph::OutlineCurve::Quad(_, ctrl, to) => {
                        verbs.push(PathVerb::QuadTo {
                            ctrl: Point {
                                x: glyph_x + ctrl.x,
                                y: ctrl.y - glyph_y,
                            },
                            to: Point {
                                x: glyph_x + to.x,
                                y: to.y - glyph_y,
                            },
                        });
                        last_x = glyph_x + to.x;
                        last_y = to.y - glyph_y;
                    }
                    ab_glyph::OutlineCurve::Cubic(_, c1, c2, to) => {
                        verbs.push(PathVerb::CubicTo {
                            ctrl1: Point {
                                x: glyph_x + c1.x,
                                y: c1.y - glyph_y,
                            },
                            ctrl2: Point {
                                x: glyph_x + c2.x,
                                y: c2.y - glyph_y,
                            },
                            to: Point {
                                x: glyph_x + to.x,
                                y: to.y - glyph_y,
                            },
                        });
                        last_x = glyph_x + to.x;
                        last_y = to.y - glyph_y;
                    }
                }
            }
            // Close the last contour of this glyph
            if !last_x.is_nan() {
                verbs.push(PathVerb::Close);
            }
        }
    }

    // Path is in font-unit space (large coordinates like 0–2048) for tessellation
    // quality. Return the scale factor so callers can build a Shape with the
    // correct transform (font-units → layout world-space).
    let scale_factor = layout.glyphs[0].size / height_unscaled;
    let scale_xform = Affine2::from_scale(Vec2::splat(scale_factor));
    let transform = text_inst.transform * scale_xform;

    Ok((
        PathData {
            verbs,
            closed: true, // glyph outlines are closed contours
        },
        transform,
    ))
}

// ---------------------------------------------------------------------------
// Execute text node
// ---------------------------------------------------------------------------

pub fn execute_text(
    text: &str,
    font_family: &str,
    font_path: &str,
    inputs: &vector_flow_core::compute::ResolvedInputs,
    font_cache: &mut FontCache,
    project_dir: &str,
) -> Result<(Arc<TextInstance>, f64, f64), ComputeError> {
    let position = super::get_vec2(inputs, 0);
    let font_size = super::get_scalar(inputs, 1);
    let font_weight = super::get_int(inputs, 2).clamp(100, 900) as u16;
    let font_style_int = super::get_int(inputs, 3);
    let letter_spacing = super::get_scalar(inputs, 4);
    let line_height = super::get_scalar(inputs, 5);
    let alignment_int = super::get_int(inputs, 6);
    let box_width = super::get_scalar(inputs, 7);
    let box_height = super::get_scalar(inputs, 8);
    let wrap = super::get_bool(inputs, 9);
    let color = super::get_color(inputs, 10);
    let opacity = super::get_scalar(inputs, 11).clamp(0.0, 1.0) as f32;

    let font_style = match font_style_int {
        1 => FontStyle::Italic,
        2 => FontStyle::Oblique,
        _ => FontStyle::Normal,
    };
    let alignment = match alignment_int {
        1 => TextAlignment::Center,
        2 => TextAlignment::Right,
        _ => TextAlignment::Left,
    };

    let style = TextStyle {
        font_family: font_family.to_string(),
        font_path: font_path.to_string(),
        font_size,
        font_weight,
        font_style,
        letter_spacing,
        line_height,
        alignment,
        wrap,
        box_width,
        box_height,
    };

    let (font_data, font_index) = font_cache.resolve_font(&style, project_dir);
    let layout = layout_text(text, &style, &font_data, font_index)?;
    let bounds = layout.bounds;

    let transform = Affine2::from_translation(position);

    let inst = Arc::new(TextInstance {
        text: text.to_string(),
        style,
        color,
        transform,
        opacity,
        layout: Arc::new(layout),
    });

    Ok((inst, bounds.0 as f64, bounds.1 as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::types::Color;

    #[test]
    fn bundled_font_loads() {
        let font = FontArc::try_from_slice(NOTO_SANS_REGULAR);
        assert!(font.is_ok());
    }

    #[test]
    fn layout_simple_text() {
        let style = TextStyle::default();
        let layout = layout_text("Hello", &style, NOTO_SANS_REGULAR, 0).unwrap();
        assert!(!layout.glyphs.is_empty());
        assert!(layout.bounds.0 > 0.0);
        assert!(layout.bounds.1 > 0.0);
    }

    #[test]
    fn layout_multiline() {
        let style = TextStyle::default();
        let layout = layout_text("Hello\nWorld", &style, NOTO_SANS_REGULAR, 0).unwrap();
        // Should have glyphs on two lines
        let ys: Vec<f32> = layout.glyphs.iter().map(|g| g.y).collect();
        let unique_ys: std::collections::HashSet<u32> = ys.iter().map(|y| *y as u32).collect();
        assert!(unique_ys.len() >= 2);
    }

    #[test]
    fn layout_wrapping() {
        let style = TextStyle {
            box_width: 50.0,
            font_size: 24.0,
            wrap: true,
            ..Default::default()
        };
        let layout = layout_text("Hello World Test", &style, NOTO_SANS_REGULAR, 0).unwrap();
        // With a 50px box, text should wrap
        let ys: Vec<f32> = layout.glyphs.iter().map(|g| g.y).collect();
        let unique_ys: std::collections::HashSet<u32> = ys.iter().map(|y| *y as u32).collect();
        assert!(unique_ys.len() >= 2);
    }

    #[test]
    fn font_cache_resolves_bundled() {
        let mut cache = FontCache::new();
        let style = TextStyle::default();
        let (data, idx) = cache.resolve_font(&style, "");
        assert!(!data.is_empty());
        assert_eq!(idx, 0);
    }

    #[test]
    fn text_to_path_produces_verbs() {
        let style = TextStyle::default();
        let layout = layout_text("A", &style, NOTO_SANS_REGULAR, 0).unwrap();
        let inst = TextInstance {
            text: "A".to_string(),
            style,
            color: Color::WHITE,
            transform: Affine2::IDENTITY,
            opacity: 1.0,
            layout: Arc::new(layout),
        };
        let (path, xform) = text_to_path(&inst).unwrap();
        assert!(!path.verbs.is_empty());
        // Transform should include scale from font units to layout size
        assert_ne!(xform, Affine2::IDENTITY);
    }

    #[test]
    fn outline_has_correct_contour_count() {
        let font = FontArc::try_from_slice(NOTO_SANS_REGULAR).unwrap();
        // Letter "O" has an outer and inner contour
        let glyph_id = font.glyph_id('O');
        let outline = font.outline(glyph_id).expect("O should have outline");

        let mut contour_count = 1usize;
        for i in 1..outline.curves.len() {
            let prev_to = match &outline.curves[i - 1] {
                ab_glyph::OutlineCurve::Line(_, to)
                | ab_glyph::OutlineCurve::Quad(_, _, to)
                | ab_glyph::OutlineCurve::Cubic(_, _, _, to) => to,
            };
            let cur_from = match &outline.curves[i] {
                ab_glyph::OutlineCurve::Line(from, _)
                | ab_glyph::OutlineCurve::Quad(from, _, _)
                | ab_glyph::OutlineCurve::Cubic(from, _, _, _) => from,
            };
            let gap = ((prev_to.x - cur_from.x).powi(2) + (prev_to.y - cur_from.y).powi(2)).sqrt();
            if gap > 0.01 {
                contour_count += 1;
            }
        }
        assert_eq!(contour_count, 2, "O should have 2 contours (outer + hole)");
    }
}
