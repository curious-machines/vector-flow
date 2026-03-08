use std::sync::Arc;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
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
pub fn resample_path(path: &PathData, count: i64, tolerance: f32) -> NodeData {
    let n = count.max(2) as usize;

    // Collect line segments (subdividing curves for accuracy).
    let segments = flatten_to_segments(path, tolerance);
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
pub fn resample_with_tangents(path: &PathData, count: i64, tolerance: f32) -> (PointBatch, Vec<f64>) {
    let n = count.max(2) as usize;

    let segments = flatten_to_segments(path, tolerance);
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

/// Default tolerance for lyon's adaptive curve flattening.
pub const DEFAULT_FLATTEN_TOLERANCE: f32 = 0.5;

/// Minimum tolerance to prevent degenerate flattening (e.g. from missing ports).
const MIN_TOLERANCE: f32 = 0.001;

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
fn flatten_to_segments(path: &PathData, tolerance: f32) -> Vec<(Point, Point)> {
    let tol = tolerance.max(MIN_TOLERANCE);
    let lyon_path = build_lyon_path(path);
    let mut segs = Vec::new();
    let mut last = Point { x: 0.0, y: 0.0 };

    for evt in lyon_path.iter().flattened(tol) {
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

///// Offset a path by `distance`. Positive expands outward (for CCW winding),
/// negative contracts inward. Curves are flattened first, then each edge is
/// shifted by `distance` along its outward normal and adjacent offset edges
/// are intersected (miter join) to find the correct corner points.
pub fn path_offset(path: &PathData, distance: f64, tolerance: f32) -> PathData {
    let dist = distance as f32;
    let tol = tolerance.max(MIN_TOLERANCE);

    if dist.abs() < 1e-10 {
        return path.clone();
    }

    // Flatten curves into line segments per contour.
    let contours = flatten_to_contours(path, tol);
    if contours.is_empty() {
        return path.clone();
    }

    let mut verbs = Vec::new();
    let mut any_output = false;

    for pts in &contours {
        let is_closed = pts.len() >= 3
            && (pts.first().unwrap().x - pts.last().unwrap().x).abs() < 1e-4
            && (pts.first().unwrap().y - pts.last().unwrap().y).abs() < 1e-4;

        // Deduplicate coincident start/end for closed paths.
        let points: &[Point] = if is_closed && pts.len() > 1 { &pts[..pts.len() - 1] } else { pts };
        let n = points.len();
        if n < 2 {
            continue;
        }

        // For closed paths, detect winding via signed area (shoelace formula).
        // Positive signed area = CCW, negative = CW.
        // Left-hand normal points outward for CCW paths, so for CW paths we
        // negate the distance to ensure positive distance always expands.
        let effective_dist = if is_closed {
            let mut signed_area = 0.0f32;
            for i in 0..n {
                let j = (i + 1) % n;
                signed_area += points[i].x * points[j].y - points[j].x * points[i].y;
            }
            if signed_area > 0.0 { -dist } else { dist }
        } else {
            dist
        };

        // Compute offset edge lines. Each edge i is from points[i] to points[i+1].
        // The offset edge is shifted along its left-hand normal.
        let edge_count = if is_closed { n } else { n - 1 };
        // Each offset edge stored as (point_on_line, direction).
        let mut offset_edges: Vec<(Point, Point)> = Vec::with_capacity(edge_count);
        for i in 0..edge_count {
            let j = (i + 1) % n;
            let dx = points[j].x - points[i].x;
            let dy = points[j].y - points[i].y;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-10 {
                // Degenerate edge — use arbitrary normal.
                offset_edges.push((points[i], Point { x: 1.0, y: 0.0 }));
                continue;
            }
            // Left-hand normal: (-dy, dx) / len
            let nx = -dy / len;
            let ny = dx / len;
            let shifted = Point {
                x: points[i].x + nx * effective_dist,
                y: points[i].y + ny * effective_dist,
            };
            let dir = Point { x: dx, y: dy };
            offset_edges.push((shifted, dir));
        }

        // Compute offset vertices by intersecting adjacent offset edges.
        let mut offset_pts: Vec<Point> = Vec::with_capacity(n);
        if is_closed {
            for i in 0..n {
                let prev = if i == 0 { n - 1 } else { i - 1 };
                offset_pts.push(intersect_lines(&offset_edges[prev], &offset_edges[i]));
            }
        } else {
            // First point: project onto first offset edge at t=0.
            offset_pts.push(offset_edges[0].0);
            for i in 1..n - 1 {
                offset_pts.push(intersect_lines(&offset_edges[i - 1], &offset_edges[i]));
            }
            // Last point: project onto last offset edge at t=1.
            let last_edge = &offset_edges[edge_count - 1];
            offset_pts.push(Point {
                x: last_edge.0.x + last_edge.1.x,
                y: last_edge.0.y + last_edge.1.y,
            });
        }

        // Build output verbs.
        verbs.push(PathVerb::MoveTo(offset_pts[0]));
        for p in &offset_pts[1..] {
            verbs.push(PathVerb::LineTo(*p));
        }
        if is_closed {
            verbs.push(PathVerb::Close);
        }
        any_output = true;
    }

    if !any_output {
        return path.clone();
    }

    PathData {
        verbs,
        closed: path.closed,
    }
}

/// Intersect two offset edge lines. Falls back to bevel if the edges are
/// (nearly) parallel or the miter ratio exceeds the limit (very sharp angles).
fn intersect_lines(
    (p1, d1): &(Point, Point),
    (p2, d2): &(Point, Point),
) -> Point {
    // Line 1: p1 + t * d1, Line 2: p2 + s * d2
    let cross = d1.x * d2.y - d1.y * d2.x;

    // Normalize the cross product by edge lengths to get sin(angle).
    let len1 = (d1.x * d1.x + d1.y * d1.y).sqrt();
    let len2 = (d2.x * d2.x + d2.y * d2.y).sqrt();
    let len_product = len1 * len2;
    if len_product < 1e-10 {
        return *p2;
    }
    let sin_angle = cross / len_product;

    // Miter ratio = 1/|sin(angle)|. Limit of 4 (matches SVG default).
    // For angles where the miter would spike too far, fall back to bevel.
    if sin_angle.abs() < 0.25 {
        // sin < 0.25 means miter ratio > 4 — bevel fallback.
        return *p2;
    }

    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let t = (dx * d2.y - dy * d2.x) / cross;

    Point {
        x: p1.x + t * d1.x,
        y: p1.y + t * d1.y,
    }
}

/// Flatten a PathData into contours of Points (preserving separate sub-paths).
fn flatten_to_contours(path: &PathData, tolerance: f32) -> Vec<Vec<Point>> {
    let lyon_path = build_lyon_path(path);
    let mut contours: Vec<Vec<Point>> = Vec::new();
    let mut current: Vec<Point> = Vec::new();

    for evt in lyon_path.iter().flattened(tolerance) {
        use lyon::path::Event;
        match evt {
            Event::Begin { at } => {
                if current.len() >= 2 {
                    contours.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
                current.push(Point { x: at.x, y: at.y });
            }
            Event::Line { to, .. } => {
                current.push(Point { x: to.x, y: to.y });
            }
            Event::End { close, .. } => {
                if close && current.len() >= 2 {
                    // Add closing point matching the first.
                    let first = current[0];
                    current.push(first);
                }
                if current.len() >= 2 {
                    contours.push(std::mem::take(&mut current));
                }
                current.clear();
            }
            _ => {}
        }
    }
    if current.len() >= 2 {
        contours.push(current);
    }
    contours
}

/// Convert a PathData to a list of closed contours as `Vec<[f32; 2]>` for i_overlay.
/// Curves are flattened to line segments using lyon.
fn path_to_contours(path: &PathData, tolerance: f32) -> Vec<Vec<[f32; 2]>> {
    let tol = tolerance.max(MIN_TOLERANCE);
    let lyon_path = build_lyon_path(path);
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut current: Vec<[f32; 2]> = Vec::new();

    for evt in lyon_path.iter().flattened(tol) {
        use lyon::path::Event;
        match evt {
            Event::Begin { at } => {
                if current.len() >= 3 {
                    contours.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
                current.push([at.x, at.y]);
            }
            Event::Line { from: _, to } => {
                current.push([to.x, to.y]);
            }
            Event::End { first, close, .. } => {
                if close {
                    // Remove duplicate closing point if present
                    if let Some(last) = current.last() {
                        if (last[0] - first.x).abs() < 1e-6
                            && (last[1] - first.y).abs() < 1e-6
                        {
                            current.pop();
                        }
                    }
                }
                if current.len() >= 3 {
                    contours.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
            _ => {}
        }
    }
    // Handle unclosed trailing contour
    if current.len() >= 3 {
        contours.push(current);
    }
    contours
}

/// Convert i_overlay result contours back to PathData.
fn contours_to_path(shapes: Vec<Vec<Vec<[f32; 2]>>>) -> PathData {
    let mut verbs = Vec::new();
    for shape in &shapes {
        for contour in shape {
            if contour.is_empty() {
                continue;
            }
            verbs.push(PathVerb::MoveTo(Point {
                x: contour[0][0],
                y: contour[0][1],
            }));
            for pt in &contour[1..] {
                verbs.push(PathVerb::LineTo(Point { x: pt[0], y: pt[1] }));
            }
            verbs.push(PathVerb::Close);
        }
    }
    PathData {
        verbs,
        closed: true,
    }
}

/// Perform a boolean operation on two paths using i_overlay.
/// operation: 0=Union, 1=Intersect, 2=Difference, 3=Xor
pub fn path_boolean(a: &PathData, b: &PathData, operation: i32, tolerance: f32) -> PathData {
    let subj = path_to_contours(a, tolerance);
    let clip = path_to_contours(b, tolerance);

    if subj.is_empty() && clip.is_empty() {
        return PathData::new();
    }

    let rule = match operation {
        0 => OverlayRule::Union,
        1 => OverlayRule::Intersect,
        2 => OverlayRule::Difference,
        3 => OverlayRule::Xor,
        _ => OverlayRule::Union,
    };

    let result = subj.overlay(&clip, rule, FillRule::EvenOdd);
    contours_to_path(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Make a closed square from (0,0) to (10,10).
    fn make_square(x: f32, y: f32, size: f32) -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x, y }),
                PathVerb::LineTo(Point { x: x + size, y }),
                PathVerb::LineTo(Point { x: x + size, y: y + size }),
                PathVerb::LineTo(Point { x, y: y + size }),
                PathVerb::Close,
            ],
            closed: true,
        }
    }

    const TOL: f32 = DEFAULT_FLATTEN_TOLERANCE;

    #[test]
    fn boolean_union_combines_overlapping_squares() {
        let a = make_square(0.0, 0.0, 10.0);
        let b = make_square(5.0, 0.0, 10.0);
        let result = path_boolean(&a, &b, 0, TOL); // Union
        // Union of two overlapping 10x10 squares should produce a single contour
        // covering 15x10 area. Result should have verbs and be closed.
        assert!(!result.verbs.is_empty());
        assert!(result.closed);
        // Should have exactly one MoveTo (single contour)
        let move_count = result.verbs.iter().filter(|v| matches!(v, PathVerb::MoveTo(_))).count();
        assert_eq!(move_count, 1);
    }

    #[test]
    fn boolean_intersect_finds_overlap() {
        let a = make_square(0.0, 0.0, 10.0);
        let b = make_square(5.0, 0.0, 10.0);
        let result = path_boolean(&a, &b, 1, TOL); // Intersect
        // Intersection should be a 5x10 rectangle
        assert!(!result.verbs.is_empty());
        assert!(result.closed);
        let move_count = result.verbs.iter().filter(|v| matches!(v, PathVerb::MoveTo(_))).count();
        assert_eq!(move_count, 1);
    }

    #[test]
    fn boolean_difference_subtracts() {
        let a = make_square(0.0, 0.0, 10.0);
        let b = make_square(5.0, 0.0, 10.0);
        let result = path_boolean(&a, &b, 2, TOL); // Difference (a - b)
        // Should produce a 5x10 rectangle (left half of a)
        assert!(!result.verbs.is_empty());
        assert!(result.closed);
    }

    #[test]
    fn boolean_xor_excludes_overlap() {
        let a = make_square(0.0, 0.0, 10.0);
        let b = make_square(5.0, 0.0, 10.0);
        let result = path_boolean(&a, &b, 3, TOL); // Xor
        // Xor should produce two separate rectangles (non-overlapping parts)
        assert!(!result.verbs.is_empty());
        assert!(result.closed);
        let move_count = result.verbs.iter().filter(|v| matches!(v, PathVerb::MoveTo(_))).count();
        assert_eq!(move_count, 2);
    }

    #[test]
    fn boolean_disjoint_union() {
        let a = make_square(0.0, 0.0, 5.0);
        let b = make_square(20.0, 20.0, 5.0);
        let result = path_boolean(&a, &b, 0, TOL); // Union
        // Non-overlapping: union should produce two separate contours
        let move_count = result.verbs.iter().filter(|v| matches!(v, PathVerb::MoveTo(_))).count();
        assert_eq!(move_count, 2);
    }

    #[test]
    fn boolean_disjoint_intersect_is_empty() {
        let a = make_square(0.0, 0.0, 5.0);
        let b = make_square(20.0, 20.0, 5.0);
        let result = path_boolean(&a, &b, 1, TOL); // Intersect
        // Non-overlapping: intersection should be empty
        assert!(result.verbs.is_empty());
    }

    #[test]
    fn boolean_empty_inputs() {
        let empty = PathData::new();
        let sq = make_square(0.0, 0.0, 10.0);
        // Both empty
        let result = path_boolean(&empty, &empty, 0, TOL);
        assert!(result.verbs.is_empty());
        // One empty: union should return the non-empty shape
        let result = path_boolean(&sq, &empty, 0, TOL);
        assert!(!result.verbs.is_empty());
    }

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
        let result = resample_path(&tri, 10, TOL);
        if let NodeData::Points(pts) = result {
            assert_eq!(pts.len(), 10);
        } else {
            panic!("expected Points");
        }
    }

    fn make_circle_with_curves() -> PathData {
        let r = 50.0f32;
        let k = 0.5522847498f32 * r; // cubic bezier approximation of quarter circle
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: r, y: 0.0 }),
                PathVerb::CubicTo { ctrl1: Point { x: r, y: k }, ctrl2: Point { x: k, y: r }, to: Point { x: 0.0, y: r } },
                PathVerb::CubicTo { ctrl1: Point { x: -k, y: r }, ctrl2: Point { x: -r, y: k }, to: Point { x: -r, y: 0.0 } },
                PathVerb::CubicTo { ctrl1: Point { x: -r, y: -k }, ctrl2: Point { x: -k, y: -r }, to: Point { x: 0.0, y: -r } },
                PathVerb::CubicTo { ctrl1: Point { x: k, y: -r }, ctrl2: Point { x: r, y: -k }, to: Point { x: r, y: 0.0 } },
                PathVerb::Close,
            ],
            closed: true,
        }
    }

    #[test]
    fn lower_tolerance_produces_more_segments() {
        let circle = make_circle_with_curves();
        let coarse = flatten_to_segments(&circle, 5.0);
        let fine = flatten_to_segments(&circle, 0.1);
        assert!(fine.len() > coarse.len(),
            "fine ({}) should have more segments than coarse ({})", fine.len(), coarse.len());
    }

    #[test]
    fn tolerance_clamped_above_zero() {
        let circle = make_circle_with_curves();
        let result = flatten_to_segments(&circle, 0.0);
        assert!(!result.is_empty(), "zero tolerance should be clamped and still produce segments");
        let result_neg = flatten_to_segments(&circle, -1.0);
        assert!(!result_neg.is_empty(), "negative tolerance should be clamped and still produce segments");
    }

    #[test]
    fn boolean_with_custom_tolerance() {
        let a = make_square(0.0, 0.0, 10.0);
        let b = make_square(5.0, 0.0, 10.0);
        let result = path_boolean(&a, &b, 0, 0.1);
        assert!(!result.verbs.is_empty());
        assert!(result.closed);
    }

    #[test]
    fn resample_with_custom_tolerance() {
        let circle = make_circle_with_curves();
        let result = resample_path(&circle, 20, 0.1);
        if let NodeData::Points(pts) = result {
            assert_eq!(pts.len(), 20);
        } else {
            panic!("expected Points");
        }
    }

    // ── Path Offset tests ───────────────────────────────────────

    /// Helper: extract all points (MoveTo/LineTo) from a PathData.
    fn collect_points(path: &PathData) -> Vec<Point> {
        path.verbs.iter().filter_map(|v| match v {
            PathVerb::MoveTo(p) | PathVerb::LineTo(p) => Some(*p),
            _ => None,
        }).collect()
    }

    #[test]
    fn offset_square_expands_uniformly() {
        // CCW square: (0,0) → (10,0) → (10,10) → (0,10)
        // With positive distance, each edge should move outward by exactly `dist`.
        let sq = make_square(0.0, 0.0, 10.0);
        let dist = 5.0;
        let result = path_offset(&sq, dist as f64, TOL);
        let pts = collect_points(&result);

        // All points should be exactly `dist` outside the original square.
        // For a square, the offset corners should be at (-5,-5), (15,-5), (15,15), (-5,15).
        assert_eq!(pts.len(), 4, "offset square should have 4 vertices");
        for p in &pts {
            // Each coordinate should be either -5 or 15.
            let valid_x = (p.x - (-dist)).abs() < 0.1 || (p.x - (10.0 + dist)).abs() < 0.1;
            let valid_y = (p.y - (-dist)).abs() < 0.1 || (p.y - (10.0 + dist)).abs() < 0.1;
            assert!(valid_x, "x={} not at expected offset position", p.x);
            assert!(valid_y, "y={} not at expected offset position", p.y);
        }
    }

    #[test]
    fn offset_square_contracts_with_negative_distance() {
        let sq = make_square(0.0, 0.0, 10.0);
        let dist = -2.0;
        let result = path_offset(&sq, dist as f64, TOL);
        let pts = collect_points(&result);

        // Inset by 2: corners at (2,2), (8,2), (8,8), (2,8).
        assert_eq!(pts.len(), 4);
        for p in &pts {
            let valid_x = (p.x - 2.0).abs() < 0.1 || (p.x - 8.0).abs() < 0.1;
            let valid_y = (p.y - 2.0).abs() < 0.1 || (p.y - 8.0).abs() < 0.1;
            assert!(valid_x, "x={} not at expected inset position", p.x);
            assert!(valid_y, "y={} not at expected inset position", p.y);
        }
    }

    #[test]
    fn offset_rectangle_uniform_distance() {
        // Non-square rectangle: the old bug was uneven offset on different axes.
        let rect = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 200.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 200.0, y: 100.0 }),
                PathVerb::LineTo(Point { x: 0.0, y: 100.0 }),
                PathVerb::Close,
            ],
            closed: true,
        };
        let dist = 15.0;
        let result = path_offset(&rect, dist as f64, TOL);
        let pts = collect_points(&result);

        // Expected corners: (-15,-15), (215,-15), (215,115), (-15,115).
        assert_eq!(pts.len(), 4);
        let xs: Vec<f32> = pts.iter().map(|p| p.x).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p.y).collect();
        let min_x = xs.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_x = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min_y = ys.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_y = ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!((min_x - (-15.0)).abs() < 0.1, "min_x={} expected -15", min_x);
        assert!((max_x - 215.0).abs() < 0.1, "max_x={} expected 215", max_x);
        assert!((min_y - (-15.0)).abs() < 0.1, "min_y={} expected -15", min_y);
        assert!((max_y - 115.0).abs() < 0.1, "max_y={} expected 115", max_y);
    }

    #[test]
    fn offset_circle_stays_circular() {
        // Offset a circle by 10 — result should be a larger circle.
        let circle = make_circle_with_curves();
        let dist = 10.0;
        let result = path_offset(&circle, dist as f64, 0.1);
        let pts = collect_points(&result);

        // All offset points should be approximately r+dist from the origin.
        let expected_r = 50.0 + dist;
        for p in &pts {
            let r = (p.x * p.x + p.y * p.y).sqrt();
            assert!(
                (r - expected_r).abs() < 1.5,
                "point ({:.1},{:.1}) radius {:.1} expected ~{:.1}",
                p.x, p.y, r, expected_r
            );
        }
    }

    #[test]
    fn offset_zero_distance_returns_equivalent() {
        let sq = make_square(0.0, 0.0, 10.0);
        let result = path_offset(&sq, 0.0, TOL);
        // Zero offset should return the original path unchanged.
        assert_eq!(result.verbs.len(), sq.verbs.len());
    }

    #[test]
    fn offset_open_path() {
        // Open path: just a horizontal line.
        let line = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
            ],
            closed: false,
        };
        let result = path_offset(&line, 5.0, TOL);
        let pts = collect_points(&result);
        // Should offset upward (left-hand normal of rightward line is up).
        assert_eq!(pts.len(), 2);
        assert!((pts[0].y - (-5.0)).abs() < 0.1 || (pts[0].y - 5.0).abs() < 0.1);
        // Both y values should be the same (parallel line).
        assert!((pts[0].y - pts[1].y).abs() < 0.1);
    }
}
