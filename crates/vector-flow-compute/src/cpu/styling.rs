use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::Affine2;
use lyon::math::point;
use lyon::path::iterator::PathIterator;
use lyon::path::Path as LyonPath;
use lyon::tessellation::{
    BuffersBuilder, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};

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

fn apply_dash_pattern(path: &PathData, dash_array: &[f32], dash_offset: f32) -> Vec<PathData> {
    if dash_array.is_empty() {
        return vec![path.clone()];
    }

    let lyon_path = build_lyon_path(path);
    let tolerance = 0.5;

    // Flatten path to line segments and compute cumulative distances.
    let mut segments: Vec<(Point, Point)> = Vec::new();
    let mut current = Point { x: 0.0, y: 0.0 };
    for evt in lyon_path.iter().flattened(tolerance) {
        use lyon::path::Event;
        match evt {
            Event::Begin { at } => {
                current = Point { x: at.x, y: at.y };
            }
            Event::Line { from: _, to } => {
                let to_pt = Point { x: to.x, y: to.y };
                segments.push((current, to_pt));
                current = to_pt;
            }
            Event::End { .. } => {}
            _ => {}
        }
    }

    if segments.is_empty() {
        return vec![path.clone()];
    }

    // Walk segments applying dash pattern.
    let total_pattern: f32 = dash_array.iter().sum();
    if total_pattern <= 0.0 {
        return vec![path.clone()];
    }

    let mut result: Vec<PathData> = Vec::new();
    let mut offset = dash_offset % total_pattern;
    if offset < 0.0 {
        offset += total_pattern;
    }

    // Find starting position in dash pattern.
    let mut dash_idx = 0usize;
    let mut dash_remaining = dash_array[0];
    let mut drawing = true; // even indices are dashes (drawn), odd are gaps

    // Consume offset.
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

    for (from, to) in &segments {
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
        let lyon_path = build_lyon_path(sub_path);
        let outline = tessellate_and_extract_boundary(&lyon_path, stroke);
        all_verbs.extend(outline.verbs);
    }

    PathData {
        verbs: all_verbs,
        closed: true,
    }
}

fn tessellate_and_extract_boundary(lyon_path: &LyonPath, stroke: &StrokeStyle) -> PathData {
    let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tessellator = StrokeTessellator::new();

    let mut options = StrokeOptions::tolerance(0.5).with_line_width(stroke.width);
    let cap = match stroke.line_cap {
        LineCap::Butt => lyon::tessellation::LineCap::Butt,
        LineCap::Round => lyon::tessellation::LineCap::Round,
        LineCap::Square => lyon::tessellation::LineCap::Square,
    };
    options.start_cap = cap;
    options.end_cap = cap;
    options.line_join = match stroke.line_join {
        LineJoin::Miter(limit) => {
            options.miter_limit = limit;
            lyon::tessellation::LineJoin::Miter
        }
        LineJoin::Round => lyon::tessellation::LineJoin::Round,
        LineJoin::Bevel => lyon::tessellation::LineJoin::Bevel,
    };

    let result = tessellator.tessellate_path(
        lyon_path,
        &options,
        &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex| {
            let p = vertex.position();
            [p.x, p.y]
        }),
    );

    if result.is_err() || geometry.indices.is_empty() {
        return PathData::new();
    }

    extract_boundary(&geometry.vertices, &geometry.indices)
}

/// Extract boundary edges from a triangle mesh and chain them into closed path loops.
fn extract_boundary(vertices: &[[f32; 2]], indices: &[u32]) -> PathData {
    // Count edge occurrences (boundary edges appear exactly once).
    let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let edges = [
            (tri[0].min(tri[1]), tri[0].max(tri[1])),
            (tri[1].min(tri[2]), tri[1].max(tri[2])),
            (tri[0].min(tri[2]), tri[0].max(tri[2])),
        ];
        for e in &edges {
            *edge_count.entry(*e).or_insert(0) += 1;
        }
    }

    // Build adjacency for boundary edges.
    let mut adjacency: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&(a, b), &count) in &edge_count {
        if count == 1 {
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
        }
    }

    // Chain boundary edges into loops.
    let mut visited_edges: HashSet<(u32, u32)> = HashSet::new();
    let mut verbs = Vec::new();

    let boundary_verts: Vec<u32> = adjacency.keys().copied().collect();
    for &start in &boundary_verts {
        let Some(neighbors) = adjacency.get(&start) else { continue };
        for &next in neighbors {
            let edge_key = (start.min(next), start.max(next));
            if !visited_edges.insert(edge_key) {
                continue;
            }

            // Walk a chain.
            let mut chain = vec![start];
            let mut curr = next;

            loop {
                chain.push(curr);
                let Some(neighbors) = adjacency.get(&curr) else { break };
                let mut found_next = false;
                for &n in neighbors {
                    let ek = (curr.min(n), curr.max(n));
                    if visited_edges.insert(ek) {
                        curr = n;
                        found_next = true;
                        break;
                    }
                }
                if !found_next || curr == start {
                    break;
                }
            }

            if chain.len() >= 3 {
                let v = vertices[chain[0] as usize];
                verbs.push(PathVerb::MoveTo(Point { x: v[0], y: v[1] }));
                for &idx in &chain[1..] {
                    let v = vertices[idx as usize];
                    verbs.push(PathVerb::LineTo(Point { x: v[0], y: v[1] }));
                }
                verbs.push(PathVerb::Close);
            }
        }
    }

    PathData {
        verbs,
        closed: true,
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
}
