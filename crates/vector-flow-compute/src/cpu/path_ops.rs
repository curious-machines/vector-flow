use std::sync::Arc;

use lyon::math::point as lyon_point;
use lyon::path::iterator::PathIterator;
use lyon::path::Path as LyonPath;

use vector_flow_core::types::{NodeData, PathData, PathVerb, Point, PointBatch};

/// Reverse the direction of a path.
pub fn path_reverse(path: &PathData) -> PathData {
    if path.verbs.is_empty() {
        return path.clone();
    }

    // Collect all points in order, then rebuild reversed.
    let mut points: Vec<Point> = Vec::new();
    for v in &path.verbs {
        match *v {
            PathVerb::MoveTo(p) | PathVerb::LineTo(p) => points.push(p),
            PathVerb::QuadTo { to, .. } => points.push(to),
            PathVerb::CubicTo { to, .. } => points.push(to),
            PathVerb::Close => {}
        }
    }

    points.reverse();

    let mut verbs = Vec::with_capacity(path.verbs.len());
    for (i, &pt) in points.iter().enumerate() {
        if i == 0 {
            verbs.push(PathVerb::MoveTo(pt));
        } else {
            verbs.push(PathVerb::LineTo(pt));
        }
    }
    if path.closed {
        verbs.push(PathVerb::Close);
    }

    PathData {
        verbs,
        closed: path.closed,
    }
}

/// Subdivide each line segment at its midpoint, `levels` times.
pub fn path_subdivide(path: &PathData, levels: i64) -> PathData {
    let mut current = path.clone();
    let n = levels.max(0) as usize;
    for _ in 0..n {
        current = subdivide_once(&current);
    }
    current
}

fn subdivide_once(path: &PathData) -> PathData {
    let mut verbs = Vec::new();
    let mut last = Point { x: 0.0, y: 0.0 };

    for v in &path.verbs {
        match *v {
            PathVerb::MoveTo(p) => {
                verbs.push(PathVerb::MoveTo(p));
                last = p;
            }
            PathVerb::LineTo(p) => {
                let mid = Point {
                    x: (last.x + p.x) * 0.5,
                    y: (last.y + p.y) * 0.5,
                };
                verbs.push(PathVerb::LineTo(mid));
                verbs.push(PathVerb::LineTo(p));
                last = p;
            }
            PathVerb::QuadTo { ctrl, to } => {
                // De Casteljau split at t=0.5
                let m0 = midpoint(last, ctrl);
                let m1 = midpoint(ctrl, to);
                let mid = midpoint(m0, m1);
                verbs.push(PathVerb::QuadTo { ctrl: m0, to: mid });
                verbs.push(PathVerb::QuadTo { ctrl: m1, to });
                last = to;
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                // De Casteljau split at t=0.5
                let m01 = midpoint(last, ctrl1);
                let m12 = midpoint(ctrl1, ctrl2);
                let m23 = midpoint(ctrl2, to);
                let m012 = midpoint(m01, m12);
                let m123 = midpoint(m12, m23);
                let mid = midpoint(m012, m123);
                verbs.push(PathVerb::CubicTo { ctrl1: m01, ctrl2: m012, to: mid });
                verbs.push(PathVerb::CubicTo { ctrl1: m123, ctrl2: m23, to });
                last = to;
            }
            PathVerb::Close => {
                verbs.push(PathVerb::Close);
            }
        }
    }

    PathData {
        verbs,
        closed: path.closed,
    }
}

fn midpoint(a: Point, b: Point) -> Point {
    Point {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
    }
}

/// Resample a path into `count` evenly-spaced points along its length.
/// For closed paths, points are distributed around the loop (no overlap).
/// For open paths, points span from start to end inclusive.
pub fn resample_path(path: &PathData, count: i64) -> NodeData {
    let n = count.max(2) as usize;

    // Collect line segments (subdividing curves for accuracy).
    let segments = flatten_to_segments(path);
    if segments.is_empty() {
        return NodeData::Points(Arc::new(PointBatch::new()));
    }

    // Compute cumulative lengths
    let mut lengths = Vec::with_capacity(segments.len());
    let mut total = 0.0f32;
    for &(a, b) in &segments {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        total += len;
        lengths.push(total);
    }

    if total < 1e-10 {
        // Degenerate path, return first point repeated
        let p = segments[0].0;
        return NodeData::Points(Arc::new(PointBatch {
            xs: vec![p.x; n],
            ys: vec![p.y; n],
        }));
    }

    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);

    // For closed paths, divide by n (points wrap around, no overlap).
    // For open paths, divide by n-1 (endpoints are start and end).
    let divisor = if path.closed { n as f32 } else { (n - 1) as f32 };

    for i in 0..n {
        let t = if n > 1 { i as f32 / divisor } else { 0.0 };
        let target_len = t * total;

        // Find the segment containing this distance
        let seg_idx = lengths
            .iter()
            .position(|&l| l >= target_len)
            .unwrap_or(segments.len() - 1);

        let seg_start_len = if seg_idx > 0 { lengths[seg_idx - 1] } else { 0.0 };
        let seg_len = lengths[seg_idx] - seg_start_len;
        let local_t = if seg_len > 1e-10 {
            (target_len - seg_start_len) / seg_len
        } else {
            0.0
        };

        let (a, b) = segments[seg_idx];
        xs.push(a.x + (b.x - a.x) * local_t);
        ys.push(a.y + (b.y - a.y) * local_t);
    }

    NodeData::Points(Arc::new(PointBatch { xs, ys }))
}

