//! Pure color math utilities: conversions, interpolation, CSS named colors.

use vector_flow_core::types::Color;

// ---------------------------------------------------------------------------
// RGB <-> HSL
// ---------------------------------------------------------------------------

/// Returns (h, s, l) where h is in [0, 1), s and l in [0, 1].
pub fn rgb_to_hsl(c: Color) -> (f32, f32, f32) {
    let r = c.r;
    let g = c.g;
    let b = c.b;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;

    if (max - min).abs() < 1e-7 {
        return (0.0, 0.0, l);
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - r).abs() < 1e-7 {
        let mut h = (g - b) / d;
        if g < b {
            h += 6.0;
        }
        h
    } else if (max - g).abs() < 1e-7 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };

    (h / 6.0, s, l)
}

/// Convert HSL (h in [0,1), s,l in [0,1]) to RGB.
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s.abs() < 1e-7 {
        return (l, l, l);
    }

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;

    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);

    (r, g, b)
}

fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

// ---------------------------------------------------------------------------
// RGB <-> CIE Lab (D65)
// ---------------------------------------------------------------------------

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

// D65 reference white
const XN: f32 = 0.950489;
const YN: f32 = 1.0;
const ZN: f32 = 1.088_84;

fn lab_f(t: f32) -> f32 {
    const DELTA: f32 = 6.0 / 29.0;
    if t > DELTA * DELTA * DELTA {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

fn lab_f_inv(t: f32) -> f32 {
    const DELTA: f32 = 6.0 / 29.0;
    if t > DELTA {
        t * t * t
    } else {
        3.0 * DELTA * DELTA * (t - 4.0 / 29.0)
    }
}

/// Convert sRGB Color to CIE Lab. Returns (L, a, b) where L is [0, 100].
pub fn rgb_to_lab(c: Color) -> (f32, f32, f32) {
    let rl = srgb_to_linear(c.r);
    let gl = srgb_to_linear(c.g);
    let bl = srgb_to_linear(c.b);

    // sRGB -> XYZ (D65)
    let x = 0.4124564 * rl + 0.3575761 * gl + 0.1804375 * bl;
    let y = 0.2126729 * rl + 0.7151522 * gl + 0.0721750 * bl;
    let z = 0.0193339 * rl + 0.119_192 * gl + 0.9503041 * bl;

    let fx = lab_f(x / XN);
    let fy = lab_f(y / YN);
    let fz = lab_f(z / ZN);

    let l_star = 116.0 * fy - 16.0;
    let a_star = 500.0 * (fx - fy);
    let b_star = 200.0 * (fy - fz);

    (l_star, a_star, b_star)
}

/// Convert CIE Lab to sRGB Color. Clamps to [0,1].
pub fn lab_to_rgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;

    let x = XN * lab_f_inv(fx);
    let y = YN * lab_f_inv(fy);
    let z = ZN * lab_f_inv(fz);

    // XYZ -> linear RGB
    let rl = 3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let gl = -0.969_266 * x + 1.8760108 * y + 0.0415560 * z;
    let bl = 0.0556434 * x - 0.2040259 * y + 1.0572252 * z;

    (
        linear_to_srgb(rl).clamp(0.0, 1.0),
        linear_to_srgb(gl).clamp(0.0, 1.0),
        linear_to_srgb(bl).clamp(0.0, 1.0),
    )
}

// ---------------------------------------------------------------------------
// Interpolation
// ---------------------------------------------------------------------------

pub fn lerp_rgb(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

pub fn lerp_lab(c0: Color, c1: Color, t: f32) -> Color {
    let (l0, a0, b0) = rgb_to_lab(c0);
    let (l1, a1, b1) = rgb_to_lab(c1);
    let l = l0 + (l1 - l0) * t;
    let aa = a0 + (a1 - a0) * t;
    let bb = b0 + (b1 - b0) * t;
    let (r, g, b) = lab_to_rgb(l, aa, bb);
    let alpha = c0.a + (c1.a - c0.a) * t;
    Color { r, g, b, a: alpha }
}

// ---------------------------------------------------------------------------
// Perceptual luminance (BT.709)
// ---------------------------------------------------------------------------

pub fn perceptual_luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

// ---------------------------------------------------------------------------
// CSS Named Colors lookup
// ---------------------------------------------------------------------------

pub fn css_named_color(name: &str) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    let (r, g, b) = match lower.as_str() {
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aqua" => (0, 255, 255),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "black" => (0, 0, 0),
        "blanchedalmond" => (255, 235, 205),
        "blue" => (0, 0, 255),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "burlywood" => (222, 184, 135),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "cornsilk" => (255, 248, 220),
        "crimson" => (220, 20, 60),
        "cyan" => (0, 255, 255),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkkhaki" => (189, 183, 107),
        "darkmagenta" => (139, 0, 139),
        "darkolivegreen" => (85, 107, 47),
        "darkorange" => (255, 140, 0),
        "darkorchid" => (153, 50, 204),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkseagreen" => (143, 188, 143),
        "darkslateblue" => (72, 61, 139),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "floralwhite" => (255, 250, 240),
        "forestgreen" => (34, 139, 34),
        "fuchsia" => (255, 0, 255),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "gray" | "grey" => (128, 128, 128),
        "green" => (0, 128, 0),
        "greenyellow" => (173, 255, 47),
        "honeydew" => (240, 255, 240),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lavenderblush" => (255, 240, 245),
        "lawngreen" => (124, 252, 0),
        "lemonchiffon" => (255, 250, 205),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "lightyellow" => (255, 255, 224),
        "lime" => (0, 255, 0),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "magenta" => (255, 0, 255),
        "maroon" => (128, 0, 0),
        "mediumaquamarine" => (102, 205, 170),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 111, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumspringgreen" => (0, 250, 154),
        "mediumturquoise" => (72, 209, 204),
        "mediumvioletred" => (199, 21, 133),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "navajowhite" => (255, 222, 173),
        "navy" => (0, 0, 128),
        "oldlace" => (253, 245, 230),
        "olive" => (128, 128, 0),
        "olivedrab" => (107, 142, 35),
        "orange" => (255, 165, 0),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "purple" => (128, 0, 128),
        "rebeccapurple" => (102, 51, 153),
        "red" => (255, 0, 0),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "silver" => (192, 192, 192),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" | "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "teal" => (0, 128, 128),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "white" => (255, 255, 255),
        "whitesmoke" => (245, 245, 245),
        "yellow" => (255, 255, 0),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    };
    Some(Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    })
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
    fn hsl_round_trip() {
        let colors = [
            Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
            Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 },
            Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
            Color { r: 0.5, g: 0.3, b: 0.8, a: 1.0 },
            Color::WHITE,
            Color::BLACK,
        ];
        for c in &colors {
            let (h, s, l) = rgb_to_hsl(*c);
            let (r, g, b) = hsl_to_rgb(h, s, l);
            assert!(approx(r, c.r, 1e-4), "r: {} vs {}", r, c.r);
            assert!(approx(g, c.g, 1e-4), "g: {} vs {}", g, c.g);
            assert!(approx(b, c.b, 1e-4), "b: {} vs {}", b, c.b);
        }
    }

    #[test]
    fn lab_round_trip() {
        let colors = [
            Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
            Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 },
            Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
            Color { r: 0.5, g: 0.3, b: 0.8, a: 1.0 },
            Color::WHITE,
            Color::BLACK,
        ];
        for c in &colors {
            let (l, a, b) = rgb_to_lab(*c);
            let (r, g, bb) = lab_to_rgb(l, a, b);
            assert!(approx(r, c.r, 1e-3), "r: {} vs {} (L={l})", r, c.r);
            assert!(approx(g, c.g, 1e-3), "g: {} vs {} (L={l})", g, c.g);
            assert!(approx(bb, c.b, 1e-3), "b: {} vs {} (L={l})", bb, c.b);
        }
    }

    #[test]
    fn lerp_rgb_midpoint() {
        let a = Color::BLACK;
        let b = Color::WHITE;
        let mid = lerp_rgb(a, b, 0.5);
        assert!(approx(mid.r, 0.5, 1e-5));
        assert!(approx(mid.g, 0.5, 1e-5));
        assert!(approx(mid.b, 0.5, 1e-5));
    }

    #[test]
    fn lerp_lab_endpoints() {
        let a = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
        let b = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
        let start = lerp_lab(a, b, 0.0);
        assert!(approx(start.r, a.r, 1e-3));
        let end = lerp_lab(a, b, 1.0);
        assert!(approx(end.b, b.b, 1e-3));
    }

    #[test]
    fn luminance_white_black() {
        assert!(approx(perceptual_luminance(Color::WHITE), 1.0, 1e-4));
        assert!(approx(perceptual_luminance(Color::BLACK), 0.0, 1e-4));
    }

    #[test]
    fn css_named_colors_lookup() {
        let tomato = css_named_color("tomato").unwrap();
        assert!(approx(tomato.r, 255.0 / 255.0, 1e-3));
        assert!(approx(tomato.g, 99.0 / 255.0, 1e-3));
        assert!(approx(tomato.b, 71.0 / 255.0, 1e-3));

        let cornflower = css_named_color("CornflowerBlue").unwrap();
        assert!(approx(cornflower.r, 100.0 / 255.0, 1e-3));

        assert!(css_named_color("notacolor").is_none());
    }

    #[test]
    fn hsl_red_is_hue_zero() {
        let (h, s, l) = rgb_to_hsl(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
        assert!(approx(h, 0.0, 1e-5));
        assert!(approx(s, 1.0, 1e-5));
        assert!(approx(l, 0.5, 1e-5));
    }

    #[test]
    fn lab_white_l_is_100() {
        let (l, a, b) = rgb_to_lab(Color::WHITE);
        assert!(approx(l, 100.0, 0.1));
        assert!(approx(a, 0.0, 0.5));
        assert!(approx(b, 0.0, 0.5));
    }
}
