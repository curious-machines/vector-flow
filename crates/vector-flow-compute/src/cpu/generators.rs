use std::sync::Arc;

use glam::Vec2;

use vector_flow_core::types::{NodeData, PathData, PathVerb, Point, PointBatch};

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
            ys.push(row as f32 * sp - oy);
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
