//! Node-level color operation functions with batch handling.

use std::sync::Arc;

use vector_flow_core::types::{Color, NodeData};

use super::color_math;

// ---------------------------------------------------------------------------
// Helper: apply a color transform to single or batch
// ---------------------------------------------------------------------------

fn map_color(data: &NodeData, f: impl Fn(Color) -> Color) -> NodeData {
    match data {
        NodeData::Color(c) => NodeData::Color(f(*c)),
        NodeData::Colors(cs) => {
            let mapped: Vec<Color> = cs.iter().map(|c| f(*c)).collect();
            NodeData::Colors(Arc::new(mapped))
        }
        _ => NodeData::Color(f(Color::BLACK)),
    }
}

// ---------------------------------------------------------------------------
// Manipulation nodes
// ---------------------------------------------------------------------------

pub fn adjust_hue(data: &NodeData, amount: f64, absolute: bool) -> NodeData {
    let amount = amount as f32 / 360.0; // degrees -> [0,1] fraction
    map_color(data, |c| {
        let (h, s, l) = color_math::rgb_to_hsl(c);
        let new_h = if absolute {
            amount.rem_euclid(1.0)
        } else {
            (h + amount).rem_euclid(1.0)
        };
        let (r, g, b) = color_math::hsl_to_rgb(new_h, s, l);
        Color { r, g, b, a: c.a }
    })
}

pub fn adjust_saturation(data: &NodeData, amount: f64, absolute: bool) -> NodeData {
    let amount = amount as f32;
    map_color(data, |c| {
        let (h, s, l) = color_math::rgb_to_hsl(c);
        let new_s = if absolute {
            amount.clamp(0.0, 1.0)
        } else {
            (s + amount).clamp(0.0, 1.0)
        };
        let (r, g, b) = color_math::hsl_to_rgb(h, new_s, l);
        Color { r, g, b, a: c.a }
    })
}

pub fn adjust_lightness(data: &NodeData, amount: f64, absolute: bool) -> NodeData {
    let amount = amount as f32;
    map_color(data, |c| {
        let (h, s, l) = color_math::rgb_to_hsl(c);
        let new_l = if absolute {
            amount.clamp(0.0, 1.0)
        } else {
            (l + amount).clamp(0.0, 1.0)
        };
        let (r, g, b) = color_math::hsl_to_rgb(h, s, new_l);
        Color { r, g, b, a: c.a }
    })
}

pub fn adjust_luminance(data: &NodeData, amount: f64, absolute: bool) -> NodeData {
    let amount = amount as f32;
    map_color(data, |c| {
        let (l, a, b) = color_math::rgb_to_lab(c);
        let new_l = if absolute {
            amount.clamp(0.0, 100.0)
        } else {
            (l + amount).clamp(0.0, 100.0)
        };
        let (r, g, bb) = color_math::lab_to_rgb(new_l, a, b);
        Color { r, g, b: bb, a: c.a }
    })
}

pub fn invert_color(data: &NodeData) -> NodeData {
    map_color(data, |c| Color {
        r: 1.0 - c.r,
        g: 1.0 - c.g,
        b: 1.0 - c.b,
        a: c.a,
    })
}

pub fn grayscale(data: &NodeData) -> NodeData {
    map_color(data, |c| {
        let lum = color_math::perceptual_luminance(c);
        Color { r: lum, g: lum, b: lum, a: c.a }
    })
}

// ---------------------------------------------------------------------------
// Blending
// ---------------------------------------------------------------------------

