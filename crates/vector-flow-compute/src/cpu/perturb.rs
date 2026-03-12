use std::sync::Arc;

use vector_flow_core::types::{NodeData, PathData, PathVerb, Point, PointBatch, Shape};

use super::noise;
use super::transforms;

/// Perturb points/path geometry by random or noise-based displacement.
///
/// `method`: 0=Uniform, 1=Gaussian, 2=Noise
/// `target`: 0=Anchors, 1=Handles Only, 2=Both
#[allow(clippy::too_many_arguments)]
pub fn perturb_points(
    geometry: &NodeData,
    seed: i64,
    method: i32,
    target: i32,
    per_axis: bool,
    preserve_smoothness: bool,
    amount: f64,
    amount_x: f64,
    amount_y: f64,
    frequency: f64,
    octaves: i64,
    lacunarity: f64,
    handle_scale: f64,
) -> NodeData {
    let seed = seed as u64;
    let amount = amount as f32;
    let amount_x = amount_x as f32;
    let amount_y = amount_y as f32;
    let handle_scale = handle_scale as f32;
    let octaves = (octaves.max(1) as usize).min(32);

    match geometry {
        NodeData::Points(batch) => {
            let perturbed = perturb_point_batch(
                batch, seed, method, per_axis, amount, amount_x, amount_y,
                frequency, octaves, lacunarity,
            );
            NodeData::Points(Arc::new(perturbed))
        }
        NodeData::Path(p) => {
            let perturbed = perturb_path_data(
                p, seed, method, target, per_axis, preserve_smoothness,
                amount, amount_x, amount_y, frequency, octaves, lacunarity, handle_scale,
            );
            NodeData::Path(Arc::new(perturbed))
        }
        NodeData::Paths(paths) => {
            let perturbed: Vec<PathData> = paths.iter().enumerate().map(|(pi, p)| {
                let path_seed = seed.wrapping_add(pi as u64 * 100_000);
                perturb_path_data(
                    p, path_seed, method, target, per_axis, preserve_smoothness,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity, handle_scale,
                )
            }).collect();
            NodeData::Paths(Arc::new(perturbed))
        }
        NodeData::Shape(s) => {
            let perturbed_path = perturb_path_data(
                &s.path, seed, method, target, per_axis, preserve_smoothness,
                amount, amount_x, amount_y, frequency, octaves, lacunarity, handle_scale,
            );
            NodeData::Shape(Arc::new(Shape {
                path: Arc::new(perturbed_path),
                ..(**s).clone()
            }))
        }
        NodeData::Shapes(shapes) => {
            let perturbed: Vec<Shape> = shapes.iter().enumerate().map(|(si, s)| {
                let shape_seed = seed.wrapping_add(si as u64 * 100_000);
                let baked = transforms::bake_shape_to_path(s);
                let perturbed_path = perturb_path_data(
                    &baked, shape_seed, method, target, per_axis, preserve_smoothness,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity, handle_scale,
                );
                Shape {
                    path: Arc::new(perturbed_path),
                    transform: glam::Affine2::IDENTITY,
                    ..s.clone()
                }
            }).collect();
            NodeData::Shapes(Arc::new(perturbed))
        }
        other => other.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn perturb_point_batch(
    batch: &PointBatch,
    seed: u64,
    method: i32,
    per_axis: bool,
    amount: f32,
    amount_x: f32,
    amount_y: f32,
    frequency: f64,
    octaves: usize,
    lacunarity: f64,
) -> PointBatch {
    let len = batch.len();
    let mut xs = Vec::with_capacity(len);
    let mut ys = Vec::with_capacity(len);
    for i in 0..len {
        let x = batch.xs[i];
        let y = batch.ys[i];
        let (dx, dy) = noise::displace(
            x, y, seed, i as u64,
            method, per_axis, amount, amount_x, amount_y,
            frequency, octaves, lacunarity,
        );
        xs.push(x + dx);
        ys.push(y + dy);
    }
    PointBatch { xs, ys }
}

#[allow(clippy::too_many_arguments)]
fn perturb_path_data(
    path: &PathData,
    seed: u64,
    method: i32,
    target: i32,
    per_axis: bool,
    preserve_smoothness: bool,
    amount: f32,
    amount_x: f32,
    amount_y: f32,
    frequency: f64,
    octaves: usize,
    lacunarity: f64,
    handle_scale: f32,
) -> PathData {
    let mut result = PathData { verbs: Vec::with_capacity(path.verbs.len()), closed: path.closed };
    let mut point_idx: u64 = 0;
    // Offset for handle seeds to avoid correlation with anchor seeds.
    let handle_seed_offset: u64 = 500_000;

    // Track current anchor position (after perturbation) for handle computations.
    let mut last_anchor = Point { x: 0.0, y: 0.0 };
    // Track pending anchor delta for coherent mode.
    let mut last_anchor_delta = (0.0f32, 0.0f32);

    // Track subpath start for closed-path coherence: when the last anchor of a
    // closed subpath coincides with the MoveTo, reuse the MoveTo's delta so the
    // path closes smoothly without a seam.
    let mut subpath_start_pos = Point { x: 0.0, y: 0.0 };
    let mut subpath_start_perturbed = Point { x: 0.0, y: 0.0 };
    let mut subpath_start_delta = (0.0f32, 0.0f32);
    let mut subpath_start_idx: u64 = 0;

    for verb in &path.verbs {
        match verb {
            PathVerb::MoveTo(pt) => {
                let (new_pt, delta) = perturb_anchor(
                    pt, seed, point_idx, method, target, per_axis,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity,
                );
                last_anchor = new_pt;
                last_anchor_delta = delta;
                subpath_start_pos = *pt;
                subpath_start_perturbed = new_pt;
                subpath_start_delta = delta;
                subpath_start_idx = point_idx;
                point_idx += 1;
                result.verbs.push(PathVerb::MoveTo(new_pt));
            }
            PathVerb::LineTo(pt) => {
                let (new_pt, delta) = perturb_anchor_or_reuse_start(
                    pt, seed, point_idx, method, target, per_axis,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity,
                    &subpath_start_pos, &subpath_start_perturbed, &subpath_start_delta,
                    subpath_start_idx,
                );
                last_anchor = new_pt;
                last_anchor_delta = delta;
                point_idx += 1;
                result.verbs.push(PathVerb::LineTo(new_pt));
            }
            PathVerb::QuadTo { ctrl, to } => {
                // Perturb the "to" anchor.
                let (new_to, to_delta) = perturb_anchor_or_reuse_start(
                    to, seed, point_idx, method, target, per_axis,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity,
                    &subpath_start_pos, &subpath_start_perturbed, &subpath_start_delta,
                    subpath_start_idx,
                );
                point_idx += 1;

                // Perturb the control point (handle).
                let new_ctrl = perturb_handle(
                    ctrl, &last_anchor, seed, point_idx + handle_seed_offset,
                    method, target, per_axis, preserve_smoothness,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity,
                    handle_scale, &last_anchor_delta,
                );
                point_idx += 1;

                last_anchor = new_to;
                last_anchor_delta = to_delta;
                result.verbs.push(PathVerb::QuadTo { ctrl: new_ctrl, to: new_to });
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                // Perturb the "to" anchor (reuse subpath start delta if closing).
                let (new_to, to_delta) = perturb_anchor_or_reuse_start(
                    to, seed, point_idx, method, target, per_axis,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity,
                    &subpath_start_pos, &subpath_start_perturbed, &subpath_start_delta,
                    subpath_start_idx,
                );
                point_idx += 1;

                // Perturb ctrl1 (relative to last_anchor).
                let new_ctrl1 = perturb_handle(
                    ctrl1, &last_anchor, seed, point_idx + handle_seed_offset,
                    method, target, per_axis, preserve_smoothness,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity,
                    handle_scale, &last_anchor_delta,
                );
                point_idx += 1;

                // Perturb ctrl2 (relative to new_to anchor).
                let new_ctrl2 = perturb_handle(
                    ctrl2, &new_to, seed, point_idx + handle_seed_offset,
                    method, target, per_axis, preserve_smoothness,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity,
                    handle_scale, &to_delta,
                );
                point_idx += 1;

                last_anchor = new_to;
                last_anchor_delta = to_delta;
                result.verbs.push(PathVerb::CubicTo { ctrl1: new_ctrl1, ctrl2: new_ctrl2, to: new_to });
            }
            PathVerb::Close => {
                result.verbs.push(PathVerb::Close);
            }
        }
    }

    result
}

/// Perturb an anchor point. Returns (new_point, delta).
/// target: 0=Anchors, 1=Handles Only, 2=Both
#[allow(clippy::too_many_arguments)]
fn perturb_anchor(
    pt: &Point,
    seed: u64,
    index: u64,
    method: i32,
    target: i32,
    per_axis: bool,
    amount: f32,
    amount_x: f32,
    amount_y: f32,
    frequency: f64,
    octaves: usize,
    lacunarity: f64,
) -> (Point, (f32, f32)) {
    // target=1 (Handles Only) → anchors unchanged
    if target == 1 {
        return (*pt, (0.0, 0.0));
    }

    let (dx, dy) = noise::displace(
        pt.x, pt.y, seed, index,
        method, per_axis, amount, amount_x, amount_y,
        frequency, octaves, lacunarity,
    );

    (Point { x: pt.x + dx, y: pt.y + dy }, (dx, dy))
}

/// Perturb an anchor, but if it coincides with the subpath start position,
/// reuse the start's delta instead. This ensures closed paths close smoothly.
#[allow(clippy::too_many_arguments)]
fn perturb_anchor_or_reuse_start(
    pt: &Point,
    seed: u64,
    index: u64,
    method: i32,
    target: i32,
    per_axis: bool,
    amount: f32,
    amount_x: f32,
    amount_y: f32,
    frequency: f64,
    octaves: usize,
    lacunarity: f64,
    subpath_start_pos: &Point,
    subpath_start_perturbed: &Point,
    subpath_start_delta: &(f32, f32),
    subpath_start_idx: u64,
) -> (Point, (f32, f32)) {
    // If this anchor is at the same position as the subpath start (and isn't the
    // start itself), reuse the start's perturbation for a seamless close.
    if index != subpath_start_idx
        && (pt.x - subpath_start_pos.x).abs() < 1e-3
        && (pt.y - subpath_start_pos.y).abs() < 1e-3
    {
        return (*subpath_start_perturbed, *subpath_start_delta);
    }

    perturb_anchor(
        pt, seed, index, method, target, per_axis,
        amount, amount_x, amount_y, frequency, octaves, lacunarity,
    )
}

/// Perturb a handle (control point).
/// target: 0=Anchors, 1=Handles Only, 2=Both
#[allow(clippy::too_many_arguments)]
fn perturb_handle(
    handle: &Point,
    anchor: &Point,
    seed: u64,
    index: u64,
    method: i32,
    target: i32,
    per_axis: bool,
    preserve_smoothness: bool,
    amount: f32,
    amount_x: f32,
    amount_y: f32,
    frequency: f64,
    octaves: usize,
    lacunarity: f64,
    handle_scale: f32,
    anchor_delta: &(f32, f32),
) -> Point {
    match target {
        0 => {
            // Anchors mode: handles follow anchor delta, with optional coherent
            // length deformation controlled by handle_scale.
            // handle_scale=0 → exact follow (offset preserved)
            // handle_scale>0 → coherent length deformation
            coherent_handle_follow(handle, anchor, anchor_delta, handle_scale)
        }
        1 | 2 => {
            // Handles Only (1) or Both (2) → perturb handles independently.
            if preserve_smoothness {
                perturb_handle_smooth(
                    handle, anchor, anchor_delta, seed, index, method, amount,
                    frequency, octaves, lacunarity,
                )
            } else {
                let (dx, dy) = noise::displace(
                    handle.x, handle.y, seed, index,
                    method, per_axis, amount, amount_x, amount_y,
                    frequency, octaves, lacunarity,
                );
                Point { x: handle.x + dx, y: handle.y + dy }
            }
        }
        _ => *handle,
    }
}

/// Anchors+Coherent handle perturbation: follow the anchor (preserving tangent
/// direction), then adjust the handle length based on the projection of the anchor
/// delta onto the handle direction.
///
/// - handle_scale=0 → identical to Anchors Only (offset fully preserved)
/// - handle_scale>0 → handles aligned with the anchor motion get longer/shorter
fn coherent_handle_follow(
    handle: &Point,
    anchor: &Point, // already perturbed
    anchor_delta: &(f32, f32),
    handle_scale: f32,
) -> Point {
    // Step 1: drag handle by full anchor delta (preserving offset from anchor).
    let followed_x = handle.x + anchor_delta.0;
    let followed_y = handle.y + anchor_delta.1;

    if handle_scale.abs() < 1e-10 {
        return Point { x: followed_x, y: followed_y };
    }

    // The offset from the (perturbed) anchor to the followed handle equals the
    // original offset (handle_old − anchor_old), so direction is preserved.
    let dx = followed_x - anchor.x;
    let dy = followed_y - anchor.y;
    let len = (dx * dx + dy * dy).sqrt();

    if len < 1e-10 {
        return Point { x: followed_x, y: followed_y };
    }

    let dir_x = dx / len;
    let dir_y = dy / len;

    // Project anchor delta onto handle direction for a signed length change.
    let proj = anchor_delta.0 * dir_x + anchor_delta.1 * dir_y;
    let new_len = (len + proj * handle_scale).max(0.0);

    Point {
        x: anchor.x + dir_x * new_len,
        y: anchor.y + dir_y * new_len,
    }
}

/// Preserve-smoothness handle perturbation: only change handle length, not direction.
///
/// Direction is computed from the *original* anchor (before perturbation) so that
/// collinear handle pairs stay collinear — this is critical at closed-path seams.
#[allow(clippy::too_many_arguments)]
fn perturb_handle_smooth(
    handle: &Point,
    anchor: &Point,           // perturbed anchor
    anchor_delta: &(f32, f32),
    seed: u64,
    index: u64,
    method: i32,
    amount: f32,
    frequency: f64,
    octaves: usize,
    lacunarity: f64,
) -> Point {
    let scalar = noise::displace_scalar(
        handle.x, handle.y, seed, index,
        method, amount, frequency, octaves, lacunarity,
    );
    // Recover original anchor to get the true handle direction.
    let orig_anchor_x = anchor.x - anchor_delta.0;
    let orig_anchor_y = anchor.y - anchor_delta.1;
    let dx = handle.x - orig_anchor_x;
    let dy = handle.y - orig_anchor_y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-10 {
        return *anchor;
    }
    let dir_x = dx / len;
    let dir_y = dy / len;
    let new_len = (len + scalar).max(0.0);
    // Place relative to perturbed anchor, preserving original direction.
    Point {
        x: anchor.x + dir_x * new_len,
        y: anchor.y + dir_y * new_len,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_line_path() -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 100.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 100.0, y: 100.0 }),
            ],
            closed: false,
        }
    }

    fn make_cubic_path() -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::CubicTo {
                    ctrl1: Point { x: 33.0, y: 50.0 },
                    ctrl2: Point { x: 66.0, y: 50.0 },
                    to: Point { x: 100.0, y: 0.0 },
                },
            ],
            closed: false,
        }
    }

    #[test]
    fn deterministic_results() {
        let path = NodeData::Path(Arc::new(make_line_path()));
        let a = perturb_points(&path, 42, 0, 0, false, false, 10.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5);
        let b = perturb_points(&path, 42, 0, 0, false, false, 10.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5);
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }

    #[test]
    fn zero_amount_no_change() {
        let path = make_line_path();
        let result = perturb_path_data(
            &path, 42, 0, 0, false, false,
            0.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5,
        );
        assert_eq!(path.verbs.len(), result.verbs.len());
        // With amount=0, uniform radial gives zero displacement.
        for (orig, res) in path.verbs.iter().zip(result.verbs.iter()) {
            match (orig, res) {
                (PathVerb::MoveTo(a), PathVerb::MoveTo(b))
                | (PathVerb::LineTo(a), PathVerb::LineTo(b)) => {
                    assert!((a.x - b.x).abs() < 1e-6);
                    assert!((a.y - b.y).abs() < 1e-6);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn anchors_only_drags_handles() {
        let path = make_cubic_path();
        // handle_scale=0 → exact follow (offset preserved).
        let result = perturb_path_data(
            &path, 42, 0, 0, false, false,
            10.0, 0.0, 0.0, 1.0, 4, 2.0, 0.0,
        );
        // In anchors-only mode, handles should be dragged by the same delta as their anchor.
        // Check that ctrl1 offset from anchor is preserved.
        if let (PathVerb::CubicTo { ctrl1: orig_ctrl1, .. }, PathVerb::CubicTo { ctrl1: new_ctrl1, .. }) =
            (&path.verbs[1], &result.verbs[1])
        {
            if let (PathVerb::MoveTo(orig_anchor), PathVerb::MoveTo(new_anchor)) =
                (&path.verbs[0], &result.verbs[0])
            {
                let orig_offset_x = orig_ctrl1.x - orig_anchor.x;
                let orig_offset_y = orig_ctrl1.y - orig_anchor.y;
                let new_offset_x = new_ctrl1.x - new_anchor.x;
                let new_offset_y = new_ctrl1.y - new_anchor.y;
                assert!((orig_offset_x - new_offset_x).abs() < 1e-4,
                    "X offset changed: {} vs {}", orig_offset_x, new_offset_x);
                assert!((orig_offset_y - new_offset_y).abs() < 1e-4,
                    "Y offset changed: {} vs {}", orig_offset_y, new_offset_y);
            }
        }
    }

    #[test]
    fn handles_only_leaves_anchors_unchanged() {
        let path = make_cubic_path();
        let result = perturb_path_data(
            &path, 42, 0, 1, false, false,
            10.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5,
        );
        // Anchors should be unchanged.
        if let (PathVerb::MoveTo(a), PathVerb::MoveTo(b)) = (&path.verbs[0], &result.verbs[0]) {
            assert_eq!(a.x, b.x);
            assert_eq!(a.y, b.y);
        }
        if let (PathVerb::CubicTo { to: a, .. }, PathVerb::CubicTo { to: b, .. }) =
            (&path.verbs[1], &result.verbs[1])
        {
            assert_eq!(a.x, b.x);
            assert_eq!(a.y, b.y);
        }
    }

    #[test]
    fn noise_method_works() {
        let path = make_line_path();
        let result = perturb_path_data(
            &path, 42, 2, 0, false, false,
            10.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5,
        );
        // Just check it produced a result with the same number of verbs.
        assert_eq!(path.verbs.len(), result.verbs.len());
    }

    #[test]
    fn per_axis_mode_works() {
        let path = make_line_path();
        let result = perturb_path_data(
            &path, 42, 0, 0, true, false,
            0.0, 10.0, 5.0, 1.0, 4, 2.0, 0.5,
        );
        assert_eq!(path.verbs.len(), result.verbs.len());
    }

    #[test]
    fn perturb_point_batch_works() {
        let batch = PointBatch {
            xs: vec![0.0, 10.0, 20.0],
            ys: vec![0.0, 10.0, 20.0],
        };
        let data = NodeData::Points(Arc::new(batch));
        let result = perturb_points(&data, 42, 0, 0, false, false, 5.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5);
        if let NodeData::Points(pts) = &result {
            assert_eq!(pts.len(), 3);
        } else {
            panic!("Expected Points");
        }
    }

    #[test]
    fn preserve_smoothness_with_handles_only() {
        let path = make_cubic_path();
        let result = perturb_path_data(
            &path, 42, 0, 1, false, true,
            10.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5,
        );
        // Anchors should be unchanged (handles-only mode).
        if let (PathVerb::MoveTo(a), PathVerb::MoveTo(b)) = (&path.verbs[0], &result.verbs[0]) {
            assert_eq!(a.x, b.x);
            assert_eq!(a.y, b.y);
        }
        // Handles should have changed (amount > 0).
        if let (PathVerb::CubicTo { ctrl1: a, .. }, PathVerb::CubicTo { ctrl1: b, .. }) =
            (&path.verbs[1], &result.verbs[1])
        {
            // With preserve_smoothness, direction from anchor to handle should be the same.
            let orig_dx = a.x - 0.0; // anchor is (0,0)
            let orig_dy = a.y - 0.0;
            let new_dx = b.x - 0.0;
            let new_dy = b.y - 0.0;
            let orig_len = (orig_dx * orig_dx + orig_dy * orig_dy).sqrt();
            let new_len = (new_dx * new_dx + new_dy * new_dy).sqrt();
            if orig_len > 1e-6 && new_len > 1e-6 {
                let orig_dir = (orig_dx / orig_len, orig_dy / orig_len);
                let new_dir = (new_dx / new_len, new_dy / new_len);
                assert!((orig_dir.0 - new_dir.0).abs() < 1e-4, "Direction changed");
                assert!((orig_dir.1 - new_dir.1).abs() < 1e-4, "Direction changed");
            }
        }
    }

    #[test]
    fn coherent_handles_preserve_tangent_direction() {
        // A smooth curve: MoveTo(0,0) → CubicTo with handles collinear through anchor (100,0).
        // After Anchors mode with handle_scale>0, the handles at (100,0) must remain
        // collinear through the perturbed anchor (i.e., tangent direction preserved).
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::CubicTo {
                    ctrl1: Point { x: 33.0, y: 0.0 },  // outgoing handle from (0,0)
                    ctrl2: Point { x: 66.0, y: 0.0 },   // incoming handle to (100,0)
                    to: Point { x: 100.0, y: 0.0 },
                },
                PathVerb::CubicTo {
                    ctrl1: Point { x: 133.0, y: 0.0 },  // outgoing handle from (100,0)
                    ctrl2: Point { x: 166.0, y: 0.0 },  // incoming handle to (200,0)
                    to: Point { x: 200.0, y: 0.0 },
                },
            ],
            closed: false,
        };

        let result = perturb_path_data(
            &path, 42, 0, 0, false, false,
            20.0, 0.0, 0.0, 1.0, 4, 2.0, 0.7,
        );

        // Extract the middle anchor (100,0) and its two handles.
        let ctrl2_incoming = match &result.verbs[1] {
            PathVerb::CubicTo { ctrl2, .. } => *ctrl2,
            _ => panic!("Expected CubicTo"),
        };
        let anchor = match &result.verbs[1] {
            PathVerb::CubicTo { to, .. } => *to,
            _ => panic!("Expected CubicTo"),
        };
        let ctrl1_outgoing = match &result.verbs[2] {
            PathVerb::CubicTo { ctrl1, .. } => *ctrl1,
            _ => panic!("Expected CubicTo"),
        };

        // Check collinearity: cross product of (anchor→ctrl2) × (anchor→ctrl1) ≈ 0
        let v1 = (ctrl2_incoming.x - anchor.x, ctrl2_incoming.y - anchor.y);
        let v2 = (ctrl1_outgoing.x - anchor.x, ctrl1_outgoing.y - anchor.y);
        let cross = v1.0 * v2.1 - v1.1 * v2.0;
        assert!(
            cross.abs() < 1e-3,
            "Handles not collinear at anchor: cross product = {} (ctrl2={:?}, anchor={:?}, ctrl1={:?})",
            cross, ctrl2_incoming, anchor, ctrl1_outgoing,
        );
    }

    #[test]
    fn closed_path_no_seam() {
        // Simulate a closed path: MoveTo(P) → CubicTo → CubicTo(to=P) → Close.
        // The last "to" coincides with the initial MoveTo.
        let start = Point { x: 100.0, y: 0.0 };
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(start),
                PathVerb::CubicTo {
                    ctrl1: Point { x: 100.0, y: 55.0 },
                    ctrl2: Point { x: 55.0, y: 100.0 },
                    to: Point { x: 0.0, y: 100.0 },
                },
                PathVerb::CubicTo {
                    ctrl1: Point { x: -55.0, y: 100.0 },
                    ctrl2: Point { x: 100.0, y: -55.0 },
                    to: start, // coincides with MoveTo
                },
                PathVerb::Close,
            ],
            closed: true,
        };

        // Perturb with Uniform method, Anchors target with handle_scale.
        let result = perturb_path_data(
            &path, 42, 0, 0, false, false,
            20.0, 0.0, 0.0, 1.0, 4, 2.0, 0.5,
        );

        // The perturbed MoveTo and the last CubicTo's "to" must match exactly.
        let move_pt = match &result.verbs[0] {
            PathVerb::MoveTo(p) => *p,
            _ => panic!("Expected MoveTo"),
        };
        let close_pt = match &result.verbs[2] {
            PathVerb::CubicTo { to, .. } => *to,
            _ => panic!("Expected CubicTo"),
        };

        assert!(
            (move_pt.x - close_pt.x).abs() < 1e-6 && (move_pt.y - close_pt.y).abs() < 1e-6,
            "Seam detected: MoveTo=({}, {}) vs last to=({}, {})",
            move_pt.x, move_pt.y, close_pt.x, close_pt.y,
        );
    }
}
