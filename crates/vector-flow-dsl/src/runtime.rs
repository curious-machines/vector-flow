//! Runtime math intrinsics for JIT-compiled DSL code.
//! All functions use `extern "C"` ABI for Cranelift interop.

// ---- 1-arg: f64 → f64 ----

pub extern "C" fn vf_sin(x: f64) -> f64 { x.sin() }
pub extern "C" fn vf_cos(x: f64) -> f64 { x.cos() }
pub extern "C" fn vf_tan(x: f64) -> f64 { x.tan() }
pub extern "C" fn vf_asin(x: f64) -> f64 { x.asin() }
pub extern "C" fn vf_acos(x: f64) -> f64 { x.acos() }
pub extern "C" fn vf_atan(x: f64) -> f64 { x.atan() }
pub extern "C" fn vf_sqrt(x: f64) -> f64 { x.sqrt() }
pub extern "C" fn vf_abs(x: f64) -> f64 { x.abs() }
pub extern "C" fn vf_floor(x: f64) -> f64 { x.floor() }
pub extern "C" fn vf_ceil(x: f64) -> f64 { x.ceil() }
pub extern "C" fn vf_round(x: f64) -> f64 { x.round() }
pub extern "C" fn vf_fract(x: f64) -> f64 { x.fract() }
pub extern "C" fn vf_exp(x: f64) -> f64 { x.exp() }
pub extern "C" fn vf_ln(x: f64) -> f64 { x.ln() }
pub extern "C" fn vf_sign(x: f64) -> f64 { x.signum() }

// ---- 2-arg: (f64, f64) → f64 ----

pub extern "C" fn vf_min(a: f64, b: f64) -> f64 { a.min(b) }
pub extern "C" fn vf_max(a: f64, b: f64) -> f64 { a.max(b) }
pub extern "C" fn vf_pow(base: f64, exp: f64) -> f64 { base.powf(exp) }
pub extern "C" fn vf_atan2(y: f64, x: f64) -> f64 { y.atan2(x) }
pub extern "C" fn vf_fmod(a: f64, b: f64) -> f64 { a % b }
pub extern "C" fn vf_step(edge: f64, x: f64) -> f64 {
    if x < edge { 0.0 } else { 1.0 }
}

// ---- 3-arg: (f64, f64, f64) → f64 ----

pub extern "C" fn vf_lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub extern "C" fn vf_clamp(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

pub extern "C" fn vf_smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---- Procedural ----

/// Deterministic hash-based random: maps seed → [0, 1).
pub extern "C" fn vf_rand(seed: u64) -> f64 {
    // Simple xorshift-style hash
    let mut h = seed;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^= h >> 33;
    (h & 0x001f_ffff_ffff_ffff) as f64 / (0x0020_0000_0000_0000u64 as f64)
}

/// Simple 2D value noise.
pub extern "C" fn vf_noise(x: f64, y: f64) -> f64 {
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - x.floor();
    let fy = y - y.floor();

    // Smooth interpolation weights
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);

    let hash = |px: i64, py: i64| -> f64 {
        let seed = (px.wrapping_mul(1597) ^ py.wrapping_mul(51749)) as u64;
        vf_rand(seed)
    };

    let v00 = hash(ix, iy);
    let v10 = hash(ix + 1, iy);
    let v01 = hash(ix, iy + 1);
    let v11 = hash(ix + 1, iy + 1);

    let a = v00 + (v10 - v00) * ux;
    let b = v01 + (v11 - v01) * ux;
    a + (b - a) * uy
}

// ---- Int ops ----

pub extern "C" fn vf_iabs(x: i64) -> i64 { x.abs() }
pub extern "C" fn vf_imin(a: i64, b: i64) -> i64 { a.min(b) }
pub extern "C" fn vf_imax(a: i64, b: i64) -> i64 { a.max(b) }

// ---- Color math (pure, no external deps) ----

