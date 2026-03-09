use std::sync::Arc;

use glam::Vec2;

use vector_flow_core::types::{NodeData, PathData, PathVerb, Point, PointBatch};

/// Approximate a circular arc with cubic Bézier curves.
/// Each segment covers at most 90° for sub-pixel accuracy.
/// Uses the standard tangent-length formula: `t = (4/3) * tan(θ/4)`.
fn arc_beziers(path: &mut PathData, cx: f32, cy: f32, radius: f32, start_rad: f32, sweep_rad: f32) {
    if radius <= 0.0 || sweep_rad.abs() < 1e-9 {
        return;
    }

    // Split into segments of at most 90°
    let max_segment = std::f32::consts::FRAC_PI_2; // 90°
    let num_segments = ((sweep_rad.abs() / max_segment).ceil() as usize).max(1);
    let seg_angle = sweep_rad / num_segments as f32;

    let mut angle = start_rad;
    for i in 0..num_segments {
        let quarter = seg_angle * 0.25;
        let t = (4.0 / 3.0) * quarter.tan();

        let cos_a = angle.cos();
        let sin_a = angle.sin();
        let cos_b = (angle + seg_angle).cos();
        let sin_b = (angle + seg_angle).sin();

        let p1 = Point {
            x: cx + radius * cos_a,
            y: cy + radius * sin_a,
        };
        let ctrl1 = Point {
            x: cx + radius * (cos_a - t * sin_a),
            y: cy + radius * (sin_a + t * cos_a),
        };
        let ctrl2 = Point {
            x: cx + radius * (cos_b + t * sin_b),
            y: cy + radius * (sin_b - t * cos_b),
        };
        let p2 = Point {
            x: cx + radius * cos_b,
            y: cy + radius * sin_b,
        };

        if i == 0 {
            path.verbs.push(PathVerb::MoveTo(p1));
        }
        path.verbs.push(PathVerb::CubicTo {
            ctrl1,
            ctrl2,
            to: p2,
        });

        angle += seg_angle;
    }
}