/// Resample a path into `count` evenly-spaced points, returning both
/// the point positions and tangent angles (in degrees) at each sample.
pub fn resample_with_tangents(path: &PathData, count: i64) -> (PointBatch, Vec<f64>) {
    let n = count.max(2) as usize;

    let segments = flatten_to_segments(path);
    if segments.is_empty() {
        return (PointBatch::new(), vec![0.0; n]);
    }

    let mut lengths = Vec::with_capacity(segments.len());
    let mut total = 0.0f32;
    for &(a, b) in &segments {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        total += (dx * dx + dy * dy).sqrt();
        lengths.push(total);
    }

    if total < 1e-10 {
        let p = segments[0].0;
        return (
            PointBatch { xs: vec![p.x; n], ys: vec![p.y; n] },
            vec![0.0; n],
        );
    }

    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);

    let divisor = if path.closed { n as f32 } else { (n - 1) as f32 };

    for i in 0..n {
        let t = if n > 1 { i as f32 / divisor } else { 0.0 };
        let target_len = t * total;

        let seg_idx = lengths
            .iter()
            .position(|&l| l >= target_len)
            .unwrap_or(segments.len() - 1);

        let seg_start_len = if seg_idx > 0 { lengths[seg_idx - 1] } else { 0.0 };
        let seg_len = lengths[seg_idx] - seg_start_len;
        let local_t = if seg_len > 1e-10 {
            (target_len - seg_start_len) / seg_len
        } else {
            0.0
        };

        let (a, b) = segments[seg_idx];
        xs.push(a.x + (b.x - a.x) * local_t);
        ys.push(a.y + (b.y - a.y) * local_t);
    }

    // Compute tangent angles from adjacent sample points (central differences).
    // This gives much more accurate tangents on curves than using the underlying
    // polygon segment direction.
    let mut angles = Vec::with_capacity(n);
    for i in 0..n {
        let (dx, dy) = if path.closed {
            let prev = if i == 0 { n - 1 } else { i - 1 };
            let next = if i == n - 1 { 0 } else { i + 1 };
            (xs[next] - xs[prev], ys[next] - ys[prev])
        } else if i == 0 {
            (xs[1] - xs[0], ys[1] - ys[0])
        } else if i == n - 1 {
            (xs[n - 1] - xs[n - 2], ys[n - 1] - ys[n - 2])
        } else {
            (xs[i + 1] - xs[i - 1], ys[i + 1] - ys[i - 1])
        };
        angles.push(dy.atan2(dx).to_degrees() as f64);
    }

    (PointBatch { xs, ys }, angles)
}

/// Tolerance for lyon's adaptive curve flattening.
const FLATTEN_TOLERANCE: f32 = 0.5;

/// Convert our PathData to a lyon Path.
pub(crate) fn build_lyon_path(path: &PathData) -> LyonPath {
    let mut builder = LyonPath::builder();
    let mut in_subpath = false;
    for verb in &path.verbs {
        match *verb {
            PathVerb::MoveTo(p) => {
                if in_subpath {
                    builder.end(false);
                }
                builder.begin(lyon_point(p.x, p.y));
                in_subpath = true;
            }
            PathVerb::LineTo(p) => {
                if !in_subpath {
                    builder.begin(lyon_point(p.x, p.y));
                    in_subpath = true;
                } else {
                    builder.line_to(lyon_point(p.x, p.y));
                }
            }
            PathVerb::QuadTo { ctrl, to } => {
                if !in_subpath {
                    builder.begin(lyon_point(ctrl.x, ctrl.y));
                    in_subpath = true;
                }
                builder.quadratic_bezier_to(lyon_point(ctrl.x, ctrl.y), lyon_point(to.x, to.y));
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                if !in_subpath {
                    builder.begin(lyon_point(ctrl1.x, ctrl1.y));
                    in_subpath = true;
                }
                builder.cubic_bezier_to(
                    lyon_point(ctrl1.x, ctrl1.y),
                    lyon_point(ctrl2.x, ctrl2.y),
                    lyon_point(to.x, to.y),
                );
            }
            PathVerb::Close => {
                if in_subpath {
                    builder.close();
                    in_subpath = false;
                }
            }
        }
    }
    if in_subpath {
        builder.end(false);
    }
    builder.build()
}

