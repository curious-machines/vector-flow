use std::sync::Arc;

use glam::Affine2;
use lyon::math::point;
use lyon::path::iterator::PathIterator;
use lyon::path::Path as LyonPath;

use vector_flow_core::types::{
    Color, LineCap, LineJoin, NodeData, PathData, PathVerb, Point, Shape, StrokeStyle,
};

/// Set fill color on geometry. Handles single and batch types.
pub fn set_fill(data: &NodeData, color: Color) -> NodeData {
    match data {
        NodeData::Shape(s) => {
            let mut shape = (**s).clone();
            shape.fill = Some(color);
            NodeData::Shape(Arc::new(shape))
        }
        NodeData::Shapes(shapes) => {
            let updated: Vec<Shape> = shapes
                .iter()
                .map(|s| {
                    let mut shape = s.clone();
                    shape.fill = Some(color);
                    shape
                })
                .collect();
            NodeData::Shapes(Arc::new(updated))
        }
        NodeData::Path(p) => NodeData::Shape(Arc::new(Shape {
            path: (**p).clone(),
            fill: Some(color),
            stroke: None,
            transform: Affine2::IDENTITY,
        })),
        NodeData::Paths(paths) => {
            let shapes: Vec<Shape> = paths
                .iter()
                .map(|p| Shape {
                    path: p.clone(),
                    fill: Some(color),
                    stroke: None,
                    transform: Affine2::IDENTITY,
                })
                .collect();
            NodeData::Shapes(Arc::new(shapes))
        }
        _ => NodeData::Shape(Arc::new(Shape {
            path: PathData::new(),
            fill: Some(color),
            stroke: None,
            transform: Affine2::IDENTITY,
        })),
    }
}

/// Set stroke style on geometry. Handles single and batch types.
pub fn set_stroke(
    data: &NodeData,
    color: Color,
    width: f64,
    cap: LineCap,
    join: LineJoin,
    dash_array: Vec<f32>,
    dash_offset: f32,
) -> NodeData {
    let stroke = StrokeStyle {
        color,
        width: width as f32,
        line_cap: cap,
        line_join: join,
        dash_array,
        dash_offset,
    };

    match data {
        NodeData::Shape(s) => {
            let mut shape = (**s).clone();
            shape.stroke = Some(stroke);
            NodeData::Shape(Arc::new(shape))
        }
        NodeData::Shapes(shapes) => {
            let updated: Vec<Shape> = shapes
                .iter()
                .map(|s| {
                    let mut shape = s.clone();
                    shape.stroke = Some(stroke.clone());
                    shape
                })
                .collect();
            NodeData::Shapes(Arc::new(updated))
        }
        NodeData::Path(p) => NodeData::Shape(Arc::new(Shape {
            path: (**p).clone(),
            fill: None,
            stroke: Some(stroke),
            transform: Affine2::IDENTITY,
        })),
        NodeData::Paths(paths) => {
            let shapes: Vec<Shape> = paths
                .iter()
                .map(|p| Shape {
                    path: p.clone(),
                    fill: None,
                    stroke: Some(stroke.clone()),
                    transform: Affine2::IDENTITY,
                })
                .collect();
            NodeData::Shapes(Arc::new(shapes))
        }
        _ => NodeData::Shape(Arc::new(Shape {
            path: PathData::new(),
            fill: None,
            stroke: Some(stroke),
            transform: Affine2::IDENTITY,
        })),
    }
}

/// Convert a stroke outline to a filled path (boundary extraction from tessellation).
pub fn stroke_to_path(data: &NodeData, stroke: &StrokeStyle) -> NodeData {
    let extract = |path: &PathData| -> PathData {
        stroke_outline(path, stroke)
    };

    match data {
        NodeData::Shape(s) => {
            let path = extract(&s.path);
            NodeData::Path(Arc::new(path))
        }
        NodeData::Shapes(shapes) => {
            let paths: Vec<PathData> = shapes.iter().map(|s| extract(&s.path)).collect();
            NodeData::Paths(Arc::new(paths))
        }
        NodeData::Path(p) => {
            let path = extract(p);
            NodeData::Path(Arc::new(path))
        }
        NodeData::Paths(paths) => {
            let result: Vec<PathData> = paths.iter().map(extract).collect();
            NodeData::Paths(Arc::new(result))
        }
        _ => NodeData::Path(Arc::new(PathData::new())),
    }
}