pub fn mix_colors(a: Color, b: Color, factor: f64, lab_mode: bool) -> Color {
    let t = factor as f32;
    if lab_mode {
        color_math::lerp_lab(a, b, t)
    } else {
        color_math::lerp_rgb(a, b, t)
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

pub fn set_alpha(data: &NodeData, alpha: f64) -> NodeData {
    let alpha = (alpha as f32).clamp(0.0, 1.0);
    map_color(data, |c| Color { r: c.r, g: c.g, b: c.b, a: alpha })
}

pub fn color_parse(text: &str) -> Color {
    let text = text.trim();
    if let Some(hex) = parse_hex_color(text) {
        return hex;
    }
    color_math::css_named_color(text).unwrap_or(Color::BLACK)
}

fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some(Color {
                r: r as f32 / 255.0,
                g: g as f32 / 255.0,
                b: b as f32 / 255.0,
                a: 1.0,
            })
        }
        8 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            let a = u8::from_str_radix(&s[6..8], 16).ok()?;
            Some(Color {
                r: r as f32 / 255.0,
                g: g as f32 / 255.0,
                b: b as f32 / 255.0,
                a: a as f32 / 255.0,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn adjust_hue_shift() {
        // Red shifted 120 degrees -> green-ish
        let red = NodeData::Color(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
        if let NodeData::Color(c) = adjust_hue(&red, 120.0, false) {
            // Hue 120 degrees = green
            assert!(c.g > 0.9, "expected green channel high, got {}", c.g);
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn adjust_hue_absolute() {
        let red = NodeData::Color(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
        // Set hue to 240 degrees (blue)
        if let NodeData::Color(c) = adjust_hue(&red, 240.0, true) {
            assert!(c.b > 0.9, "expected blue channel high, got {}", c.b);
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn adjust_saturation_desaturate() {
        let red = NodeData::Color(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
        if let NodeData::Color(c) = adjust_saturation(&red, -1.0, false) {
            // Fully desaturated -> gray
            assert!(approx(c.r, c.g, 1e-3));
            assert!(approx(c.g, c.b, 1e-3));
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn adjust_lightness_brighten() {
        let dark = NodeData::Color(Color { r: 0.2, g: 0.1, b: 0.1, a: 1.0 });
        if let NodeData::Color(c) = adjust_lightness(&dark, 0.5, false) {
            assert!(c.r > 0.2, "expected brighter");
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn adjust_luminance_lab() {
        let mid = NodeData::Color(Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 });
        if let NodeData::Color(c) = adjust_luminance(&mid, 20.0, false) {
            // Should be brighter
            let lum_before = color_math::perceptual_luminance(Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 });
            let lum_after = color_math::perceptual_luminance(c);
            assert!(lum_after > lum_before);
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn invert_color_white_to_black() {
        let white = NodeData::Color(Color::WHITE);
        if let NodeData::Color(c) = invert_color(&white) {
            assert!(approx(c.r, 0.0, 1e-5));
            assert!(approx(c.g, 0.0, 1e-5));
            assert!(approx(c.b, 0.0, 1e-5));
            assert!(approx(c.a, 1.0, 1e-5)); // alpha preserved
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn grayscale_pure_red() {
        let red = NodeData::Color(Color { r: 1.0, g: 0.0, b: 0.0, a: 0.8 });
        if let NodeData::Color(c) = grayscale(&red) {
            assert!(approx(c.r, 0.2126, 1e-3));
            assert!(approx(c.r, c.g, 1e-5));
            assert!(approx(c.g, c.b, 1e-5));
            assert!(approx(c.a, 0.8, 1e-5)); // alpha preserved
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn mix_colors_rgb_midpoint() {
        let r = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
        let b = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
        let mid = mix_colors(r, b, 0.5, false);
        assert!(approx(mid.r, 0.5, 1e-5));
        assert!(approx(mid.b, 0.5, 1e-5));
    }

    #[test]
    fn mix_colors_lab_mode() {
        let r = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
        let b = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
        let mid = mix_colors(r, b, 0.5, true);
        // Lab midpoint should be different from RGB midpoint
        // Just check it's valid
        assert!(mid.r >= 0.0 && mid.r <= 1.0);
        assert!(mid.g >= 0.0 && mid.g <= 1.0);
        assert!(mid.b >= 0.0 && mid.b <= 1.0);
    }

    #[test]
    fn set_alpha_override() {
        let c = NodeData::Color(Color::WHITE);
        if let NodeData::Color(result) = set_alpha(&c, 0.5) {
            assert!(approx(result.a, 0.5, 1e-5));
            assert!(approx(result.r, 1.0, 1e-5)); // RGB unchanged
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn color_parse_hex6() {
        let c = color_parse("#FF6347");
        assert!(approx(c.r, 1.0, 1e-3));
        assert!(approx(c.g, 99.0 / 255.0, 1e-3));
        assert!(approx(c.b, 71.0 / 255.0, 1e-3));
    }

    #[test]
    fn color_parse_hex8() {
        let c = color_parse("#FF634780");
        assert!(approx(c.a, 128.0 / 255.0, 1e-3));
    }

    #[test]
    fn color_parse_named() {
        let c = color_parse("tomato");
        assert!(approx(c.r, 1.0, 1e-3));
    }

    #[test]
    fn color_parse_fallback() {
        let c = color_parse("notacolor");
        assert_eq!(c, Color::BLACK);
    }

    #[test]
    fn batch_adjust_hue() {
        let batch = NodeData::Colors(Arc::new(vec![
            Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
            Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 },
        ]));
        if let NodeData::Colors(cs) = adjust_hue(&batch, 120.0, false) {
            assert_eq!(cs.len(), 2);
        } else {
            panic!("expected Colors batch");
        }
    }
}
