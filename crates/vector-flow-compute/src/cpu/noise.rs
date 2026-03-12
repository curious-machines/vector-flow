use noise::{Fbm, NoiseFn, OpenSimplex, MultiFractal};

// ---------------------------------------------------------------------------
// Deterministic PRNG helpers
// ---------------------------------------------------------------------------

/// SplitMix64 hash — deterministic random from (seed, index).
pub fn splitmix64(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_mul(0x9E3779B97F4A7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Two uniform floats in [0, 1) from (seed, index).
pub fn rand_pair(seed: u64, index: u64) -> (f32, f32) {
    let h1 = splitmix64(seed, index.wrapping_mul(2));
    let h2 = splitmix64(seed, index.wrapping_mul(2).wrapping_add(1));
    let u1 = (h1 >> 11) as f32 / (1u64 << 53) as f32;
    let u2 = (h2 >> 11) as f32 / (1u64 << 53) as f32;
    (u1, u2)
}

/// Box–Muller transform: two uniform samples → two independent Gaussian samples (mean 0, std 1).
pub fn box_muller(u1: f32, u2: f32) -> (f32, f32) {
    // Clamp u1 away from zero to avoid ln(0).
    let u1 = u1.max(1e-10);
    let r = (-2.0 * u1.ln()).sqrt();
    let theta = std::f32::consts::TAU * u2;
    (r * theta.cos(), r * theta.sin())
}

// ---------------------------------------------------------------------------
// Displacement functions (uniform / gaussian × radial / per-axis)
// ---------------------------------------------------------------------------

/// Random displacement within a circle of given radius.
pub fn uniform_radial(seed: u64, index: u64, amount: f32) -> (f32, f32) {
    let (u1, u2) = rand_pair(seed, index);
    let angle = std::f32::consts::TAU * u1;
    // sqrt(u2) for uniform area distribution
    let r = amount * u2.sqrt();
    (r * angle.cos(), r * angle.sin())
}

/// Random displacement in [-bx, bx] × [-by, by].
pub fn uniform_per_axis(seed: u64, index: u64, bx: f32, by: f32) -> (f32, f32) {
    let (u1, u2) = rand_pair(seed, index);
    let dx = (u1 * 2.0 - 1.0) * bx;
    let dy = (u2 * 2.0 - 1.0) * by;
    (dx, dy)
}

/// Gaussian displacement with radial magnitude (random direction, Gaussian distance).
pub fn gaussian_radial(seed: u64, index: u64, amount: f32) -> (f32, f32) {
    let (u1, u2) = rand_pair(seed, index);
    let (g1, _) = box_muller(u1, u2);
    // Use a second pair for direction.
    let (u3, _) = rand_pair(seed, index.wrapping_add(1_000_000));
    let angle = std::f32::consts::TAU * u3;
    let r = g1.abs() * amount;
    (r * angle.cos(), r * angle.sin())
}

/// Independent Gaussian displacement per axis.
pub fn gaussian_per_axis(seed: u64, index: u64, bx: f32, by: f32) -> (f32, f32) {
    let (u1, u2) = rand_pair(seed, index);
    let (g1, g2) = box_muller(u1, u2);
    (g1 * bx, g2 * by)
}

// ---------------------------------------------------------------------------
// Noise-based displacement
// ---------------------------------------------------------------------------

fn make_fbm(seed: u32, octaves: usize, lacunarity: f64, frequency: f64) -> Fbm<OpenSimplex> {
    Fbm::<OpenSimplex>::new(seed)
        .set_octaves(octaves)
        .set_lacunarity(lacunarity)
        .set_frequency(frequency)
}

/// Noise displacement (radial): two FBM channels offset by seed.
#[allow(clippy::too_many_arguments)]
pub fn noise_displacement(
    x: f32, y: f32, seed: u32, frequency: f64, octaves: usize, lacunarity: f64, amount: f32,
) -> (f32, f32) {
    let fbm_x = make_fbm(seed, octaves, lacunarity, frequency);
    let fbm_y = make_fbm(seed.wrapping_add(1000), octaves, lacunarity, frequency);
    let dx = fbm_x.get([x as f64, y as f64]) as f32 * amount;
    let dy = fbm_y.get([x as f64, y as f64]) as f32 * amount;
    (dx, dy)
}

/// Noise displacement (per-axis): independent amplitude per axis.
#[allow(clippy::too_many_arguments)]
pub fn noise_displacement_per_axis(
    x: f32, y: f32, seed: u32, frequency: f64, octaves: usize, lacunarity: f64, bx: f32, by: f32,
) -> (f32, f32) {
    let fbm_x = make_fbm(seed, octaves, lacunarity, frequency);
    let fbm_y = make_fbm(seed.wrapping_add(1000), octaves, lacunarity, frequency);
    let dx = fbm_x.get([x as f64, y as f64]) as f32 * bx;
    let dy = fbm_y.get([x as f64, y as f64]) as f32 * by;
    (dx, dy)
}

// ---------------------------------------------------------------------------
// Standalone Noise node sampling
// ---------------------------------------------------------------------------

/// Sample FBM noise at each point in the batch. Returns one scalar per point.
#[allow(clippy::too_many_arguments)]
pub fn sample_noise_batch(
    xs: &[f32], ys: &[f32],
    seed: u32, frequency: f64, octaves: usize, lacunarity: f64,
    amplitude: f64, offset_x: f64, offset_y: f64,
) -> Vec<f64> {
    let fbm = make_fbm(seed, octaves, lacunarity, frequency);
    let len = xs.len().min(ys.len());
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let x = xs[i] as f64 + offset_x;
        let y = ys[i] as f64 + offset_y;
        result.push(fbm.get([x, y]) * amplitude);
    }
    result
}

/// Scalar displacement magnitude from noise at a point.
pub fn noise_scalar(
    x: f32, y: f32, seed: u32, frequency: f64, octaves: usize, lacunarity: f64,
) -> f32 {
    let fbm = make_fbm(seed, octaves, lacunarity, frequency);
    fbm.get([x as f64, y as f64]) as f32
}

// ---------------------------------------------------------------------------
// Generic displacement dispatcher (used by perturb module)
// ---------------------------------------------------------------------------

/// Compute displacement for a point given method and parameters.
/// method: 0=Uniform, 1=Gaussian, 2=Noise
#[allow(clippy::too_many_arguments)]
pub fn displace(
    x: f32, y: f32, seed: u64, index: u64,
    method: i32, per_axis: bool,
    amount: f32, amount_x: f32, amount_y: f32,
    frequency: f64, octaves: usize, lacunarity: f64,
) -> (f32, f32) {
    match method {
        0 => {
            // Uniform
            if per_axis {
                uniform_per_axis(seed, index, amount_x, amount_y)
            } else {
                uniform_radial(seed, index, amount)
            }
        }
        1 => {
            // Gaussian
            if per_axis {
                gaussian_per_axis(seed, index, amount_x, amount_y)
            } else {
                gaussian_radial(seed, index, amount)
            }
        }
        _ => {
            // Noise
            if per_axis {
                noise_displacement_per_axis(x, y, seed as u32, frequency, octaves, lacunarity, amount_x, amount_y)
            } else {
                noise_displacement(x, y, seed as u32, frequency, octaves, lacunarity, amount)
            }
        }
    }
}

/// Compute scalar displacement magnitude (used for preserve-smoothness).
#[allow(clippy::too_many_arguments)]
pub fn displace_scalar(
    x: f32, y: f32, seed: u64, index: u64,
    method: i32,
    amount: f32,
    frequency: f64, octaves: usize, lacunarity: f64,
) -> f32 {
    match method {
        0 => {
            let (u1, _) = rand_pair(seed, index);
            (u1 * 2.0 - 1.0) * amount
        }
        1 => {
            let (u1, u2) = rand_pair(seed, index);
            let (g, _) = box_muller(u1, u2);
            g * amount
        }
        _ => {
            noise_scalar(x, y, seed as u32, frequency, octaves, lacunarity) * amount
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_deterministic() {
        let a = splitmix64(42, 0);
        let b = splitmix64(42, 0);
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_different_results() {
        let a = splitmix64(42, 0);
        let b = splitmix64(43, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn rand_pair_in_bounds() {
        for i in 0..100 {
            let (u1, u2) = rand_pair(42, i);
            assert!(u1 >= 0.0 && u1 < 1.0, "u1 out of bounds: {}", u1);
            assert!(u2 >= 0.0 && u2 < 1.0, "u2 out of bounds: {}", u2);
        }
    }

    #[test]
    fn box_muller_produces_values() {
        let (g1, g2) = box_muller(0.5, 0.5);
        assert!(g1.is_finite());
        assert!(g2.is_finite());
    }

    #[test]
    fn uniform_radial_within_bounds() {
        for i in 0..100 {
            let (dx, dy) = uniform_radial(42, i, 10.0);
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(dist <= 10.0 + 1e-5, "dist out of bounds: {}", dist);
        }
    }

    #[test]
    fn uniform_per_axis_within_bounds() {
        for i in 0..100 {
            let (dx, dy) = uniform_per_axis(42, i, 5.0, 10.0);
            assert!(dx.abs() <= 5.0 + 1e-5);
            assert!(dy.abs() <= 10.0 + 1e-5);
        }
    }

    #[test]
    fn noise_displacement_deterministic() {
        let (dx1, dy1) = noise_displacement(1.0, 2.0, 42, 1.0, 4, 2.0, 10.0);
        let (dx2, dy2) = noise_displacement(1.0, 2.0, 42, 1.0, 4, 2.0, 10.0);
        assert_eq!(dx1, dx2);
        assert_eq!(dy1, dy2);
    }

    #[test]
    fn sample_noise_batch_correct_length() {
        let xs = vec![0.0, 1.0, 2.0];
        let ys = vec![0.0, 1.0, 2.0];
        let result = sample_noise_batch(&xs, &ys, 42, 1.0, 4, 2.0, 1.0, 0.0, 0.0);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn sample_noise_batch_different_seeds() {
        let xs = vec![0.0, 1.0];
        let ys = vec![0.0, 1.0];
        let a = sample_noise_batch(&xs, &ys, 42, 1.0, 4, 2.0, 1.0, 0.0, 0.0);
        let b = sample_noise_batch(&xs, &ys, 99, 1.0, 4, 2.0, 1.0, 0.0, 0.0);
        assert_ne!(a, b);
    }

    #[test]
    fn displace_zero_amount_gives_zero() {
        let (dx, dy) = displace(0.0, 0.0, 42, 0, 0, false, 0.0, 0.0, 0.0, 1.0, 4, 2.0);
        assert_eq!(dx, 0.0);
        assert_eq!(dy, 0.0);
    }
}