/// HSL→RGB helper. h, s, l in [0,1].
fn hsl_to_rgb_f64(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s.abs() < 1e-7 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let r = hue_to_rgb_f64(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb_f64(p, q, h);
    let b = hue_to_rgb_f64(p, q, h - 1.0 / 3.0);
    (r, g, b)
}

fn hue_to_rgb_f64(p: f64, q: f64, t: f64) -> f64 {
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

/// RGB→HSL. Returns (h, s, l) where all in [0,1].
fn rgb_to_hsl_f64(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    if (max - min).abs() < 1e-7 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if (max - r).abs() < 1e-7 {
        let mut h = (g - b) / d;
        if g < b { h += 6.0; }
        h
    } else if (max - g).abs() < 1e-7 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

// ---- Color construction: write 4 f64 (r,g,b,a) to consecutive slots ----
// These functions receive raw pointers from JIT-compiled code, which guarantees validity.

/// hsl(h_deg, s_pct, l_pct) → color at dest_slot..dest_slot+3
/// h in degrees [0,360], s and l in percent [0,100].
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_hsl(slots_ptr: *mut f64, h: f64, s: f64, l: f64, dest_slot: u32) {
    let (r, g, b) = hsl_to_rgb_f64(h / 360.0, s / 100.0, l / 100.0);
    unsafe {
        *slots_ptr.add(dest_slot as usize) = r;
        *slots_ptr.add(dest_slot as usize + 1) = g;
        *slots_ptr.add(dest_slot as usize + 2) = b;
        *slots_ptr.add(dest_slot as usize + 3) = 1.0;
    }
}

/// hsla(h_deg, s_pct, l_pct, a) → color at dest_slot..dest_slot+3
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_hsla(slots_ptr: *mut f64, h: f64, s: f64, l: f64, a: f64, dest_slot: u32) {
    let (r, g, b) = hsl_to_rgb_f64(h / 360.0, s / 100.0, l / 100.0);
    unsafe {
        *slots_ptr.add(dest_slot as usize) = r;
        *slots_ptr.add(dest_slot as usize + 1) = g;
        *slots_ptr.add(dest_slot as usize + 2) = b;
        *slots_ptr.add(dest_slot as usize + 3) = a.clamp(0.0, 1.0);
    }
}

/// rgb(r, g, b) → color at dest_slot..dest_slot+3. Components in [0,1].
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_rgb(slots_ptr: *mut f64, r: f64, g: f64, b: f64, dest_slot: u32) {
    unsafe {
        *slots_ptr.add(dest_slot as usize) = r.clamp(0.0, 1.0);
        *slots_ptr.add(dest_slot as usize + 1) = g.clamp(0.0, 1.0);
        *slots_ptr.add(dest_slot as usize + 2) = b.clamp(0.0, 1.0);
        *slots_ptr.add(dest_slot as usize + 3) = 1.0;
    }
}

/// rgba(r, g, b, a) → color at dest_slot..dest_slot+3. Components in [0,1].
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_rgba(slots_ptr: *mut f64, r: f64, g: f64, b: f64, a: f64, dest_slot: u32) {
    unsafe {
        *slots_ptr.add(dest_slot as usize) = r.clamp(0.0, 1.0);
        *slots_ptr.add(dest_slot as usize + 1) = g.clamp(0.0, 1.0);
        *slots_ptr.add(dest_slot as usize + 2) = b.clamp(0.0, 1.0);
        *slots_ptr.add(dest_slot as usize + 3) = a.clamp(0.0, 1.0);
    }
}

// ---- Color component extractors: read from slots ----

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_r(slots_ptr: *const f64, src_slot: u32) -> f64 {
    unsafe { *slots_ptr.add(src_slot as usize) }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_g(slots_ptr: *const f64, src_slot: u32) -> f64 {
    unsafe { *slots_ptr.add(src_slot as usize + 1) }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_b(slots_ptr: *const f64, src_slot: u32) -> f64 {
    unsafe { *slots_ptr.add(src_slot as usize + 2) }
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_a(slots_ptr: *const f64, src_slot: u32) -> f64 {
    unsafe { *slots_ptr.add(src_slot as usize + 3) }
}

/// Extract hue (degrees 0..360) from color at src_slot.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_hue(slots_ptr: *const f64, src_slot: u32) -> f64 {
    let (r, g, b) = unsafe {(
        *slots_ptr.add(src_slot as usize),
        *slots_ptr.add(src_slot as usize + 1),
        *slots_ptr.add(src_slot as usize + 2),
    )};
    let (h, _, _) = rgb_to_hsl_f64(r, g, b);
    h * 360.0
}

/// Extract saturation (0..100) from color at src_slot.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_sat(slots_ptr: *const f64, src_slot: u32) -> f64 {
    let (r, g, b) = unsafe {(
        *slots_ptr.add(src_slot as usize),
        *slots_ptr.add(src_slot as usize + 1),
        *slots_ptr.add(src_slot as usize + 2),
    )};
    let (_, s, _) = rgb_to_hsl_f64(r, g, b);
    s * 100.0
}

/// Extract lightness (0..100) from color at src_slot.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_light(slots_ptr: *const f64, src_slot: u32) -> f64 {
    let (r, g, b) = unsafe {(
        *slots_ptr.add(src_slot as usize),
        *slots_ptr.add(src_slot as usize + 1),
        *slots_ptr.add(src_slot as usize + 2),
    )};
    let (_, _, l) = rgb_to_hsl_f64(r, g, b);
    l * 100.0
}

// ---- Color modification: read color from src_slot, modify, write to dest_slot ----

/// Set lightness of color at src_slot to `val` (0..100), write result to dest_slot.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_set_lightness(slots_ptr: *mut f64, src_slot: u32, val: f64, dest_slot: u32) {
    let (r, g, b, a) = unsafe {(
        *slots_ptr.add(src_slot as usize),
        *slots_ptr.add(src_slot as usize + 1),
        *slots_ptr.add(src_slot as usize + 2),
        *slots_ptr.add(src_slot as usize + 3),
    )};
    let (h, s, _) = rgb_to_hsl_f64(r, g, b);
    let (nr, ng, nb) = hsl_to_rgb_f64(h, s, (val / 100.0).clamp(0.0, 1.0));
    unsafe {
        *slots_ptr.add(dest_slot as usize) = nr;
        *slots_ptr.add(dest_slot as usize + 1) = ng;
        *slots_ptr.add(dest_slot as usize + 2) = nb;
        *slots_ptr.add(dest_slot as usize + 3) = a;
    }
}

/// Set saturation of color at src_slot to `val` (0..100), write result to dest_slot.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_set_saturation(slots_ptr: *mut f64, src_slot: u32, val: f64, dest_slot: u32) {
    let (r, g, b, a) = unsafe {(
        *slots_ptr.add(src_slot as usize),
        *slots_ptr.add(src_slot as usize + 1),
        *slots_ptr.add(src_slot as usize + 2),
        *slots_ptr.add(src_slot as usize + 3),
    )};
    let (h, _, l) = rgb_to_hsl_f64(r, g, b);
    let (nr, ng, nb) = hsl_to_rgb_f64(h, (val / 100.0).clamp(0.0, 1.0), l);
    unsafe {
        *slots_ptr.add(dest_slot as usize) = nr;
        *slots_ptr.add(dest_slot as usize + 1) = ng;
        *slots_ptr.add(dest_slot as usize + 2) = nb;
        *slots_ptr.add(dest_slot as usize + 3) = a;
    }
}

/// Set hue of color at src_slot to `val` (0..360), write result to dest_slot.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_set_hue(slots_ptr: *mut f64, src_slot: u32, val: f64, dest_slot: u32) {
    let (r, g, b, a) = unsafe {(
        *slots_ptr.add(src_slot as usize),
        *slots_ptr.add(src_slot as usize + 1),
        *slots_ptr.add(src_slot as usize + 2),
        *slots_ptr.add(src_slot as usize + 3),
    )};
    let (_, s, l) = rgb_to_hsl_f64(r, g, b);
    let (nr, ng, nb) = hsl_to_rgb_f64((val / 360.0).rem_euclid(1.0), s, l);
    unsafe {
        *slots_ptr.add(dest_slot as usize) = nr;
        *slots_ptr.add(dest_slot as usize + 1) = ng;
        *slots_ptr.add(dest_slot as usize + 2) = nb;
        *slots_ptr.add(dest_slot as usize + 3) = a;
    }
}