fn build_lyon_path(path: &PathData) -> LyonPath {
    let mut builder = LyonPath::builder();
    for verb in &path.verbs {
        match *verb {
            PathVerb::MoveTo(p) => {
                builder.begin(point(p.x, p.y));
            }
            PathVerb::LineTo(p) => {
                builder.line_to(point(p.x, p.y));
            }
            PathVerb::QuadTo { ctrl, to } => {
                builder.quadratic_bezier_to(point(ctrl.x, ctrl.y), point(to.x, to.y));
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                builder.cubic_bezier_to(
                    point(ctrl1.x, ctrl1.y),
                    point(ctrl2.x, ctrl2.y),
                    point(to.x, to.y),
                );
            }
            PathVerb::Close => {
                builder.close();
            }
        }
    }
    if !path.verbs.is_empty() && !matches!(path.verbs.last(), Some(PathVerb::Close)) {
        builder.end(false);
    }
    builder.build()
}

/// Apply a dash pattern to a path, returning dashed sub-paths.
/// Each contour in the input is dashed independently so that dashes don't
/// bridge across disjoint sub-paths.
fn apply_dash_pattern(path: &PathData, dash_array: &[f32], dash_offset: f32) -> Vec<PathData> {
    if dash_array.is_empty() {
        return vec![path.clone()];
    }

    let total_pattern: f32 = dash_array.iter().sum();
    if total_pattern <= 0.0 {
        return vec![path.clone()];
    }

    let lyon_path = build_lyon_path(path);
    let tolerance = 0.5;

    // Collect line segments grouped by sub-path.
    let mut sub_paths: Vec<Vec<(Point, Point)>> = Vec::new();
    let mut current_segments: Vec<(Point, Point)> = Vec::new();
    let mut current = Point { x: 0.0, y: 0.0 };
    let mut first = Point { x: 0.0, y: 0.0 };
    for evt in lyon_path.iter().flattened(tolerance) {
        use lyon::path::Event;
        match evt {
            Event::Begin { at } => {
                current = Point { x: at.x, y: at.y };
                first = current;
            }
            Event::Line { from: _, to } => {
                let to_pt = Point { x: to.x, y: to.y };
                current_segments.push((current, to_pt));
                current = to_pt;
            }
            Event::End { close, .. } => {
                // For closed sub-paths, add the closing segment back to the start.
                if close {
                    let dx = first.x - current.x;
                    let dy = first.y - current.y;
                    if dx * dx + dy * dy > 1e-6 {
                        current_segments.push((current, first));
                    }
                }
                if !current_segments.is_empty() {
                    sub_paths.push(std::mem::take(&mut current_segments));
                }
            }
            _ => {}
        }
    }
    if !current_segments.is_empty() {
        sub_paths.push(current_segments);
    }

    if sub_paths.is_empty() {
        return vec![path.clone()];
    }

    let mut result: Vec<PathData> = Vec::new();

    // Dash each sub-path independently, resetting dash state per contour.
    for segments in &sub_paths {
        let mut offset = dash_offset % total_pattern;
        if offset < 0.0 {
            offset += total_pattern;
        }

        let mut dash_idx = 0usize;
        let mut dash_remaining = dash_array[0];
        let mut drawing = true;

        let mut off = offset;
        while off > 0.0 {
            if off < dash_remaining {
                dash_remaining -= off;
                break;
            }
            off -= dash_remaining;
            drawing = !drawing;
            dash_idx = (dash_idx + 1) % dash_array.len();
            dash_remaining = dash_array[dash_idx];
        }

        let mut current_path = PathData::new();
        let mut needs_move = true;

        for (from, to) in segments {
            let dx = to.x - from.x;
            let dy = to.y - from.y;
            let seg_len = (dx * dx + dy * dy).sqrt();
            if seg_len < 1e-6 {
                continue;
            }

            let mut consumed = 0.0f32;
            while consumed < seg_len - 1e-6 {
                let remaining_seg = seg_len - consumed;
                let advance = remaining_seg.min(dash_remaining);
                let t_start = consumed / seg_len;
                let t_end = (consumed + advance) / seg_len;
                let start_pt = Point {
                    x: from.x + dx * t_start,
                    y: from.y + dy * t_start,
                };
                let end_pt = Point {
                    x: from.x + dx * t_end,
                    y: from.y + dy * t_end,
                };

                if drawing {
                    if needs_move {
                        current_path.verbs.push(PathVerb::MoveTo(start_pt));
                        needs_move = false;
                    }
                    current_path.verbs.push(PathVerb::LineTo(end_pt));
                }

                consumed += advance;
                dash_remaining -= advance;

                if dash_remaining < 1e-6 {
                    if drawing && !current_path.verbs.is_empty() {
                        result.push(current_path);
                        current_path = PathData::new();
                        needs_move = true;
                    }
                    drawing = !drawing;
                    dash_idx = (dash_idx + 1) % dash_array.len();
                    dash_remaining = dash_array[dash_idx];
                    if drawing {
                        needs_move = true;
                    }
                }
            }
        }

        if !current_path.verbs.is_empty() {
            result.push(current_path);
        }
    }

    if result.is_empty() {
        result.push(PathData::new());
    }
    result
}