/// Arc / wedge / donut-wedge generator.
///
/// - `close=false, inner_radius=0`: open arc stroke
/// - `close=true,  inner_radius=0`: wedge (pie slice)
/// - `close=true,  inner_radius>0`: donut wedge (annular sector)
/// - `close=false, inner_radius>0`: two concentric open arcs
pub fn arc(
    outer_radius: f64,
    inner_radius: f64,
    start_angle: f64,
    sweep_angle: f64,
    close: bool,
    center: Vec2,
) -> NodeData {
    let outer_r = outer_radius as f32;
    let inner_r = (inner_radius as f32).max(0.0);
    let cx = center.x;
    let cy = center.y;
    let start_rad = (start_angle as f32).to_radians();
    let sweep_rad = (sweep_angle as f32).to_radians();

    let mut path = PathData::new();

    if inner_r > 0.0 && close {
        // Donut wedge: outer arc forward, line to inner arc start, inner arc reversed, close
        arc_beziers(&mut path, cx, cy, outer_r, start_rad, sweep_rad);
        // Line to inner arc end (which is where the reversed inner arc starts)
        let inner_end = Point {
            x: cx + inner_r * (start_rad + sweep_rad).cos(),
            y: cy + inner_r * (start_rad + sweep_rad).sin(),
        };
        path.verbs.push(PathVerb::LineTo(inner_end));
        // Inner arc reversed
        arc_beziers(&mut path, cx, cy, inner_r, start_rad + sweep_rad, -sweep_rad);
        // Remove the MoveTo that arc_beziers prepends for the inner arc — replace with continuation
        // Find the second MoveTo and remove it
        let mut move_count = 0;
        let mut remove_idx = None;
        for (i, v) in path.verbs.iter().enumerate() {
            if matches!(v, PathVerb::MoveTo(_)) {
                move_count += 1;
                if move_count == 2 {
                    remove_idx = Some(i);
                    break;
                }
            }
        }
        if let Some(idx) = remove_idx {
            path.verbs.remove(idx);
        }
        path.verbs.push(PathVerb::Close);
        path.closed = true;
    } else if inner_r > 0.0 {
        // Two open concentric arcs (niche case)
        arc_beziers(&mut path, cx, cy, outer_r, start_rad, sweep_rad);
        arc_beziers(&mut path, cx, cy, inner_r, start_rad, sweep_rad);
    } else if close {
        // Wedge: line from center to arc start, arc, line back to center, close
        let arc_start = Point {
            x: cx + outer_r * start_rad.cos(),
            y: cy + outer_r * start_rad.sin(),
        };
        // Check if it's a full circle — no need for wedge lines
        if sweep_rad.abs() >= std::f32::consts::TAU - 1e-6 {
            arc_beziers(&mut path, cx, cy, outer_r, start_rad, sweep_rad);
        } else {
            path.verbs.push(PathVerb::MoveTo(Point { x: cx, y: cy }));
            path.verbs.push(PathVerb::LineTo(arc_start));
            arc_beziers(&mut path, cx, cy, outer_r, start_rad, sweep_rad);
            // Remove the MoveTo that arc_beziers prepends — we already positioned with LineTo
            let mut move_count = 0;
            let mut remove_idx = None;
            for (i, v) in path.verbs.iter().enumerate() {
                if matches!(v, PathVerb::MoveTo(_)) {
                    move_count += 1;
                    if move_count == 2 {
                        remove_idx = Some(i);
                        break;
                    }
                }
            }
            if let Some(idx) = remove_idx {
                path.verbs.remove(idx);
            }
        }
        path.verbs.push(PathVerb::Close);
        path.closed = true;
    } else {
        // Open arc
        arc_beziers(&mut path, cx, cy, outer_r, start_rad, sweep_rad);
    }

    NodeData::Path(Arc::new(path))
}

/// Regular polygon with N sides, radius, center.
pub fn regular_polygon(sides: i64, radius: f64, center: Vec2) -> NodeData {
    let n = sides.max(3) as usize;
    let r = radius as f32;
    let mut path = PathData::new();

    for i in 0..n {
        let angle = std::f32::consts::TAU * (i as f32) / (n as f32);
        let pt = Point {
            x: center.x + r * angle.cos(),
            y: center.y + r * angle.sin(),
        };
        if i == 0 {
            path.verbs.push(PathVerb::MoveTo(pt));
        } else {
            path.verbs.push(PathVerb::LineTo(pt));
        }
    }
    path.verbs.push(PathVerb::Close);
    path.closed = true;

    NodeData::Path(Arc::new(path))
}

/// Circle using 4 cubic bezier arcs (one per quadrant).
/// The kappa constant 4/3 * (√2 - 1) ≈ 0.5522847 gives a near-perfect circle.
pub fn circle(radius: f64, center: Vec2) -> NodeData {
    let r = radius as f32;
    let cx = center.x;
    let cy = center.y;
    let k = r * 0.5522847;

    let mut path = PathData::new();
    // Start at rightmost point, go counter-clockwise in screen coords.
    path.verbs.push(PathVerb::MoveTo(Point { x: cx + r, y: cy }));
    // Right → Bottom
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx + r, y: cy + k },
        ctrl2: Point { x: cx + k, y: cy + r },
        to: Point { x: cx, y: cy + r },
    });
    // Bottom → Left
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx - k, y: cy + r },
        ctrl2: Point { x: cx - r, y: cy + k },
        to: Point { x: cx - r, y: cy },
    });
    // Left → Top
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx - r, y: cy - k },
        ctrl2: Point { x: cx - k, y: cy - r },
        to: Point { x: cx, y: cy - r },
    });
    // Top → Right
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx + k, y: cy - r },
        ctrl2: Point { x: cx + r, y: cy - k },
        to: Point { x: cx + r, y: cy },
    });
    path.verbs.push(PathVerb::Close);
    path.closed = true;

    NodeData::Path(Arc::new(path))
}