/// Set alpha of color at src_slot, write result to dest_slot.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_set_alpha_color(slots_ptr: *mut f64, src_slot: u32, val: f64, dest_slot: u32) {
    unsafe {
        *slots_ptr.add(dest_slot as usize) = *slots_ptr.add(src_slot as usize);
        *slots_ptr.add(dest_slot as usize + 1) = *slots_ptr.add(src_slot as usize + 1);
        *slots_ptr.add(dest_slot as usize + 2) = *slots_ptr.add(src_slot as usize + 2);
        *slots_ptr.add(dest_slot as usize + 3) = val.clamp(0.0, 1.0);
    }
}

/// Copy 4 color slots from src to dest.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn vf_color_copy(slots_ptr: *mut f64, src_slot: u32, dest_slot: u32) {
    unsafe {
        for i in 0..4 {
            *slots_ptr.add(dest_slot as usize + i) = *slots_ptr.add(src_slot as usize + i);
        }
    }
}

/// List of all runtime symbols for Cranelift registration.
pub fn runtime_symbols() -> Vec<(&'static str, *const u8)> {
    vec![
        // 1-arg f64→f64
        ("vf_sin", vf_sin as *const u8),
        ("vf_cos", vf_cos as *const u8),
        ("vf_tan", vf_tan as *const u8),
        ("vf_asin", vf_asin as *const u8),
        ("vf_acos", vf_acos as *const u8),
        ("vf_atan", vf_atan as *const u8),
        ("vf_sqrt", vf_sqrt as *const u8),
        ("vf_abs", vf_abs as *const u8),
        ("vf_floor", vf_floor as *const u8),
        ("vf_ceil", vf_ceil as *const u8),
        ("vf_round", vf_round as *const u8),
        ("vf_fract", vf_fract as *const u8),
        ("vf_exp", vf_exp as *const u8),
        ("vf_ln", vf_ln as *const u8),
        ("vf_sign", vf_sign as *const u8),
        // 2-arg f64→f64
        ("vf_min", vf_min as *const u8),
        ("vf_max", vf_max as *const u8),
        ("vf_pow", vf_pow as *const u8),
        ("vf_atan2", vf_atan2 as *const u8),
        ("vf_fmod", vf_fmod as *const u8),
        ("vf_step", vf_step as *const u8),
        // 3-arg f64→f64
        ("vf_lerp", vf_lerp as *const u8),
        ("vf_clamp", vf_clamp as *const u8),
        ("vf_smoothstep", vf_smoothstep as *const u8),
        // Procedural
        ("vf_rand", vf_rand as *const u8),
        ("vf_noise", vf_noise as *const u8),
        // Int ops
        ("vf_iabs", vf_iabs as *const u8),
        ("vf_imin", vf_imin as *const u8),
        ("vf_imax", vf_imax as *const u8),
        // Color construction (slots_ptr, args..., dest_slot) -> void
        ("vf_hsl", vf_hsl as *const u8),
        ("vf_hsla", vf_hsla as *const u8),
        ("vf_rgb", vf_rgb as *const u8),
        ("vf_rgba", vf_rgba as *const u8),
        // Color component extractors (slots_ptr, src_slot) -> f64
        ("vf_color_r", vf_color_r as *const u8),
        ("vf_color_g", vf_color_g as *const u8),
        ("vf_color_b", vf_color_b as *const u8),
        ("vf_color_a", vf_color_a as *const u8),
        ("vf_color_hue", vf_color_hue as *const u8),
        ("vf_color_sat", vf_color_sat as *const u8),
        ("vf_color_light", vf_color_light as *const u8),
        // Color modification (slots_ptr, src_slot, val, dest_slot) -> void
        ("vf_set_lightness", vf_set_lightness as *const u8),
        ("vf_set_saturation", vf_set_saturation as *const u8),
        ("vf_set_hue", vf_set_hue as *const u8),
        ("vf_set_alpha_color", vf_set_alpha_color as *const u8),
        // Color utility
        ("vf_color_copy", vf_color_copy as *const u8),
    ]
}