fn stroke_outline(path: &PathData, stroke: &StrokeStyle) -> PathData {
    if path.verbs.is_empty() || stroke.width <= 0.0 {
        return PathData::new();
    }

    // Apply dashing first.
    let sub_paths = apply_dash_pattern(path, &stroke.dash_array, stroke.dash_offset);

    let mut all_verbs: Vec<PathVerb> = Vec::new();

    for sub_path in &sub_paths {
        if sub_path.verbs.is_empty() {
            continue;
        }
        let outline = polyline_stroke_outline(sub_path, stroke);
        all_verbs.extend(outline.verbs);
    }

    PathData {
        verbs: all_verbs,
        closed: true,
    }
}

/// Flatten a PathData into a list of polyline contours (each is a Vec of points + closed flag).
fn flatten_to_polylines(path: &PathData) -> Vec<(Vec<Point>, bool)> {
    let lyon_path = build_lyon_path(path);
    let tolerance = 0.5;

    let mut contours: Vec<(Vec<Point>, bool)> = Vec::new();
    let mut current_pts: Vec<Point> = Vec::new();

    for evt in lyon_path.iter().flattened(tolerance) {
        use lyon::path::Event;
        match evt {
            Event::Begin { at } => {
                current_pts.clear();
                current_pts.push(Point { x: at.x, y: at.y });
            }
            Event::Line { to, .. } => {
                let p = Point { x: to.x, y: to.y };
                // Avoid duplicate consecutive points.
                if let Some(last) = current_pts.last() {
                    if (last.x - p.x).abs() > 1e-6 || (last.y - p.y).abs() > 1e-6 {
                        current_pts.push(p);
                    }
                }
            }
            Event::End { close, .. } => {
                if current_pts.len() >= 2 {
                    contours.push((std::mem::take(&mut current_pts), close));
                }
            }
            _ => {}
        }
    }
    if current_pts.len() >= 2 {
        contours.push((current_pts, false));
    }
    contours
}

/// Build the stroke outline of a path directly using polyline offset.
/// Much more robust than tessellation + boundary extraction.
fn polyline_stroke_outline(path: &PathData, stroke: &StrokeStyle) -> PathData {
    let contours = flatten_to_polylines(path);
    let mut all_verbs: Vec<PathVerb> = Vec::new();

    for (pts, closed) in &contours {
        if pts.len() < 2 {
            continue;
        }
        let verbs = if *closed {
            outline_closed(pts, stroke)
        } else {
            outline_open(pts, stroke)
        };
        all_verbs.extend(verbs);
    }

    PathData {
        verbs: all_verbs,
        closed: true,
    }
}

/// Compute per-segment unit normals (pointing left of the direction of travel).
fn segment_normals(pts: &[Point]) -> Vec<(f32, f32)> {
    let mut normals = Vec::with_capacity(pts.len() - 1);
    for i in 0..pts.len() - 1 {
        let dx = pts[i + 1].x - pts[i].x;
        let dy = pts[i + 1].y - pts[i].y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            normals.push((0.0, 1.0));
        } else {
            normals.push((-dy / len, dx / len));
        }
    }
    normals
}