/// Flatten path to line segments using lyon's adaptive subdivision.
fn flatten_to_segments(path: &PathData) -> Vec<(Point, Point)> {
    let lyon_path = build_lyon_path(path);
    let mut segs = Vec::new();
    let mut last = Point { x: 0.0, y: 0.0 };

    for evt in lyon_path.iter().flattened(FLATTEN_TOLERANCE) {
        use lyon::path::Event;
        match evt {
            Event::Begin { at } => {
                last = Point { x: at.x, y: at.y };
            }
            Event::Line { from: _, to } => {
                let to_pt = Point { x: to.x, y: to.y };
                segs.push((last, to_pt));
                last = to_pt;
            }
            Event::End { last: end, first, close } => {
                if close {
                    let end_pt = Point { x: end.x, y: end.y };
                    let first_pt = Point { x: first.x, y: first.y };
                    if (end_pt.x - first_pt.x).abs() > 1e-10
                        || (end_pt.y - first_pt.y).abs() > 1e-10
                    {
                        segs.push((end_pt, first_pt));
                    }
                }
            }
            _ => {}
        }
    }
    segs
}

/// Approximate path offset: move each vertex along its estimated normal.
pub fn path_offset(path: &PathData, distance: f64) -> PathData {
    let dist = distance as f32;

    // Collect points, compute normals, offset
    let mut points: Vec<Point> = Vec::new();
    for v in &path.verbs {
        match *v {
            PathVerb::MoveTo(p) | PathVerb::LineTo(p) => points.push(p),
            PathVerb::QuadTo { to, .. } | PathVerb::CubicTo { to, .. } => points.push(to),
            PathVerb::Close => {}
        }
    }

    if points.len() < 2 {
        return path.clone();
    }

    let mut offset_points = Vec::with_capacity(points.len());
    for i in 0..points.len() {
        let prev = if i > 0 { points[i - 1] } else if path.closed { points[points.len() - 1] } else { points[i] };
        let next = if i + 1 < points.len() { points[i + 1] } else if path.closed { points[0] } else { points[i] };

        let dx = next.x - prev.x;
        let dy = next.y - prev.y;
        let len = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = if len > 1e-10 { (-dy / len, dx / len) } else { (0.0, 0.0) };

        offset_points.push(Point {
            x: points[i].x + nx * dist,
            y: points[i].y + ny * dist,
        });
    }

    let mut verbs = Vec::with_capacity(path.verbs.len());
    let mut pt_idx = 0;
    for v in &path.verbs {
        match v {
            PathVerb::MoveTo(_) => {
                verbs.push(PathVerb::MoveTo(offset_points[pt_idx]));
                pt_idx += 1;
            }
            PathVerb::LineTo(_) => {
                verbs.push(PathVerb::LineTo(offset_points[pt_idx]));
                pt_idx += 1;
            }
            PathVerb::QuadTo { .. } | PathVerb::CubicTo { .. } => {
                // Approximate curves as lines after offset
                verbs.push(PathVerb::LineTo(offset_points[pt_idx]));
                pt_idx += 1;
            }
            PathVerb::Close => {
                verbs.push(PathVerb::Close);
            }
        }
    }

    PathData {
        verbs,
        closed: path.closed,
    }
}

/// Path union — combine multiple paths into a single compound path.
/// Each input path becomes a subpath in the result.
/// Note: PathUnion node now uses shape-aware merging in cpu/mod.rs,
/// but this is kept for potential future use by boolean path ops.
#[allow(dead_code)]
pub fn path_union(paths: &[&PathData]) -> PathData {
    let total_verbs: usize = paths.iter().map(|p| p.verbs.len()).sum();
    let mut verbs = Vec::with_capacity(total_verbs);
    let mut closed = false;
    for p in paths {
        verbs.extend_from_slice(&p.verbs);
        closed = closed || p.closed;
    }
    PathData { verbs, closed }
}

/// Stub: path intersection — returns first path.
pub fn path_intersect(a: &PathData, _b: &PathData) -> PathData {
    log::warn!("PathIntersect is stubbed — returning first path");
    a.clone()
}

/// Stub: path difference — returns first path.
pub fn path_difference(a: &PathData, _b: &PathData) -> PathData {
    log::warn!("PathDifference is stubbed — returning first path");
    a.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_triangle() -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 5.0, y: 10.0 }),
                PathVerb::Close,
            ],
            closed: true,
        }
    }

    #[test]
    fn reverse_reverses() {
        let tri = make_triangle();
        let rev = path_reverse(&tri);
        // First point of reversed should be last point of original (5,10)
        match rev.verbs[0] {
            PathVerb::MoveTo(p) => {
                assert!((p.x - 5.0).abs() < 1e-5);
                assert!((p.y - 10.0).abs() < 1e-5);
            }
            _ => panic!("expected MoveTo"),
        }
    }

    #[test]
    fn subdivide_doubles_segment_count() {
        let line_path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
            ],
            closed: false,
        };
        // Original: 1 segment (MoveTo + LineTo)
        let sub = path_subdivide(&line_path, 1);
        // After 1 subdivision: MoveTo + 2 LineTo = 3 verbs
        let line_count = sub.verbs.iter().filter(|v| matches!(v, PathVerb::LineTo(_))).count();
        assert_eq!(line_count, 2);
    }

    #[test]
    fn resample_returns_correct_count() {
        let tri = make_triangle();
        let result = resample_path(&tri, 10);
        if let NodeData::Points(pts) = result {
            assert_eq!(pts.len(), 10);
        } else {
            panic!("expected Points");
        }
    }
}