/// Axis-aligned rectangle centered at `center`.
pub fn rectangle(width: f64, height: f64, center: Vec2) -> NodeData {
    let hw = (width as f32) * 0.5;
    let hh = (height as f32) * 0.5;
    let cx = center.x;
    let cy = center.y;

    let mut path = PathData::new();
    path.verbs.push(PathVerb::MoveTo(Point { x: cx - hw, y: cy - hh }));
    path.verbs.push(PathVerb::LineTo(Point { x: cx + hw, y: cy - hh }));
    path.verbs.push(PathVerb::LineTo(Point { x: cx + hw, y: cy + hh }));
    path.verbs.push(PathVerb::LineTo(Point { x: cx - hw, y: cy + hh }));
    path.verbs.push(PathVerb::Close);
    path.closed = true;

    NodeData::Path(Arc::new(path))
}

/// Line segment from `from` to `to`.
pub fn line(from: Vec2, to: Vec2) -> NodeData {
    let mut path = PathData::new();
    path.verbs.push(PathVerb::MoveTo(Point { x: from.x, y: from.y }));
    path.verbs.push(PathVerb::LineTo(Point { x: to.x, y: to.y }));

    NodeData::Path(Arc::new(path))
}

/// Grid of points with `cols` columns, `rows` rows, `spacing` between them.
/// Grid is centered at origin.
pub fn point_grid(cols: i64, rows: i64, spacing: f64) -> NodeData {
    let c = cols.max(1) as usize;
    let r = rows.max(1) as usize;
    let sp = spacing as f32;

    let total = c * r;
    let mut xs = Vec::with_capacity(total);
    let mut ys = Vec::with_capacity(total);

    let ox = (c as f32 - 1.0) * sp * 0.5;
    let oy = (r as f32 - 1.0) * sp * 0.5;

    for row in 0..r {
        for col in 0..c {
            xs.push(col as f32 * sp - ox);
            ys.push(oy - row as f32 * sp);
        }
    }

    NodeData::Points(Arc::new(PointBatch { xs, ys }))
}