/// Compute the miter offset point at a joint between two segments.
/// Returns (left_offset, right_offset) points at position `p` given
/// incoming normal `n0` and outgoing normal `n1`.
fn miter_offset(
    p: Point,
    n0: (f32, f32),
    n1: (f32, f32),
    half_w: f32,
    join: &LineJoin,
) -> (Vec<Point>, Vec<Point>) {
    let mx = n0.0 + n1.0;
    let my = n0.1 + n1.1;
    let mlen = (mx * mx + my * my).sqrt();

    if mlen < 1e-6 {
        // Near-parallel segments — use either normal.
        return (
            vec![Point { x: p.x + n0.0 * half_w, y: p.y + n0.1 * half_w }],
            vec![Point { x: p.x - n0.0 * half_w, y: p.y - n0.1 * half_w }],
        );
    }

    let nmx = mx / mlen;
    let nmy = my / mlen;
    let cos_half = n0.0 * nmx + n0.1 * nmy;
    let miter_len = if cos_half.abs() > 1e-6 {
        half_w / cos_half
    } else {
        half_w
    };

    match join {
        LineJoin::Miter(limit) => {
            if miter_len.abs() / half_w > *limit {
                // Exceed miter limit → bevel.
                bevel_points(p, n0, n1, half_w)
            } else {
                (
                    vec![Point { x: p.x + nmx * miter_len, y: p.y + nmy * miter_len }],
                    vec![Point { x: p.x - nmx * miter_len, y: p.y - nmy * miter_len }],
                )
            }
        }
        LineJoin::Bevel => bevel_points(p, n0, n1, half_w),
        LineJoin::Round => {
            // Determine which side has the outer angle (needs the arc).
            let cross = n0.0 * n1.1 - n0.1 * n1.0;
            if cross.abs() < 1e-6 {
                // Nearly straight.
                return (
                    vec![Point { x: p.x + nmx * miter_len, y: p.y + nmy * miter_len }],
                    vec![Point { x: p.x - nmx * miter_len, y: p.y - nmy * miter_len }],
                );
            }
            let arc_pts = round_join_arc(p, n0, n1, half_w, cross > 0.0);
            if cross > 0.0 {
                // Left side is outer — gets the arc, right side gets miter.
                (
                    arc_pts,
                    vec![Point { x: p.x - nmx * miter_len, y: p.y - nmy * miter_len }],
                )
            } else {
                // Right side is outer.
                (
                    vec![Point { x: p.x + nmx * miter_len, y: p.y + nmy * miter_len }],
                    arc_pts.into_iter().rev().map(|pt| {
                        // Mirror across the center.
                        Point { x: 2.0 * p.x - pt.x, y: 2.0 * p.y - pt.y }
                    }).collect(),
                )
            }
        }
    }
}

fn bevel_points(
    p: Point,
    n0: (f32, f32),
    n1: (f32, f32),
    half_w: f32,
) -> (Vec<Point>, Vec<Point>) {
    (
        vec![
            Point { x: p.x + n0.0 * half_w, y: p.y + n0.1 * half_w },
            Point { x: p.x + n1.0 * half_w, y: p.y + n1.1 * half_w },
        ],
        vec![
            Point { x: p.x - n0.0 * half_w, y: p.y - n0.1 * half_w },
            Point { x: p.x - n1.0 * half_w, y: p.y - n1.1 * half_w },
        ],
    )
}

/// Generate arc points for a round join on the outer side.
fn round_join_arc(
    center: Point,
    n0: (f32, f32),
    n1: (f32, f32),
    radius: f32,
    left_is_outer: bool,
) -> Vec<Point> {
    let (sn, en) = if left_is_outer { (n0, n1) } else { (n0, n1) };
    let start_angle = sn.1.atan2(sn.0);
    let end_angle = en.1.atan2(en.0);

    let mut sweep = end_angle - start_angle;
    // Normalize sweep to the shorter arc.
    if sweep > std::f32::consts::PI {
        sweep -= 2.0 * std::f32::consts::PI;
    } else if sweep < -std::f32::consts::PI {
        sweep += 2.0 * std::f32::consts::PI;
    }

    let steps = ((sweep.abs() / (std::f32::consts::PI / 8.0)).ceil() as usize).max(2);
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let angle = start_angle + sweep * t;
        pts.push(Point {
            x: center.x + angle.cos() * radius,
            y: center.y + angle.sin() * radius,
        });
    }
    pts
}

