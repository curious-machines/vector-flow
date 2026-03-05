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
    ]
}