/// Scatter N points randomly in a width x height region centered at origin.
/// Uses a deterministic hash-based PRNG seeded by `seed`.
pub fn scatter_points(count: i64, width: f64, height: f64, seed: i64) -> NodeData {
    let n = count.max(0) as usize;
    let w = width as f32;
    let h = height as f32;

    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);

    for i in 0..n {
        // Simple hash PRNG (splitmix64-inspired)
        let mut s = (seed as u64).wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(i as u64);
        s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
        s ^= s >> 31;
        let fx = (s & 0xFFFFFFFF) as f32 / u32::MAX as f32;

        s = s.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1);
        s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
        s ^= s >> 31;
        let fy = (s & 0xFFFFFFFF) as f32 / u32::MAX as f32;

        xs.push(fx * w - w * 0.5);
        ys.push(fy * h - h * 0.5);
    }

    NodeData::Points(Arc::new(PointBatch { xs, ys }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_open_stroke() {
        // Open 90° arc: MoveTo + 1 CubicTo, not closed
        let data = arc(100.0, 0.0, 0.0, 90.0, false, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            assert!(matches!(p.verbs[0], PathVerb::MoveTo(_)));
            assert!(matches!(p.verbs[1], PathVerb::CubicTo { .. }));
            assert!(!p.closed);
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn arc_wedge() {
        // Closed 90° wedge: MoveTo(center) + LineTo(arc start) + CubicTo + Close
        let data = arc(100.0, 0.0, 0.0, 90.0, true, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            assert!(p.closed);
            // First verb: MoveTo center
            if let PathVerb::MoveTo(pt) = &p.verbs[0] {
                assert!((pt.x).abs() < 1e-3);
                assert!((pt.y).abs() < 1e-3);
            } else {
                panic!("expected MoveTo center");
            }
            // Should end with Close
            assert!(matches!(p.verbs.last().unwrap(), PathVerb::Close));
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn arc_donut_wedge() {
        // Donut wedge with inner_radius=50, outer_radius=100, 90° sweep
        let data = arc(100.0, 50.0, 0.0, 90.0, true, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            assert!(p.closed);
            // Should have outer arc + line + inner arc (reversed) + close
            // Only one MoveTo (the second was removed)
            let move_count = p.verbs.iter().filter(|v| matches!(v, PathVerb::MoveTo(_))).count();
            assert_eq!(move_count, 1);
            assert!(matches!(p.verbs.last().unwrap(), PathVerb::Close));
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn arc_full_circle_wedge() {
        // 360° sweep with close=true should be a full circle (no wedge lines to center)
        let data = arc(100.0, 0.0, 0.0, 360.0, true, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            assert!(p.closed);
            // Should not have any LineTo (no radial lines for a full circle)
            let line_count = p.verbs.iter().filter(|v| matches!(v, PathVerb::LineTo(_))).count();
            assert_eq!(line_count, 0);
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn arc_180_degree_needs_two_segments() {
        // 180° arc should split into 2 cubic segments (each <= 90°)
        let data = arc(100.0, 0.0, 0.0, 180.0, false, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            let cubic_count = p.verbs.iter().filter(|v| matches!(v, PathVerb::CubicTo { .. })).count();
            assert_eq!(cubic_count, 2);
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn arc_with_center_offset() {
        // Arc centered at (50, 50), check first point is offset
        let data = arc(100.0, 0.0, 0.0, 90.0, false, Vec2::new(50.0, 50.0));
        if let NodeData::Path(p) = data {
            if let PathVerb::MoveTo(pt) = &p.verbs[0] {
                assert!((pt.x - 150.0).abs() < 1e-3); // cx + r * cos(0) = 50 + 100
                assert!((pt.y - 50.0).abs() < 1e-3);  // cy + r * sin(0) = 50 + 0
            } else {
                panic!("expected MoveTo");
            }
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn circle_bezier_arcs() {
        let data = circle(100.0, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            // 1 MoveTo + 4 CubicTo + 1 Close = 6 verbs
            assert_eq!(p.verbs.len(), 6);
            assert!(matches!(p.verbs[0], PathVerb::MoveTo(_)));
            for i in 1..5 {
                assert!(matches!(p.verbs[i], PathVerb::CubicTo { .. }));
            }
            assert!(matches!(p.verbs[5], PathVerb::Close));
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn rectangle_has_four_corners() {
        let data = rectangle(200.0, 100.0, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            let non_close = p.verbs.iter().filter(|v| !matches!(v, PathVerb::Close)).count();
            assert_eq!(non_close, 4); // 1 MoveTo + 3 LineTo
            assert!(p.closed);
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn polygon_sides_match() {
        let data = regular_polygon(5, 50.0, Vec2::ZERO);
        if let NodeData::Path(p) = data {
            let non_close = p.verbs.iter().filter(|v| !matches!(v, PathVerb::Close)).count();
            assert_eq!(non_close, 5);
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn point_grid_count() {
        let data = point_grid(4, 3, 10.0);
        if let NodeData::Points(pts) = data {
            assert_eq!(pts.len(), 12);
        } else {
            panic!("expected Points");
        }
    }

    #[test]
    fn scatter_deterministic() {
        let a = scatter_points(50, 100.0, 100.0, 42);
        let b = scatter_points(50, 100.0, 100.0, 42);
        if let (NodeData::Points(pa), NodeData::Points(pb)) = (a, b) {
            assert_eq!(pa.xs, pb.xs);
            assert_eq!(pa.ys, pb.ys);
        } else {
            panic!("expected Points");
        }
    }
}