/// Build outline for an OPEN polyline (with end caps).
fn outline_open(pts: &[Point], stroke: &StrokeStyle) -> Vec<PathVerb> {
    let half_w = stroke.width / 2.0;
    let normals = segment_normals(pts);

    let mut left: Vec<Point> = Vec::new();
    let mut right: Vec<Point> = Vec::new();

    // Start point.
    let n = normals[0];
    left.push(Point { x: pts[0].x + n.0 * half_w, y: pts[0].y + n.1 * half_w });
    right.push(Point { x: pts[0].x - n.0 * half_w, y: pts[0].y - n.1 * half_w });

    // Interior joints.
    for i in 1..pts.len() - 1 {
        let (l, r) = miter_offset(pts[i], normals[i - 1], normals[i], half_w, &stroke.line_join);
        left.extend(l);
        right.extend(r);
    }

    // End point.
    let n = *normals.last().unwrap();
    let last = *pts.last().unwrap();
    left.push(Point { x: last.x + n.0 * half_w, y: last.y + n.1 * half_w });
    right.push(Point { x: last.x - n.0 * half_w, y: last.y - n.1 * half_w });

    // Build the outline: left forward → end cap → right backward → start cap → close.
    let mut verbs = Vec::new();
    verbs.push(PathVerb::MoveTo(left[0]));
    for p in &left[1..] {
        verbs.push(PathVerb::LineTo(*p));
    }

    // End cap.
    add_cap(&mut verbs, last, n, half_w, stroke.line_cap, true);

    // Right side in reverse.
    for p in right.iter().rev() {
        verbs.push(PathVerb::LineTo(*p));
    }

    // Start cap.
    add_cap(&mut verbs, pts[0], normals[0], half_w, stroke.line_cap, false);

    verbs.push(PathVerb::Close);
    verbs
}

/// Build outline for a CLOSED polyline (no end caps, just joins).
fn outline_closed(pts: &[Point], stroke: &StrokeStyle) -> Vec<PathVerb> {
    let half_w = stroke.width / 2.0;

    // For closed paths, the last point may equal the first. Build segments
    // treating the path as a loop.
    let n = pts.len();
    if n < 2 {
        return vec![];
    }

    // Build normals for each segment in the loop.
    let mut normals: Vec<(f32, f32)> = Vec::with_capacity(n);
    for i in 0..n {
        let next = (i + 1) % n;
        let dx = pts[next].x - pts[i].x;
        let dy = pts[next].y - pts[i].y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            normals.push((0.0, 1.0));
        } else {
            normals.push((-dy / len, dx / len));
        }
    }

    let mut left: Vec<Point> = Vec::new();
    let mut right: Vec<Point> = Vec::new();

    for i in 0..n {
        let prev_n = normals[(i + n - 1) % n];
        let curr_n = normals[i];
        let (l, r) = miter_offset(pts[i], prev_n, curr_n, half_w, &stroke.line_join);
        left.extend(l);
        right.extend(r);
    }

    let mut verbs = Vec::new();

    // Left side as one closed loop.
    if !left.is_empty() {
        verbs.push(PathVerb::MoveTo(left[0]));
        for p in &left[1..] {
            verbs.push(PathVerb::LineTo(*p));
        }
        verbs.push(PathVerb::Close);
    }

    // Right side as another closed loop (reversed winding).
    if !right.is_empty() {
        verbs.push(PathVerb::MoveTo(*right.last().unwrap()));
        for p in right.iter().rev().skip(1) {
            verbs.push(PathVerb::LineTo(*p));
        }
        verbs.push(PathVerb::Close);
    }

    verbs
}

