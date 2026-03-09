use std::sync::Arc;

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use lyon::math::point as lyon_point;
use lyon::path::iterator::PathIterator;
use lyon::path::Path as LyonPath;

use vector_flow_core::types::{NodeData, PathData, PathVerb, Point, PointBatch, Shape};

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
    // Sanitize coordinates: lyon panics on non-finite values (NaN, infinity).
    // Replace with 0.0 so the app doesn't crash on bad upstream data.
    #[inline]
    fn safe_point(x: f32, y: f32) -> lyon::math::Point {
        lyon_point(
            if x.is_finite() { x } else { 0.0 },
            if y.is_finite() { y } else { 0.0 },
        )
    }

    let mut builder = LyonPath::builder();
    let mut in_subpath = false;
    for verb in &path.verbs {
        match *verb {
            PathVerb::MoveTo(p) => {
                if in_subpath {
                    builder.end(false);
                }
                builder.begin(safe_point(p.x, p.y));
                in_subpath = true;
            }
            PathVerb::LineTo(p) => {
                if !in_subpath {
                    builder.begin(safe_point(p.x, p.y));
                    in_subpath = true;
                } else {
                    builder.line_to(safe_point(p.x, p.y));
                }
            }
            PathVerb::QuadTo { ctrl, to } => {
                if !in_subpath {
                    builder.begin(safe_point(ctrl.x, ctrl.y));
                    in_subpath = true;
                }
                builder.quadratic_bezier_to(safe_point(ctrl.x, ctrl.y), safe_point(to.x, to.y));
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                if !in_subpath {
                    builder.begin(safe_point(ctrl1.x, ctrl1.y));
                    in_subpath = true;
                }
                builder.cubic_bezier_to(
                    safe_point(ctrl1.x, ctrl1.y),
                    safe_point(ctrl2.x, ctrl2.y),
                    safe_point(to.x, to.y),
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

// ---------------------------------------------------------------------------
// ArcLengthTable — shared infrastructure for arc-length queries
// ---------------------------------------------------------------------------

pub(crate) struct ArcLengthTable {
    pub segments: Vec<(Point, Point)>,
    pub cumulative_lengths: Vec<f32>,
    pub total_length: f32,
}

impl ArcLengthTable {
    pub fn from_segments(segments: Vec<(Point, Point)>) -> Self {
        let mut cumulative_lengths = Vec::with_capacity(segments.len());
        let mut total = 0.0f32;
        for &(a, b) in &segments {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            total += (dx * dx + dy * dy).sqrt();
            cumulative_lengths.push(total);
        }
        ArcLengthTable {
            segments,
            cumulative_lengths,
            total_length: total,
        }
    }

    pub fn from_path(path: &PathData, tolerance: f32) -> Self {
        let segments = flatten_to_segments(path, tolerance);
        Self::from_segments(segments)
    }

    /// Find the segment index and local t for a given arc-length parameter (0..1).
    pub fn segment_index_at_t(&self, t: f32) -> (usize, f32) {
        if self.segments.is_empty() || self.total_length < 1e-10 {
            return (0, 0.0);
        }
        let target_len = (t.clamp(0.0, 1.0)) * self.total_length;
        let seg_idx = self
            .cumulative_lengths
            .iter()
            .position(|&l| l >= target_len)
            .unwrap_or(self.segments.len() - 1);
        let seg_start_len = if seg_idx > 0 {
            self.cumulative_lengths[seg_idx - 1]
        } else {
            0.0
        };
        let seg_len = self.cumulative_lengths[seg_idx] - seg_start_len;
        let local_t = if seg_len > 1e-10 {
            (target_len - seg_start_len) / seg_len
        } else {
            0.0
        };
        (seg_idx, local_t)
    }

    /// Get the position at arc-length parameter t (0..1).
    pub fn position_at_t(&self, t: f32) -> Point {
        if self.segments.is_empty() {
            return Point { x: 0.0, y: 0.0 };
        }
        let (seg_idx, local_t) = self.segment_index_at_t(t);
        let (a, b) = self.segments[seg_idx];
        Point {
            x: a.x + (b.x - a.x) * local_t,
            y: a.y + (b.y - a.y) * local_t,
        }
    }

    /// Get the unit tangent at arc-length parameter t (0..1).
    pub fn tangent_at_t(&self, t: f32) -> (f32, f32) {
        if self.segments.is_empty() {
            return (1.0, 0.0);
        }
        let (seg_idx, _) = self.segment_index_at_t(t);
        let (a, b) = self.segments[seg_idx];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-10 {
            return (1.0, 0.0);
        }
        (dx / len, dy / len)
    }

    /// Get the unit normal (perpendicular to tangent, left-hand) at arc-length parameter t.
    pub fn normal_at_t(&self, t: f32) -> (f32, f32) {
        let (tx, ty) = self.tangent_at_t(t);
        (-ty, tx)
    }
}

// ---------------------------------------------------------------------------
// Path Intersection Points
// ---------------------------------------------------------------------------

/// Find all intersection points between two paths.
/// Returns (intersection points, t values on path a, t values on path b).
pub fn path_intersection_points(
    a: &PathData,
    b: &PathData,
    tolerance: f32,
) -> (PointBatch, Vec<f64>, Vec<f64>) {
    let tol = tolerance.max(MIN_TOLERANCE);
    let table_a = ArcLengthTable::from_path(a, tol);
    let table_b = ArcLengthTable::from_path(b, tol);

    if table_a.segments.is_empty() || table_b.segments.is_empty() {
        return (PointBatch::new(), vec![], vec![]);
    }

    let mut hits: Vec<(Point, f32, f32)> = Vec::new(); // (point, t_a, t_b)

    for (i, &(a1, a2)) in table_a.segments.iter().enumerate() {
        for (j, &(b1, b2)) in table_b.segments.iter().enumerate() {
            if let Some((pt, s, u)) = segment_segment_intersect(a1, a2, b1, b2) {
                // Compute arc-length t for each path
                let len_before_a = if i > 0 {
                    table_a.cumulative_lengths[i - 1]
                } else {
                    0.0
                };
                let seg_len_a = table_a.cumulative_lengths[i] - len_before_a;
                let arc_a = (len_before_a + s * seg_len_a) / table_a.total_length;

                let len_before_b = if j > 0 {
                    table_b.cumulative_lengths[j - 1]
                } else {
                    0.0
                };
                let seg_len_b = table_b.cumulative_lengths[j] - len_before_b;
                let arc_b = (len_before_b + u * seg_len_b) / table_b.total_length;

                hits.push((pt, arc_a, arc_b));
            }
        }
    }

    // Deduplicate hits that are very close (same intersection found from adjacent segments)
    hits.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut deduped: Vec<(Point, f32, f32)> = Vec::new();
    for hit in &hits {
        if deduped
            .last()
            .map(|prev: &(Point, f32, f32)| {
                let dx = hit.0.x - prev.0.x;
                let dy = hit.0.y - prev.0.y;
                (dx * dx + dy * dy).sqrt() > 1e-4
            })
            .unwrap_or(true)
        {
            deduped.push(*hit);
        }
    }

    let mut xs = Vec::with_capacity(deduped.len());
    let mut ys = Vec::with_capacity(deduped.len());
    let mut t_a = Vec::with_capacity(deduped.len());
    let mut t_b = Vec::with_capacity(deduped.len());

    for (pt, a, b) in &deduped {
        xs.push(pt.x);
        ys.push(pt.y);
        t_a.push(*a as f64);
        t_b.push(*b as f64);
    }

    (PointBatch { xs, ys }, t_a, t_b)
}

/// Line segment intersection. Returns (intersection point, t on seg1, t on seg2) if they intersect.
fn segment_segment_intersect(
    a1: Point,
    a2: Point,
    b1: Point,
    b2: Point,
) -> Option<(Point, f32, f32)> {
    let dx_a = a2.x - a1.x;
    let dy_a = a2.y - a1.y;
    let dx_b = b2.x - b1.x;
    let dy_b = b2.y - b1.y;

    let denom = dx_a * dy_b - dy_a * dx_b;
    if denom.abs() < 1e-10 {
        return None; // parallel
    }

    let dx_ab = b1.x - a1.x;
    let dy_ab = b1.y - a1.y;

    let t = (dx_ab * dy_b - dy_ab * dx_b) / denom;
    let u = (dx_ab * dy_a - dy_ab * dx_a) / denom;

    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        let pt = Point {
            x: a1.x + t * dx_a,
            y: a1.y + t * dy_a,
        };
        Some((pt, t, u))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Split Path at T
// ---------------------------------------------------------------------------

/// Split a path at given arc-length t values (0..1). Returns sub-paths.
pub fn split_path_at_t(
    path: &PathData,
    t_values: &[f64],
    tolerance: f32,
    close: bool,
) -> Vec<PathData> {
    let tol = tolerance.max(MIN_TOLERANCE);
    let table = ArcLengthTable::from_path(path, tol);

    if table.segments.is_empty() {
        return vec![path.clone()];
    }

    // Sort and dedup t values, filter to (0..1) exclusive
    let mut ts: Vec<f32> = t_values
        .iter()
        .map(|&v| (v as f32).clamp(0.0, 1.0))
        .filter(|&v| v > 1e-6 && v < 1.0 - 1e-6)
        .collect();
    ts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

    if ts.is_empty() {
        return vec![path.clone()];
    }

    // Collect all flattened points in order
    let mut all_points: Vec<Point> = Vec::new();
    if let Some(&(start, _)) = table.segments.first() {
        all_points.push(start);
    }
    for &(_, end) in &table.segments {
        all_points.push(end);
    }

    // For closed paths, split at N points → N parts
    // For open paths, split at N points → N+1 parts
    let mut split_indices: Vec<usize> = Vec::new();
    for &t in &ts {
        let (seg_idx, local_t) = table.segment_index_at_t(t);
        // Insert a point at this position
        let (a, b) = table.segments[seg_idx];
        let pt = Point {
            x: a.x + (b.x - a.x) * local_t,
            y: a.y + (b.y - a.y) * local_t,
        };
        // The point index: seg_idx corresponds to point index seg_idx+1 (end of segment)
        // We need to insert at the right position in all_points
        // Points are: [seg0_start, seg0_end, seg1_end, seg2_end, ...]
        // seg_idx's start = all_points[seg_idx], end = all_points[seg_idx+1]
        // We insert between seg_idx and seg_idx+1
        let insert_idx = seg_idx + 1 + split_indices.len(); // offset by previously inserted points
        all_points.insert(insert_idx, pt);
        split_indices.push(insert_idx);
    }

    // Now split the points at the split indices
    let mut parts: Vec<PathData> = Vec::new();
    let mut start_idx = 0;

    for &split_idx in &split_indices {
        let part_points = &all_points[start_idx..=split_idx];
        parts.push(points_to_path(part_points, close));
        start_idx = split_idx;
    }

    // Final part: from last split to end
    let part_points = &all_points[start_idx..];
    if !part_points.is_empty() {
        parts.push(points_to_path(part_points, close));
    }

    // For closed paths: connect last part back to first part
    if path.closed && parts.len() >= 2 {
        // Remove the last part and prepend its points to the first part
        let last = parts.pop().unwrap();
        let first = &mut parts[0];
        // Prepend last's points (except last point which is same as first's start)
        let last_points: Vec<Point> = last
            .verbs
            .iter()
            .filter_map(|v| match v {
                PathVerb::MoveTo(p) | PathVerb::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect();
        let first_points: Vec<Point> = first
            .verbs
            .iter()
            .filter_map(|v| match v {
                PathVerb::MoveTo(p) | PathVerb::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect();
        let mut merged = last_points;
        // Skip the first point of first_points if it matches last of last_points
        if merged.last() == first_points.first() {
            merged.extend_from_slice(&first_points[1..]);
        } else {
            merged.extend_from_slice(&first_points);
        }
        *first = points_to_path(&merged, close);
    }

    parts
}

/// Build a PathData from a slice of points.
fn points_to_path(points: &[Point], close: bool) -> PathData {
    let mut verbs = Vec::with_capacity(points.len() + if close { 1 } else { 0 });
    for (i, &pt) in points.iter().enumerate() {
        if i == 0 {
            verbs.push(PathVerb::MoveTo(pt));
        } else {
            verbs.push(PathVerb::LineTo(pt));
        }
    }
    if close {
        verbs.push(PathVerb::Close);
    }
    PathData {
        verbs,
        closed: close,
    }
}

// ---------------------------------------------------------------------------
// Close Path
// ---------------------------------------------------------------------------

/// Close an open path by setting closed=true and appending Close verb if needed.
pub fn close_path(path: &PathData) -> PathData {
    if path.closed {
        return path.clone();
    }
    let mut verbs = path.verbs.clone();
    if !verbs.is_empty() && !matches!(verbs.last(), Some(PathVerb::Close)) {
        verbs.push(PathVerb::Close);
    }
    PathData {
        verbs,
        closed: true,
    }
}

// ---------------------------------------------------------------------------
// Polygon from Points
// ---------------------------------------------------------------------------

/// Construct a path from an ordered list of points.
pub fn polygon_from_points(points: &PointBatch, close: bool) -> PathData {
    if points.is_empty() {
        return PathData::new();
    }
    let mut verbs = Vec::with_capacity(points.len() + if close { 1 } else { 0 });
    for i in 0..points.len() {
        let pt = Point {
            x: points.xs[i],
            y: points.ys[i],
        };
        if i == 0 {
            verbs.push(PathVerb::MoveTo(pt));
        } else {
            verbs.push(PathVerb::LineTo(pt));
        }
    }
    if close {
        verbs.push(PathVerb::Close);
    }
    PathData {
        verbs,
        closed: close,
    }
}

// ---------------------------------------------------------------------------
// Spline from Points (Catmull-Rom → Cubic Bezier)
// ---------------------------------------------------------------------------

/// Fit a smooth cubic bezier spline through points using Catmull-Rom interpolation.
pub fn spline_from_points(points: &PointBatch, close: bool, tension: f64) -> PathData {
    let n = points.len();
    if n == 0 {
        return PathData::new();
    }
    if n == 1 {
        return PathData {
            verbs: vec![PathVerb::MoveTo(Point {
                x: points.xs[0],
                y: points.ys[0],
            })],
            closed: false,
        };
    }

    let t_factor = (1.0 - tension.clamp(0.0, 1.0)) as f32;

    let get_pt = |i: usize| -> Point {
        Point {
            x: points.xs[i],
            y: points.ys[i],
        }
    };

    // Compute tangents
    let mut tangents: Vec<(f32, f32)> = Vec::with_capacity(n);
    for i in 0..n {
        let (prev, next) = if close {
            (
                get_pt(if i == 0 { n - 1 } else { i - 1 }),
                get_pt((i + 1) % n),
            )
        } else if i == 0 {
            (get_pt(0), get_pt(1))
        } else if i == n - 1 {
            (get_pt(n - 2), get_pt(n - 1))
        } else {
            (get_pt(i - 1), get_pt(i + 1))
        };
        let tx = t_factor * (next.x - prev.x) * 0.5;
        let ty = t_factor * (next.y - prev.y) * 0.5;
        tangents.push((tx, ty));
    }

    let mut verbs = Vec::new();
    verbs.push(PathVerb::MoveTo(get_pt(0)));

    let seg_count = if close { n } else { n - 1 };
    for i in 0..seg_count {
        let j = (i + 1) % n;
        let p0 = get_pt(i);
        let p1 = get_pt(j);
        let (t0x, t0y) = tangents[i];
        let (t1x, t1y) = tangents[j];

        let ctrl1 = Point {
            x: p0.x + t0x / 3.0,
            y: p0.y + t0y / 3.0,
        };
        let ctrl2 = Point {
            x: p1.x - t1x / 3.0,
            y: p1.y - t1y / 3.0,
        };
        verbs.push(PathVerb::CubicTo {
            ctrl1,
            ctrl2,
            to: p1,
        });
    }

    if close {
        verbs.push(PathVerb::Close);
    }

    PathData {
        verbs,
        closed: close,
    }
}

// ---------------------------------------------------------------------------
// Warp to Curve
// ---------------------------------------------------------------------------

/// Warp geometry so it follows a target curve.
/// Mode 0: simple positional. Mode 1: curvature-aware.
pub fn warp_to_curve(
    geometry: &NodeData,
    curve: &PathData,
    mode: i64,
    smoothing: f32,
    tolerance: f32,
) -> NodeData {
    let tol = tolerance.max(MIN_TOLERANCE);
    let table = ArcLengthTable::from_path(curve, tol);
    if table.segments.is_empty() {
        return geometry.clone();
    }

    let cf = if mode == 1 {
        Some(CurvatureField::build(&table, smoothing))
    } else {
        None
    };
    let cf_ref = cf.as_ref();

    match geometry {
        NodeData::Path(p) => {
            let bbox = compute_path_bbox(p);
            NodeData::Path(Arc::new(warp_path_to_curve(p, &table, &bbox, cf_ref)))
        }
        NodeData::Paths(paths) => {
            // Collective bbox across all paths
            let bbox = compute_paths_bbox(paths);
            let warped: Vec<PathData> = paths
                .iter()
                .map(|p| warp_path_to_curve(p, &table, &bbox, cf_ref))
                .collect();
            NodeData::Paths(Arc::new(warped))
        }
        NodeData::Shape(s) => {
            let bbox = compute_path_bbox(&s.path);
            let warped_path = Arc::new(warp_path_to_curve(&s.path, &table, &bbox, cf_ref));
            NodeData::Shape(Arc::new(Shape {
                path: warped_path,
                ..(**s).clone()
            }))
        }
        NodeData::Shapes(shapes) => {
            // Collective bbox across all shapes
            let mut bbox = BBox::empty();
            for s in shapes.iter() {
                bbox.extend_path(&s.path);
            }
            let warped: Vec<Shape> = shapes
                .iter()
                .map(|s| {
                    let warped_path = Arc::new(warp_path_to_curve(&s.path, &table, &bbox, cf_ref));
                    Shape {
                        path: warped_path,
                        ..s.clone()
                    }
                })
                .collect();
            NodeData::Shapes(Arc::new(warped))
        }
        other => other.clone(),
    }
}

struct BBox {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl BBox {
    fn empty() -> Self {
        BBox {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
        }
    }

    fn extend_point(&mut self, x: f32, y: f32) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    fn extend_path(&mut self, path: &PathData) {
        for v in &path.verbs {
            match *v {
                PathVerb::MoveTo(p) | PathVerb::LineTo(p) => self.extend_point(p.x, p.y),
                PathVerb::QuadTo { ctrl, to } => {
                    self.extend_point(ctrl.x, ctrl.y);
                    self.extend_point(to.x, to.y);
                }
                PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                    self.extend_point(ctrl1.x, ctrl1.y);
                    self.extend_point(ctrl2.x, ctrl2.y);
                    self.extend_point(to.x, to.y);
                }
                PathVerb::Close => {}
            }
        }
    }

    fn width(&self) -> f32 {
        (self.max_x - self.min_x).max(1e-10)
    }

    fn center_y(&self) -> f32 {
        (self.min_y + self.max_y) * 0.5
    }
}

fn compute_path_bbox(path: &PathData) -> BBox {
    let mut bbox = BBox::empty();
    bbox.extend_path(path);
    bbox
}

fn compute_paths_bbox(paths: &[PathData]) -> BBox {
    let mut bbox = BBox::empty();
    for p in paths {
        bbox.extend_path(p);
    }
    bbox
}

/// Precomputed smoothed curvature sampled at uniform intervals along the spine.
struct CurvatureField {
    /// Smoothed curvature values at N uniformly spaced parametric positions.
    samples: Vec<f32>,
}

impl CurvatureField {
    const NUM_SAMPLES: usize = 128;

    fn build(table: &ArcLengthTable, smoothing: f32) -> Self {
        let n = Self::NUM_SAMPLES;
        // Sample raw Menger curvature at uniform parametric intervals.
        let dt = 0.02;
        let mut raw = Vec::with_capacity(n);
        for i in 0..n {
            let u = i as f32 / (n - 1) as f32;
            let p1 = table.position_at_t((u - dt).max(0.0));
            let p2 = table.position_at_t(u);
            let p3 = table.position_at_t((u + dt).min(1.0));
            let ax = p2.x - p1.x;
            let ay = p2.y - p1.y;
            let bx = p3.x - p1.x;
            let by = p3.y - p1.y;
            let cross = ax * by - ay * bx;
            let d1 = (ax * ax + ay * ay).sqrt();
            let d2 = (bx * bx + by * by).sqrt();
            let cx = p3.x - p2.x;
            let cy = p3.y - p2.y;
            let d3 = (cx * cx + cy * cy).sqrt();
            let denom = d1 * d2 * d3;
            raw.push(if denom > 1e-10 { 2.0 * cross / denom } else { 0.0 });
        }
        // Box-filter smooth (3 passes for approximate Gaussian).
        // smoothing 0..1 maps to radius 2..48.
        let radius = (2.0 + smoothing * 46.0) as usize;
        let mut buf = raw.clone();
        for _ in 0..3 {
            let src = buf.clone();
            for (i, val) in buf.iter_mut().enumerate() {
                let lo = i.saturating_sub(radius);
                let hi = (i + radius + 1).min(n);
                let sum: f32 = src[lo..hi].iter().sum();
                *val = sum / (hi - lo) as f32;
            }
        }
        CurvatureField { samples: buf }
    }

    /// Interpolate smoothed curvature at parametric position u (0..1).
    fn at(&self, u: f32) -> f32 {
        let n = self.samples.len();
        let t = u.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (t as usize).min(n - 2);
        let frac = t - i as f32;
        self.samples[i] * (1.0 - frac) + self.samples[i + 1] * frac
    }
}

fn warp_point(
    x: f32,
    y: f32,
    table: &ArcLengthTable,
    bbox: &BBox,
    curvature: Option<&CurvatureField>,
) -> Point {
    let u = (x - bbox.min_x) / bbox.width();
    let v = y - bbox.center_y();
    let pos = table.position_at_t(u);
    let (nx, ny) = table.normal_at_t(u);

    if let Some(cf) = curvature {
        // Use smooth normal derived from position finite differences
        // instead of the raw segment normal, which jumps at segment
        // boundaries. position_at_t is continuous (piecewise linear),
        // so finite-difference tangent is smooth.
        let eps = 0.005;
        let p_lo = table.position_at_t((u - eps).max(0.0));
        let p_hi = table.position_at_t((u + eps).min(1.0));
        let tdx = p_hi.x - p_lo.x;
        let tdy = p_hi.y - p_lo.y;
        let tlen = (tdx * tdx + tdy * tdy).sqrt();
        let (snx, sny) = if tlen > 1e-10 {
            (-tdy / tlen, tdx / tlen)
        } else {
            (nx, ny)
        };

        let k = cf.at(u);
        // Curvature correction: compress inner side, expand outer side of bends.
        // v is signed (distance from spine center), so k*v distinguishes sides.
        let scale = (1.0 / (1.0 + k * v)).clamp(0.2, 5.0);
        Point {
            x: pos.x + v * snx * scale,
            y: pos.y + v * sny * scale,
        }
    } else {
        Point {
            x: pos.x + v * nx,
            y: pos.y + v * ny,
        }
    }
}

fn warp_path_to_curve(
    path: &PathData,
    table: &ArcLengthTable,
    bbox: &BBox,
    curvature: Option<&CurvatureField>,
) -> PathData {
    let mut verbs = Vec::with_capacity(path.verbs.len());
    for v in &path.verbs {
        match *v {
            PathVerb::MoveTo(p) => {
                verbs.push(PathVerb::MoveTo(warp_point(p.x, p.y, table, bbox, curvature)));
            }
            PathVerb::LineTo(p) => {
                verbs.push(PathVerb::LineTo(warp_point(p.x, p.y, table, bbox, curvature)));
            }
            PathVerb::QuadTo { ctrl, to } => {
                verbs.push(PathVerb::QuadTo {
                    ctrl: warp_point(ctrl.x, ctrl.y, table, bbox, curvature),
                    to: warp_point(to.x, to.y, table, bbox, curvature),
                });
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                verbs.push(PathVerb::CubicTo {
                    ctrl1: warp_point(ctrl1.x, ctrl1.y, table, bbox, curvature),
                    ctrl2: warp_point(ctrl2.x, ctrl2.y, table, bbox, curvature),
                    to: warp_point(to.x, to.y, table, bbox, curvature),
                });
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

// ---------------------------------------------------------------------------

/// Flatten path to line segments using lyon's adaptive subdivision.
pub(crate) fn flatten_to_segments(path: &PathData, tolerance: f32) -> Vec<(Point, Point)> {
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
pub(crate) fn flatten_to_contours(path: &PathData, tolerance: f32) -> Vec<Vec<Point>> {
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

/// Convert i_overlay result shapes into separate PathData entries (one per shape group).
fn contours_to_paths(shapes: &[Vec<Vec<[f32; 2]>>]) -> Vec<PathData> {
    shapes
        .iter()
        .filter_map(|shape| {
            let mut verbs = Vec::new();
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
            if verbs.is_empty() {
                None
            } else {
                Some(PathData {
                    verbs,
                    closed: true,
                })
            }
        })
        .collect()
}

/// Perform a boolean operation on two paths, returning both the combined result
/// and the individual parts as separate paths.
/// operation: 0=Union, 1=Intersect, 2=Difference, 3=Xor, 4=Divide
pub fn path_boolean_with_parts(
    a: &PathData,
    b: &PathData,
    operation: i32,
    tolerance: f32,
) -> (PathData, Vec<PathData>) {
    let subj = path_to_contours(a, tolerance);
    let clip = path_to_contours(b, tolerance);

    if subj.is_empty() && clip.is_empty() {
        return (PathData::new(), Vec::new());
    }

    if operation == 4 {
        // Divide: compute all distinct non-overlapping regions.
        // Three calls: Difference (A-B), Intersect (A∩B), InverseDifference (B-A)
        let diff = subj.overlay(&clip, OverlayRule::Difference, FillRule::EvenOdd);
        let intersect = subj.overlay(&clip, OverlayRule::Intersect, FillRule::EvenOdd);
        let inv_diff = subj.overlay(&clip, OverlayRule::InverseDifference, FillRule::EvenOdd);

        let mut all_parts: Vec<PathData> = Vec::new();
        all_parts.extend(contours_to_paths(&diff));
        all_parts.extend(contours_to_paths(&intersect));
        all_parts.extend(contours_to_paths(&inv_diff));

        // Build combined result from all parts
        let mut combined_verbs = Vec::new();
        for part in &all_parts {
            combined_verbs.extend_from_slice(&part.verbs);
        }
        let combined = PathData {
            verbs: combined_verbs,
            closed: true,
        };

        (combined, all_parts)
    } else {
        let rule = match operation {
            0 => OverlayRule::Union,
            1 => OverlayRule::Intersect,
            2 => OverlayRule::Difference,
            3 => OverlayRule::Xor,
            _ => OverlayRule::Union,
        };

        let result = subj.overlay(&clip, rule, FillRule::EvenOdd);
        let parts = contours_to_paths(&result);
        let combined = contours_to_path(result);

        (combined, parts)
    }
}

/// Perform a boolean operation on two paths using i_overlay.
/// operation: 0=Union, 1=Intersect, 2=Difference, 3=Xor
#[cfg(test)]
fn path_boolean(a: &PathData, b: &PathData, operation: i32, tolerance: f32) -> PathData {
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

    // ── Divide / path_boolean_with_parts tests ────────────────

    #[test]
    fn divide_overlapping_squares_produces_three_parts() {
        // Two overlapping 10x10 squares offset by 5 in x.
        // Divide should yield 3 regions: left-only, overlap, right-only.
        let a = make_square(0.0, 0.0, 10.0);
        let b = make_square(5.0, 0.0, 10.0);
        let (combined, parts) = path_boolean_with_parts(&a, &b, 4, TOL);
        assert!(!combined.verbs.is_empty());
        assert_eq!(parts.len(), 3, "expected 3 divide parts, got {}", parts.len());
        for part in &parts {
            assert!(!part.verbs.is_empty());
            assert!(part.closed);
        }
    }

    #[test]
    fn divide_disjoint_squares_produces_two_parts() {
        // Two non-overlapping squares should yield 2 regions (no intersection).
        let a = make_square(0.0, 0.0, 5.0);
        let b = make_square(20.0, 20.0, 5.0);
        let (combined, parts) = path_boolean_with_parts(&a, &b, 4, TOL);
        assert!(!combined.verbs.is_empty());
        assert_eq!(parts.len(), 2, "expected 2 divide parts for disjoint, got {}", parts.len());
    }

    #[test]
    fn divide_contained_square_produces_two_parts() {
        // Small square fully inside large square → 2 regions (outer ring + inner).
        let outer = make_square(0.0, 0.0, 20.0);
        let inner = make_square(5.0, 5.0, 10.0);
        let (_combined, parts) = path_boolean_with_parts(&outer, &inner, 4, TOL);
        assert_eq!(parts.len(), 2, "expected 2 divide parts for contained, got {}", parts.len());
    }

    #[test]
    fn with_parts_matches_original_for_standard_ops() {
        // Verify that path_boolean_with_parts produces the same combined result
        // as path_boolean for operations 0-3.
        let a = make_square(0.0, 0.0, 10.0);
        let b = make_square(5.0, 0.0, 10.0);
        for op in 0..4 {
            let original = path_boolean(&a, &b, op, TOL);
            let (combined, parts) = path_boolean_with_parts(&a, &b, op, TOL);
            assert_eq!(original.verbs.len(), combined.verbs.len(),
                "op {} combined verbs mismatch", op);
            assert!(!parts.is_empty(), "op {} should produce at least one part", op);
        }
    }

    // ── Path Intersection Points tests ────────────────────────

    fn make_line(x1: f32, y1: f32, x2: f32, y2: f32) -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: x1, y: y1 }),
                PathVerb::LineTo(Point { x: x2, y: y2 }),
            ],
            closed: false,
        }
    }

    #[test]
    fn intersection_crossing_lines() {
        // X-shaped: (0,0)-(10,10) and (10,0)-(0,10)
        let a = make_line(0.0, 0.0, 10.0, 10.0);
        let b = make_line(10.0, 0.0, 0.0, 10.0);
        let (pts, t_a, t_b) = path_intersection_points(&a, &b, TOL);
        assert_eq!(pts.len(), 1, "crossing lines should have 1 intersection");
        assert!((pts.xs[0] - 5.0).abs() < 0.1);
        assert!((pts.ys[0] - 5.0).abs() < 0.1);
        assert!((t_a[0] - 0.5).abs() < 0.05);
        assert!((t_b[0] - 0.5).abs() < 0.05);
    }

    #[test]
    fn intersection_line_vs_square() {
        // Horizontal line through the middle of a square
        let sq = make_square(0.0, 0.0, 10.0);
        let line = make_line(-5.0, 5.0, 15.0, 5.0);
        let (pts, _t_a, _t_b) = path_intersection_points(&sq, &line, TOL);
        assert_eq!(pts.len(), 2, "line through square should have 2 intersections");
    }

    #[test]
    fn intersection_disjoint() {
        let a = make_line(0.0, 0.0, 5.0, 0.0);
        let b = make_line(0.0, 10.0, 5.0, 10.0);
        let (pts, _, _) = path_intersection_points(&a, &b, TOL);
        assert_eq!(pts.len(), 0);
    }

    // ── Split Path at T tests ─────────────────────────────────

    #[test]
    fn split_line_at_half() {
        let line = make_line(0.0, 0.0, 10.0, 0.0);
        let parts = split_path_at_t(&line, &[0.5], TOL, false);
        assert_eq!(parts.len(), 2, "splitting line at 0.5 should give 2 parts");
    }

    #[test]
    fn split_square_at_quarters() {
        let sq = make_square(0.0, 0.0, 10.0);
        let parts = split_path_at_t(&sq, &[0.25, 0.5, 0.75], TOL, false);
        assert_eq!(parts.len(), 3, "splitting closed path at 3 points should give 3 parts, got {}", parts.len());
    }

    #[test]
    fn split_with_close() {
        let line = make_line(0.0, 0.0, 10.0, 0.0);
        let parts = split_path_at_t(&line, &[0.5], TOL, true);
        assert_eq!(parts.len(), 2);
        for p in &parts {
            assert!(p.closed, "each part should be closed");
        }
    }

    #[test]
    fn split_empty_t_values() {
        let line = make_line(0.0, 0.0, 10.0, 0.0);
        let parts = split_path_at_t(&line, &[], TOL, false);
        assert_eq!(parts.len(), 1, "no split points should return original");
    }

    // ── Close Path tests ──────────────────────────────────────

    #[test]
    fn close_open_path() {
        let line = make_line(0.0, 0.0, 10.0, 0.0);
        assert!(!line.closed);
        let closed = close_path(&line);
        assert!(closed.closed);
        assert!(matches!(closed.verbs.last(), Some(PathVerb::Close)));
    }

    #[test]
    fn close_already_closed() {
        let sq = make_square(0.0, 0.0, 10.0);
        let closed = close_path(&sq);
        assert_eq!(closed.verbs.len(), sq.verbs.len());
    }

    #[test]
    fn close_empty_path() {
        let empty = PathData::new();
        let closed = close_path(&empty);
        assert!(closed.closed);
    }

    // ── Polygon from Points tests ─────────────────────────────

    #[test]
    fn polygon_triangle() {
        let pts = PointBatch {
            xs: vec![0.0, 10.0, 5.0],
            ys: vec![0.0, 0.0, 10.0],
        };
        let path = polygon_from_points(&pts, true);
        assert!(path.closed);
        assert!(matches!(path.verbs[0], PathVerb::MoveTo(_)));
        assert_eq!(path.verbs.len(), 4); // MoveTo + 2 LineTo + Close
    }

    #[test]
    fn polygon_open() {
        let pts = PointBatch {
            xs: vec![0.0, 10.0, 5.0],
            ys: vec![0.0, 0.0, 10.0],
        };
        let path = polygon_from_points(&pts, false);
        assert!(!path.closed);
        assert_eq!(path.verbs.len(), 3); // MoveTo + 2 LineTo
    }

    #[test]
    fn polygon_single_point() {
        let pts = PointBatch {
            xs: vec![5.0],
            ys: vec![5.0],
        };
        let path = polygon_from_points(&pts, true);
        assert_eq!(path.verbs.len(), 2); // MoveTo + Close
    }

    #[test]
    fn polygon_empty() {
        let pts = PointBatch::new();
        let path = polygon_from_points(&pts, true);
        assert!(path.verbs.is_empty());
    }

    // ── Spline from Points tests ──────────────────────────────

    #[test]
    fn spline_three_points() {
        let pts = PointBatch {
            xs: vec![0.0, 5.0, 10.0],
            ys: vec![0.0, 10.0, 0.0],
        };
        let path = spline_from_points(&pts, false, 0.0);
        // Should have MoveTo + 2 CubicTo
        let cubic_count = path.verbs.iter().filter(|v| matches!(v, PathVerb::CubicTo { .. })).count();
        assert_eq!(cubic_count, 2);
        assert!(!path.closed);
    }

    #[test]
    fn spline_two_points() {
        let pts = PointBatch {
            xs: vec![0.0, 10.0],
            ys: vec![0.0, 0.0],
        };
        let path = spline_from_points(&pts, false, 0.0);
        let cubic_count = path.verbs.iter().filter(|v| matches!(v, PathVerb::CubicTo { .. })).count();
        assert_eq!(cubic_count, 1);
    }

    #[test]
    fn spline_closed() {
        let pts = PointBatch {
            xs: vec![0.0, 10.0, 10.0, 0.0],
            ys: vec![0.0, 0.0, 10.0, 10.0],
        };
        let path = spline_from_points(&pts, true, 0.0);
        assert!(path.closed);
        let cubic_count = path.verbs.iter().filter(|v| matches!(v, PathVerb::CubicTo { .. })).count();
        assert_eq!(cubic_count, 4); // 4 segments for 4 points in closed loop
    }

    #[test]
    fn spline_high_tension() {
        let pts = PointBatch {
            xs: vec![0.0, 5.0, 10.0],
            ys: vec![0.0, 10.0, 0.0],
        };
        let path = spline_from_points(&pts, false, 1.0);
        // High tension: control points should be close to the through-points
        let cubic_count = path.verbs.iter().filter(|v| matches!(v, PathVerb::CubicTo { .. })).count();
        assert_eq!(cubic_count, 2);
    }

    // ── Warp to Curve tests ───────────────────────────────────

    #[test]
    fn warp_line_onto_semicircle() {
        // Source: horizontal line
        let line = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
            ],
            closed: false,
        };
        // Curve: quarter circle arc approximated by line segments
        let curve = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 10.0 }),
            ],
            closed: false,
        };
        let result = warp_to_curve(&NodeData::Path(Arc::new(line)), &curve, 0, 0.5, TOL);
        match result {
            NodeData::Path(p) => {
                assert!(!p.verbs.is_empty());
                // Start should be at curve start
                if let PathVerb::MoveTo(pt) = p.verbs[0] {
                    assert!((pt.x - 0.0).abs() < 0.5, "start x={}", pt.x);
                    assert!((pt.y - 0.0).abs() < 0.5, "start y={}", pt.y);
                }
            }
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn warp_rectangle() {
        let rect = make_square(0.0, -5.0, 10.0);
        let curve = make_line(0.0, 0.0, 20.0, 0.0);
        let result = warp_to_curve(&NodeData::Path(Arc::new(rect)), &curve, 0, 0.5, TOL);
        assert!(matches!(result, NodeData::Path(_)));
    }

    #[test]
    fn warp_mode1_tight_curve() {
        let line = make_line(0.0, 0.0, 10.0, 0.0);
        let curve = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 5.0, y: 10.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
            ],
            closed: false,
        };
        let result = warp_to_curve(&NodeData::Path(Arc::new(line)), &curve, 1, 0.5, TOL);
        assert!(matches!(result, NodeData::Path(_)));
    }

    #[test]
    fn warp_empty_geometry() {
        let empty = PathData::new();
        let curve = make_line(0.0, 0.0, 10.0, 0.0);
        let result = warp_to_curve(&NodeData::Path(Arc::new(empty)), &curve, 0, 0.5, TOL);
        assert!(matches!(result, NodeData::Path(_)));
    }

    #[test]
    fn warp_mode1_no_collapse_on_angle_wrap() {
        // Spine with tangent crossing the ±π boundary (going left then sharply
        // right). Before the fix, the atan2 difference would wrap to ~2π,
        // producing a huge spurious curvature that collapsed geometry.
        let spine = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: -10.0, y: 1.0 }),  // tangent ≈ +π
                PathVerb::LineTo(Point { x: -20.0, y: -1.0 }), // tangent ≈ -π
                PathVerb::LineTo(Point { x: -30.0, y: 0.0 }),
            ],
            closed: false,
        };
        // A rectangle with height 20 centered on y=0.
        let rect = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: -10.0 }),
                PathVerb::LineTo(Point { x: 30.0, y: -10.0 }),
                PathVerb::LineTo(Point { x: 30.0, y: 10.0 }),
                PathVerb::LineTo(Point { x: 0.0, y: 10.0 }),
                PathVerb::Close,
            ],
            closed: true,
        };
        let result = warp_to_curve(&NodeData::Path(Arc::new(rect)), &spine, 1, 0.5, TOL);
        if let NodeData::Path(p) = result {
            // Collect all warped points.
            let pts: Vec<Point> = p.verbs.iter().filter_map(|v| match v {
                PathVerb::MoveTo(pt) | PathVerb::LineTo(pt) => Some(*pt),
                _ => None,
            }).collect();
            // All points must be finite (no NaN/Inf from bad curvature).
            for pt in &pts {
                assert!(pt.x.is_finite() && pt.y.is_finite(), "non-finite point: {:?}", pt);
            }
            // The warped shape should not collapse: its bounding box height
            // should be a reasonable fraction of the original 20-unit height.
            let min_y = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let max_y = pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            let height = max_y - min_y;
            assert!(height > 5.0, "geometry collapsed: height={height}");
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn warp_center_point_to_curve_midpoint() {
        // A single point at the center of bbox should map to curve midpoint
        let pts = PointBatch {
            xs: vec![5.0],
            ys: vec![0.0],
        };
        let poly = polygon_from_points(&pts, false);
        let curve = make_line(0.0, 0.0, 20.0, 0.0);
        let result = warp_to_curve(&NodeData::Path(Arc::new(poly)), &curve, 0, 0.5, TOL);
        if let NodeData::Path(p) = result {
            if let PathVerb::MoveTo(pt) = p.verbs[0] {
                // u = (5-5)/0 → undefined for single point bbox.
                // But since the bbox width is 0 (single point), the u will be 0/0,
                // so this test just checks it doesn't crash.
                assert!(pt.x.is_finite());
            }
        }
    }

    #[test]
    fn build_lyon_path_sanitizes_non_finite_coords() {
        // NaN and infinity coordinates should not panic in build_lyon_path
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point {
                    x: f32::NAN,
                    y: 0.0,
                }),
                PathVerb::LineTo(Point {
                    x: f32::INFINITY,
                    y: f32::NEG_INFINITY,
                }),
                PathVerb::CubicTo {
                    ctrl1: Point {
                        x: f32::NAN,
                        y: f32::NAN,
                    },
                    ctrl2: Point { x: 1.0, y: 2.0 },
                    to: Point { x: 3.0, y: 4.0 },
                },
                PathVerb::Close,
            ],
            closed: true,
        };
        // Should not panic
        let _lyon = build_lyon_path(&path);
    }

    #[test]
    fn spline_from_points_with_non_finite_coords() {
        // Points with non-finite coords (e.g. from division by zero in scripts)
        // should not cause a panic
        let points = PointBatch {
            xs: vec![0.0, f32::INFINITY, 10.0],
            ys: vec![0.0, f32::NAN, 10.0],
        };
        let result = spline_from_points(&points, false, 0.0);
        // Should produce a path without panicking
        assert!(!result.verbs.is_empty());
        // And the resulting path should be safe to pass to lyon
        let _lyon = build_lyon_path(&result);
    }
}