/// Add an end cap to the outline path.
fn add_cap(
    verbs: &mut Vec<PathVerb>,
    tip: Point,
    normal: (f32, f32),
    half_w: f32,
    cap: LineCap,
    is_end: bool,
) {
    match cap {
        LineCap::Butt => {
            // No extra geometry — the left→right connection is the butt cap.
        }
        LineCap::Square => {
            // Extend by half_w in the direction of travel.
            let dir = if is_end { (normal.1, -normal.0) } else { (-normal.1, normal.0) };
            let ext = Point {
                x: tip.x + dir.0 * half_w,
                y: tip.y + dir.1 * half_w,
            };
            // Two corner points of the square extension.
            let c1 = Point {
                x: ext.x + normal.0 * half_w,
                y: ext.y + normal.1 * half_w,
            };
            let c2 = Point {
                x: ext.x - normal.0 * half_w,
                y: ext.y - normal.1 * half_w,
            };
            if is_end {
                verbs.push(PathVerb::LineTo(c1));
                verbs.push(PathVerb::LineTo(c2));
            } else {
                verbs.push(PathVerb::LineTo(c2));
                verbs.push(PathVerb::LineTo(c1));
            }
        }
        LineCap::Round => {
            // Semicircle approximation.
            let dir = if is_end { 1.0f32 } else { -1.0 };
            let start_angle = normal.1.atan2(normal.0);
            let steps = 8;
            for i in 1..steps {
                let t = i as f32 / steps as f32;
                let angle = start_angle + dir * std::f32::consts::PI * t;
                verbs.push(PathVerb::LineTo(Point {
                    x: tip.x + angle.cos() * half_w,
                    y: tip.y + angle.sin() * half_w,
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::types::{PathVerb, Point};

    #[test]
    fn set_fill_on_path() {
        let path = PathData {
            verbs: vec![PathVerb::MoveTo(Point { x: 0.0, y: 0.0 })],
            closed: false,
        };
        let data = NodeData::Path(Arc::new(path));
        let result = set_fill(&data, Color::WHITE);
        if let NodeData::Shape(s) = result {
            assert_eq!(s.fill, Some(Color::WHITE));
            assert!(s.stroke.is_none());
        } else {
            panic!("expected Shape");
        }
    }

    #[test]
    fn set_stroke_on_shape() {
        let shape = Shape {
            path: PathData::new(),
            fill: Some(Color::BLACK),
            stroke: None,
            transform: Affine2::IDENTITY,
        };
        let data = NodeData::Shape(Arc::new(shape));
        let result = set_stroke(&data, Color::WHITE, 3.0, LineCap::Butt, LineJoin::Miter(4.0), vec![], 0.0);
        if let NodeData::Shape(s) = result {
            assert_eq!(s.fill, Some(Color::BLACK)); // preserved
            let st = s.stroke.clone().unwrap();
            assert_eq!(st.color, Color::WHITE);
            assert!((st.width - 3.0).abs() < 1e-5);
        } else {
            panic!("expected Shape");
        }
    }

    #[test]
    fn set_fill_on_shapes_batch() {
        let shapes = vec![
            Shape {
                path: PathData::new(),
                fill: None,
                stroke: None,
                transform: Affine2::IDENTITY,
            },
            Shape {
                path: PathData::new(),
                fill: None,
                stroke: None,
                transform: Affine2::IDENTITY,
            },
        ];
        let data = NodeData::Shapes(Arc::new(shapes));
        let result = set_fill(&data, Color::WHITE);
        if let NodeData::Shapes(s) = result {
            assert_eq!(s.len(), 2);
            assert!(s.iter().all(|s| s.fill == Some(Color::WHITE)));
        } else {
            panic!("expected Shapes batch");
        }
    }

    #[test]
    fn set_stroke_on_shapes_batch() {
        let shapes = vec![
            Shape {
                path: PathData::new(),
                fill: Some(Color::BLACK),
                stroke: None,
                transform: Affine2::IDENTITY,
            },
        ];
        let data = NodeData::Shapes(Arc::new(shapes));
        let result = set_stroke(&data, Color::WHITE, 2.0, LineCap::Butt, LineJoin::Miter(4.0), vec![], 0.0);
        if let NodeData::Shapes(s) = result {
            assert_eq!(s.len(), 1);
            assert_eq!(s[0].fill, Some(Color::BLACK)); // preserved
            assert!(s[0].stroke.is_some());
        } else {
            panic!("expected Shapes batch");
        }
    }

    fn make_square_path() -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 100.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 100.0, y: 100.0 }),
                PathVerb::LineTo(Point { x: 0.0, y: 100.0 }),
                PathVerb::Close,
            ],
            closed: true,
        }
    }

    #[test]
    fn set_stroke_with_cap_and_join() {
        let path = make_square_path();
        let data = NodeData::Path(Arc::new(path));
        let result = set_stroke(
            &data,
            Color::WHITE,
            5.0,
            LineCap::Round,
            LineJoin::Bevel,
            vec![],
            0.0,
        );
        if let NodeData::Shape(s) = result {
            let st = s.stroke.clone().unwrap();
            assert_eq!(st.line_cap, LineCap::Round);
            assert_eq!(st.line_join, LineJoin::Bevel);
            assert!(st.dash_array.is_empty());
        } else {
            panic!("expected Shape");
        }
    }

    #[test]
    fn set_stroke_with_dash_pattern() {
        let path = make_square_path();
        let data = NodeData::Path(Arc::new(path));
        let result = set_stroke(
            &data,
            Color::BLACK,
            2.0,
            LineCap::Butt,
            LineJoin::Miter(4.0),
            vec![10.0, 5.0],
            0.0,
        );
        if let NodeData::Shape(s) = result {
            let st = s.stroke.clone().unwrap();
            assert_eq!(st.dash_array, vec![10.0, 5.0]);
            assert_eq!(st.dash_offset, 0.0);
        } else {
            panic!("expected Shape");
        }
    }

    #[test]
    fn stroke_to_path_produces_path() {
        let path = make_square_path();
        let data = NodeData::Path(Arc::new(path));
        let stroke = StrokeStyle {
            color: Color::BLACK,
            width: 4.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter(4.0),
            dash_array: vec![],
            dash_offset: 0.0,
        };
        let result = stroke_to_path(&data, &stroke);
        if let NodeData::Path(p) = result {
            assert!(!p.verbs.is_empty(), "stroke outline should have verbs");
            // Should have at least one MoveTo and Close
            assert!(p.verbs.iter().any(|v| matches!(v, PathVerb::MoveTo(_))));
            assert!(p.verbs.iter().any(|v| matches!(v, PathVerb::Close)));
        } else {
            panic!("expected Path, got {:?}", result.data_type());
        }
    }

    #[test]
    fn stroke_to_path_with_dashes() {
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 100.0, y: 0.0 }),
            ],
            closed: false,
        };
        let data = NodeData::Path(Arc::new(path));
        let stroke = StrokeStyle {
            color: Color::BLACK,
            width: 4.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter(4.0),
            dash_array: vec![20.0, 10.0],
            dash_offset: 0.0,
        };
        let result = stroke_to_path(&data, &stroke);
        if let NodeData::Path(p) = result {
            assert!(!p.verbs.is_empty(), "dashed stroke outline should have verbs");
            // With 100px line and 20/10 pattern, expect multiple MoveTo (multiple dashes)
            let move_count = p.verbs.iter().filter(|v| matches!(v, PathVerb::MoveTo(_))).count();
            assert!(move_count >= 2, "dashed stroke should produce multiple sub-paths, got {move_count}");
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn apply_dash_pattern_splits_path() {
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 100.0, y: 0.0 }),
            ],
            closed: false,
        };
        let dashes = apply_dash_pattern(&path, &[20.0, 10.0], 0.0);
        // 100px line, pattern=30px cycle → dashes at 0-20, 30-50, 60-80, 90-100
        assert!(dashes.len() >= 3, "expected at least 3 dash segments, got {}", dashes.len());
    }

    #[test]
    fn apply_dash_pattern_empty_returns_original() {
        let path = make_square_path();
        let dashes = apply_dash_pattern(&path, &[], 0.0);
        assert_eq!(dashes.len(), 1);
        assert_eq!(dashes[0], path);
    }

    #[test]
    fn stroke_to_path_is_deterministic() {
        let path = make_square_path();
        let data = NodeData::Path(Arc::new(path));
        let stroke = StrokeStyle {
            color: Color::BLACK,
            width: 4.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter(4.0),
            dash_array: vec![],
            dash_offset: 0.0,
        };
        let result1 = stroke_to_path(&data, &stroke);
        let result2 = stroke_to_path(&data, &stroke);
        if let (NodeData::Path(p1), NodeData::Path(p2)) = (&result1, &result2) {
            assert_eq!(p1.verbs.len(), p2.verbs.len(), "verb count differs across calls");
            for (i, (v1, v2)) in p1.verbs.iter().zip(p2.verbs.iter()).enumerate() {
                assert_eq!(v1, v2, "verb {i} differs across calls");
            }
        } else {
            panic!("expected Path outputs");
        }
    }

    #[test]
    fn stroke_to_path_with_dashes_is_deterministic() {
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 100.0, y: 0.0 }),
            ],
            closed: false,
        };
        let data = NodeData::Path(Arc::new(path));
        let stroke = StrokeStyle {
            color: Color::BLACK,
            width: 20.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter(4.0),
            dash_array: vec![12.0],
            dash_offset: 0.0,
        };
        let result1 = stroke_to_path(&data, &stroke);
        let result2 = stroke_to_path(&data, &stroke);
        if let (NodeData::Path(p1), NodeData::Path(p2)) = (&result1, &result2) {
            assert_eq!(p1.verbs, p2.verbs, "dashed stroke outline differs across calls");
        } else {
            panic!("expected Path outputs");
        }
    }
}
