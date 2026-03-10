use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::{Affine2, Mat4};
use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};

use vector_flow_core::scheduler::EvalResult;
use vector_flow_core::types::{
    Color, ImageData, LineCap, LineJoin, NodeData, NodeId, PathData, PathVerb, Point, PointBatch,
    Shape, StrokeStyle, TextInstance,
};

use crate::vertex::{CanvasVertex, ImageVertex};

// ---------------------------------------------------------------------------
// Linear → sRGB conversion
// ---------------------------------------------------------------------------

/// Convert a single linear channel to sRGB.
fn linear_to_srgb_channel(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Convert an RGBA color from linear to sRGB (alpha is untouched).
fn linear_to_srgb(c: [f32; 4]) -> [f32; 4] {
    [
        linear_to_srgb_channel(c[0]),
        linear_to_srgb_channel(c[1]),
        linear_to_srgb_channel(c[2]),
        c[3],
    ]
}

// ---------------------------------------------------------------------------
// Collected shapes from evaluation
// ---------------------------------------------------------------------------

pub struct CollectedShape {
    pub shape: Shape,
    pub dimmed: bool,
}

pub struct CollectedImage {
    pub image: Arc<ImageData>,
    pub transform: Affine2,
    pub opacity: f32,
    pub dimmed: bool,
}

pub struct CollectedText {
    pub text: Arc<TextInstance>,
    pub dimmed: bool,
}

pub struct CollectedScene {
    pub shapes: Vec<CollectedShape>,
    pub images: Vec<CollectedImage>,
    pub texts: Vec<CollectedText>,
}

/// Extract renderable shapes from an EvalResult.
///
/// `visible_nodes`: if `Some`, only include shapes from those nodes (selection/lock).
/// If `None`, include all shapes from every node.
pub fn collect_shapes(
    eval_result: &EvalResult,
    visible_nodes: Option<&HashSet<NodeId>>,
) -> Vec<CollectedShape> {
    collect_scene(eval_result, visible_nodes).shapes
}

/// Radius (in world units) of point marker circles.
const POINT_MARKER_RADIUS: f32 = 3.0;

/// Color for point marker circles (semi-transparent light gray).
const POINT_MARKER_COLOR: Color = Color { r: 0.7, g: 0.7, b: 0.7, a: 0.8 };

/// Build a small circle path centered at (`cx`, `cy`) with the given radius.
fn circle_marker_path(cx: f32, cy: f32, r: f32) -> PathData {
    let k = r * 0.5522847; // kappa for cubic bezier circle approximation
    let mut path = PathData::new();
    path.verbs.push(PathVerb::MoveTo(Point { x: cx + r, y: cy }));
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx + r, y: cy + k },
        ctrl2: Point { x: cx + k, y: cy + r },
        to: Point { x: cx, y: cy + r },
    });
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx - k, y: cy + r },
        ctrl2: Point { x: cx - r, y: cy + k },
        to: Point { x: cx - r, y: cy },
    });
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx - r, y: cy - k },
        ctrl2: Point { x: cx - k, y: cy - r },
        to: Point { x: cx, y: cy - r },
    });
    path.verbs.push(PathVerb::CubicTo {
        ctrl1: Point { x: cx + k, y: cy - r },
        ctrl2: Point { x: cx + r, y: cy - k },
        to: Point { x: cx + r, y: cy },
    });
    path.verbs.push(PathVerb::Close);
    path.closed = true;
    path
}

/// Convert a point batch into marker shapes for preview rendering.
fn points_to_marker_shapes(points: &PointBatch) -> Vec<Shape> {
    let r = POINT_MARKER_RADIUS;
    let fill = POINT_MARKER_COLOR;
    points.xs.iter().zip(points.ys.iter()).map(|(&x, &y)| {
        Shape {
            path: Arc::new(circle_marker_path(x, y, r)),
            fill: Some(fill),
            stroke: None,
            transform: Affine2::IDENTITY,
        }
    }).collect()
}

/// Default stroke for raw path previews — ensures open/collinear paths are visible.
fn raw_path_preview_stroke() -> StrokeStyle {
    StrokeStyle {
        color: Color::WHITE,
        width: 1.5,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        dash_array: Vec::new(),
        dash_offset: 0.0,
        tolerance: 0.0,
    }
}

/// Collect a single NodeData item into the appropriate output lists.
/// Recursively unwraps `Mixed` bundles.
fn collect_node_data(
    data: &NodeData,
    dimmed: bool,
    shapes: &mut Vec<CollectedShape>,
    images: &mut Vec<CollectedImage>,
    texts: &mut Vec<CollectedText>,
) {
    match data {
        NodeData::Shape(s) => {
            shapes.push(CollectedShape {
                shape: (**s).clone(),
                dimmed,
            });
        }
        NodeData::Shapes(ss) => {
            for s in ss.iter() {
                shapes.push(CollectedShape {
                    shape: s.clone(),
                    dimmed,
                });
            }
        }
        NodeData::Path(p) => {
            shapes.push(CollectedShape {
                shape: Shape {
                    path: Arc::new((**p).clone()),
                    fill: Some(Color::WHITE),
                    stroke: Some(raw_path_preview_stroke()),
                    transform: Affine2::IDENTITY,
                },
                dimmed,
            });
        }
        NodeData::Paths(paths) => {
            for p in paths.iter() {
                shapes.push(CollectedShape {
                    shape: Shape {
                        path: Arc::new(p.clone()),
                        fill: Some(Color::WHITE),
                        stroke: Some(raw_path_preview_stroke()),
                        transform: Affine2::IDENTITY,
                    },
                    dimmed,
                });
            }
        }
        NodeData::Image(img) => {
            images.push(CollectedImage {
                image: Arc::clone(&img.image),
                transform: img.transform,
                opacity: img.opacity,
                dimmed,
            });
        }
        NodeData::Text(txt) => {
            texts.push(CollectedText {
                text: Arc::clone(txt),
                dimmed,
            });
        }
        NodeData::Points(pts) => {
            for marker in points_to_marker_shapes(pts) {
                shapes.push(CollectedShape {
                    shape: marker,
                    dimmed,
                });
            }
        }
        NodeData::Mixed(items) => {
            for item in items.iter() {
                collect_node_data(item, dimmed, shapes, images, texts);
            }
        }
        _ => {}
    }
}

/// Extract renderable shapes and images from an EvalResult.
pub fn collect_scene(
    eval_result: &EvalResult,
    visible_nodes: Option<&HashSet<NodeId>>,
) -> CollectedScene {
    collect_scene_ordered(eval_result, visible_nodes, None)
}

pub fn collect_scene_ordered(
    eval_result: &EvalResult,
    visible_nodes: Option<&HashSet<NodeId>>,
    node_order: Option<&HashMap<NodeId, i32>>,
) -> CollectedScene {
    // Collect node IDs to render, sorted by order if provided.
    let mut node_ids: Vec<NodeId> = eval_result
        .outputs
        .keys()
        .copied()
        .filter(|id| visible_nodes.is_none_or(|vis| vis.contains(id)))
        .collect();

    if let Some(order) = node_order {
        node_ids.sort_by_key(|id| order.get(id).copied().unwrap_or(0));
    }

    let mut shapes = Vec::new();
    let mut images = Vec::new();
    let mut texts = Vec::new();

    for node_id in &node_ids {
        if let Some(outputs) = eval_result.outputs.get(node_id) {
            let dimmed = false;
            for data in outputs {
                collect_node_data(data, dimmed, &mut shapes, &mut images, &mut texts);
            }
        }
    }

    CollectedScene { shapes, images, texts }
}

// ---------------------------------------------------------------------------
// Prepared scene for GPU upload
// ---------------------------------------------------------------------------

pub struct DrawBatch {
    pub vertex_offset: u32,
    pub index_offset: u32,
    pub index_count: u32,
    pub transform: Mat4,
    pub color: [f32; 4],
}

pub struct ImageDrawBatch {
    pub image: Arc<ImageData>,
    pub vertices: [ImageVertex; 4],
    pub indices: [u32; 6],
    pub transform: Mat4,
    pub color: [f32; 4], // tint: [1,1,1,opacity] or dimmed
}

pub struct PreparedScene {
    pub vertices: Vec<CanvasVertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<DrawBatch>,
    pub image_batches: Vec<ImageDrawBatch>,
}

impl PreparedScene {
    /// Compute the axis-aligned bounding box of all content in world space.
    /// Returns `None` if the scene is empty.
    /// Result is `(min, max)` as `(Vec2, Vec2)`.
    pub fn bounds(&self) -> Option<(glam::Vec2, glam::Vec2)> {
        if self.batches.is_empty() || self.vertices.is_empty() {
            return None;
        }

        let mut min = glam::Vec2::splat(f32::INFINITY);
        let mut max = glam::Vec2::splat(f32::NEG_INFINITY);

        for batch in &self.batches {
            let start = batch.index_offset as usize;
            let end = start + batch.index_count as usize;
            for &idx in &self.indices[start..end] {
                let v = &self.vertices[idx as usize];
                let local = glam::Vec4::new(v.position[0], v.position[1], 0.0, 1.0);
                let world = batch.transform * local;
                let p = glam::Vec2::new(world.x, world.y);
                min = min.min(p);
                max = max.max(p);
            }
        }

        Some((min, max))
    }
}

const DIMMED_TINT: [f32; 4] = [0.3, 0.3, 0.3, 0.5];
const DEFAULT_TOLERANCE: f32 = 0.5;

/// Build image draw batches from collected images.
fn prepare_image_batches(images: &[CollectedImage]) -> Vec<ImageDrawBatch> {
    images
        .iter()
        .filter(|ci| ci.image.width > 0 && ci.image.height > 0)
        .map(|ci| {
            let w = ci.image.width as f32;
            let h = ci.image.height as f32;
            let hw = w / 2.0;
            let hh = h / 2.0;

            // UV y is flipped: image pixels are top-to-bottom, but world Y+ is up.
            let vertices = [
                ImageVertex { position: [-hw, -hh], uv: [0.0, 1.0] },
                ImageVertex { position: [ hw, -hh], uv: [1.0, 1.0] },
                ImageVertex { position: [ hw,  hh], uv: [1.0, 0.0] },
                ImageVertex { position: [-hw,  hh], uv: [0.0, 0.0] },
            ];
            let indices = [0, 1, 2, 0, 2, 3];

            let transform = affine2_to_mat4(&ci.transform);
            let tint = if ci.dimmed {
                DIMMED_TINT
            } else {
                [1.0, 1.0, 1.0, ci.opacity]
            };

            ImageDrawBatch {
                image: Arc::clone(&ci.image),
                vertices,
                indices,
                transform,
                color: tint,
            }
        })
        .collect()
}

/// Prepare a full scene from shapes, images, and text.
/// `zoom` and `pixels_per_point` are used for text rasterization quality.
pub fn prepare_scene_full_with_text(
    scene: &CollectedScene,
    tolerance: f32,
    zoom: f32,
    pixels_per_point: f32,
) -> PreparedScene {
    let mut prepared = prepare_scene(&scene.shapes, tolerance);
    prepared.image_batches = prepare_image_batches(&scene.images);
    // Append text as image batches (rasterized at current zoom)
    let text_batches =
        crate::text_raster::prepare_text_batches(&scene.texts, zoom, pixels_per_point);
    prepared.image_batches.extend(text_batches);
    prepared
}

/// Prepare a full scene from shapes and images (no text rasterization).
pub fn prepare_scene_full(scene: &CollectedScene, tolerance: f32) -> PreparedScene {
    let mut prepared = prepare_scene(&scene.shapes, tolerance);
    prepared.image_batches = prepare_image_batches(&scene.images);
    prepared
}

struct SceneBuffers {
    vertices: Vec<CanvasVertex>,
    indices: Vec<u32>,
    batches: Vec<DrawBatch>,
}

/// Tessellate collected shapes into a GPU-ready scene.
pub fn prepare_scene(shapes: &[CollectedShape], tolerance: f32) -> PreparedScene {
    let tol = if tolerance <= 0.0 {
        DEFAULT_TOLERANCE
    } else {
        tolerance
    };

    let mut buf = SceneBuffers {
        vertices: Vec::new(),
        indices: Vec::new(),
        batches: Vec::new(),
    };

    for cs in shapes {
        let shape = &cs.shape;
        let transform = affine2_to_mat4(&shape.transform);
        let tint = if cs.dimmed {
            DIMMED_TINT
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };

        // Fill pass
        if let Some(fill_color) = shape.fill {
            tessellate_fill(&shape.path, fill_color, tol, transform, tint, &mut buf);
        }

        // Stroke pass — use per-shape tolerance if set, otherwise global.
        if let Some(ref stroke) = shape.stroke {
            let stroke_tol = if stroke.tolerance > 0.0 { stroke.tolerance } else { tol };
            tessellate_stroke(&shape.path, stroke, stroke_tol, transform, tint, &mut buf);
        }
    }

    PreparedScene {
        vertices: buf.vertices,
        indices: buf.indices,
        batches: buf.batches,
        image_batches: Vec::new(),
    }
}

fn tessellate_fill(
    path: &PathData,
    color: Color,
    tolerance: f32,
    transform: Mat4,
    tint: [f32; 4],
    buf: &mut SceneBuffers,
) {
    if path.verbs.is_empty() {
        return;
    }

    let lyon_path = build_lyon_path(path);
    let mut geometry: VertexBuffers<CanvasVertex, u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();

    let vertex_color = linear_to_srgb([color.r, color.g, color.b, color.a]);
    let result = tessellator.tessellate_path(
        &lyon_path,
        &FillOptions::tolerance(tolerance),
        &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| {
            let p = vertex.position();
            CanvasVertex {
                position: [p.x, p.y],
                color: vertex_color,
            }
        }),
    );

    if result.is_err() || geometry.indices.is_empty() {
        return;
    }

    push_batch(geometry, transform, linear_to_srgb(tint), buf);
}

fn make_stroke_options(stroke: &StrokeStyle, tolerance: f32) -> StrokeOptions {
    let mut options = StrokeOptions::tolerance(tolerance).with_line_width(stroke.width);
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
    options
}

fn tessellate_stroke(
    path: &PathData,
    stroke: &StrokeStyle,
    tolerance: f32,
    transform: Mat4,
    tint: [f32; 4],
    buf: &mut SceneBuffers,
) {
    if path.verbs.is_empty() || stroke.width <= 0.0 {
        return;
    }

    // If dash pattern is set, split into dashed sub-paths.
    if !stroke.dash_array.is_empty() {
        let dashed = apply_dash_pattern(path, &stroke.dash_array, stroke.dash_offset, tolerance);
        for sub_path in &dashed {
            tessellate_stroke_simple(sub_path, stroke, tolerance, transform, tint, buf);
        }
        return;
    }

    tessellate_stroke_simple(path, stroke, tolerance, transform, tint, buf);
}

fn tessellate_stroke_simple(
    path: &PathData,
    stroke: &StrokeStyle,
    tolerance: f32,
    transform: Mat4,
    tint: [f32; 4],
    buf: &mut SceneBuffers,
) {
    if path.verbs.is_empty() {
        return;
    }

    let lyon_path = build_lyon_path(path);
    let mut geometry: VertexBuffers<CanvasVertex, u32> = VertexBuffers::new();
    let mut tessellator = StrokeTessellator::new();
    let options = make_stroke_options(stroke, tolerance);

    let vertex_color = linear_to_srgb([stroke.color.r, stroke.color.g, stroke.color.b, stroke.color.a]);
    let result = tessellator.tessellate_path(
        &lyon_path,
        &options,
        &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex| {
            let p = vertex.position();
            CanvasVertex {
                position: [p.x, p.y],
                color: vertex_color,
            }
        }),
    );

    if result.is_err() || geometry.indices.is_empty() {
        return;
    }

    push_batch(geometry, transform, linear_to_srgb(tint), buf);
}

/// Apply a dash pattern to a path, returning dashed sub-paths.
/// Each sub-path in the input is dashed independently so that dashes don't
/// bridge across disjoint contours.
fn apply_dash_pattern(
    path: &PathData,
    dash_array: &[f32],
    dash_offset: f32,
    tolerance: f32,
) -> Vec<PathData> {
    use lyon::path::iterator::PathIterator;
    use vector_flow_core::types::Point;

    let total_pattern: f32 = dash_array.iter().sum();
    if total_pattern <= 0.0 {
        return vec![path.clone()];
    }

    let lyon_path = build_lyon_path(path);

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
            Event::Line { to, .. } => {
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

fn push_batch(
    geometry: VertexBuffers<CanvasVertex, u32>,
    transform: Mat4,
    color: [f32; 4],
    buf: &mut SceneBuffers,
) {
    let vertex_offset = buf.vertices.len() as u32;
    let index_offset = buf.indices.len() as u32;
    let index_count = geometry.indices.len() as u32;

    buf.vertices.extend_from_slice(&geometry.vertices);
    // Offset indices to account for shared vertex buffer
    buf.indices
        .extend(geometry.indices.iter().map(|i| i + vertex_offset));

    // Try to merge with previous batch if same transform and color
    if let Some(last) = buf.batches.last_mut() {
        if last.transform == transform
            && last.color == color
            && last.index_offset + last.index_count == index_offset
        {
            last.index_count += index_count;
            return;
        }
    }

    buf.batches.push(DrawBatch {
        vertex_offset: 0, // not used for draw_indexed — we bind the whole VBO
        index_offset,
        index_count,
        transform,
        color,
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn affine2_to_mat4(affine: &Affine2) -> Mat4 {
    let cols = affine.to_cols_array();
    // Affine2 is [a, b, c, d, tx, ty] — column-major 2x2 + translation
    // Map to 4x4 with z=identity
    Mat4::from_cols_array_2d(&[
        [cols[0], cols[1], 0.0, 0.0],
        [cols[2], cols[3], 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [cols[4], cols[5], 0.0, 1.0],
    ])
}

fn build_lyon_path(path: &PathData) -> Path {
    let mut builder = Path::builder();
    let mut in_subpath = false;

    for v in &path.verbs {
        match *v {
            PathVerb::MoveTo(p) => {
                if in_subpath {
                    builder.end(false);
                }
                builder.begin(point(p.x, p.y));
                in_subpath = true;
            }
            PathVerb::LineTo(p) => {
                if !in_subpath {
                    builder.begin(point(p.x, p.y));
                    in_subpath = true;
                } else {
                    builder.line_to(point(p.x, p.y));
                }
            }
            PathVerb::QuadTo { ctrl, to } => {
                if !in_subpath {
                    builder.begin(point(ctrl.x, ctrl.y));
                    in_subpath = true;
                }
                builder.quadratic_bezier_to(point(ctrl.x, ctrl.y), point(to.x, to.y));
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                if !in_subpath {
                    builder.begin(point(ctrl1.x, ctrl1.y));
                    in_subpath = true;
                }
                builder.cubic_bezier_to(
                    point(ctrl1.x, ctrl1.y),
                    point(ctrl2.x, ctrl2.y),
                    point(to.x, to.y),
                );
            }
            PathVerb::Close => {
                if in_subpath {
                    builder.end(true);
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use vector_flow_core::types::Point;

    fn square_path() -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: -50.0, y: -50.0 }),
                PathVerb::LineTo(Point { x: 50.0, y: -50.0 }),
                PathVerb::LineTo(Point { x: 50.0, y: 50.0 }),
                PathVerb::LineTo(Point { x: -50.0, y: 50.0 }),
                PathVerb::Close,
            ],
            closed: true,
        }
    }

    fn test_shape() -> Shape {
        Shape {
            path: Arc::new(square_path()),
            fill: Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
            stroke: None,
            transform: Affine2::IDENTITY,
        }
    }

    #[test]
    fn collect_shapes_from_eval_result() {
        let mut outputs = HashMap::new();
        outputs.insert(
            NodeId(1),
            vec![NodeData::Shape(Arc::new(test_shape()))],
        );
        let result = EvalResult { outputs, errors: HashMap::new() };

        let shapes = collect_shapes(&result, None);
        assert_eq!(shapes.len(), 1);
        assert!(!shapes[0].dimmed);
    }

    #[test]
    fn collect_shapes_filters_non_visible() {
        let mut outputs = HashMap::new();
        outputs.insert(
            NodeId(1),
            vec![NodeData::Shape(Arc::new(test_shape()))],
        );
        outputs.insert(
            NodeId(2),
            vec![NodeData::Shape(Arc::new(test_shape()))],
        );
        let result = EvalResult { outputs, errors: HashMap::new() };

        let mut visible = HashSet::new();
        visible.insert(NodeId(1));

        let shapes = collect_shapes(&result, Some(&visible));
        // Only the visible node's shape should be included; node 2 is excluded.
        assert_eq!(shapes.len(), 1);
    }

    #[test]
    fn collect_bare_paths() {
        let mut outputs = HashMap::new();
        outputs.insert(
            NodeId(1),
            vec![NodeData::Path(Arc::new(square_path()))],
        );
        let result = EvalResult { outputs, errors: HashMap::new() };

        let shapes = collect_shapes(&result, None);
        assert_eq!(shapes.len(), 1);
        assert!(shapes[0].shape.fill.is_some());
    }

    #[test]
    fn prepare_scene_produces_geometry() {
        let shapes = vec![CollectedShape {
            shape: test_shape(),
            dimmed: false,
        }];

        let scene = prepare_scene(&shapes, 0.5);
        assert!(!scene.vertices.is_empty());
        assert!(!scene.indices.is_empty());
        assert_eq!(scene.batches.len(), 1);
    }

    #[test]
    fn prepare_scene_merges_same_batches() {
        // Two shapes with same transform and same tint should merge
        let shapes = vec![
            CollectedShape {
                shape: Shape {
                    path: Arc::new(square_path()),
                    fill: Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
                    stroke: None,
                    transform: Affine2::IDENTITY,
                },
                dimmed: false,
            },
            CollectedShape {
                shape: Shape {
                    path: Arc::new(square_path()),
                    fill: Some(Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 }),
                    stroke: None,
                    transform: Affine2::IDENTITY,
                },
                dimmed: false,
            },
        ];

        let scene = prepare_scene(&shapes, 0.5);
        // Both have identity transform and [1,1,1,1] tint → should merge
        assert_eq!(scene.batches.len(), 1);
    }

    #[test]
    fn prepare_scene_splits_different_transform() {
        let shapes = vec![
            CollectedShape {
                shape: test_shape(),
                dimmed: false,
            },
            CollectedShape {
                shape: Shape {
                    path: Arc::new(square_path()),
                    fill: Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
                    stroke: None,
                    transform: Affine2::from_translation(glam::Vec2::new(100.0, 0.0)),
                },
                dimmed: false,
            },
        ];

        let scene = prepare_scene(&shapes, 0.5);
        assert_eq!(scene.batches.len(), 2);
    }

    #[test]
    fn affine_identity_to_mat4() {
        let mat = affine2_to_mat4(&Affine2::IDENTITY);
        assert_eq!(mat, Mat4::IDENTITY);
    }

    #[test]
    fn affine_translation_to_mat4() {
        let affine = Affine2::from_translation(glam::Vec2::new(10.0, 20.0));
        let mat = affine2_to_mat4(&affine);
        let expected = Mat4::from_translation(glam::Vec3::new(10.0, 20.0, 0.0));
        assert_eq!(mat, expected);
    }

    #[test]
    fn stroke_tessellation() {
        let shapes = vec![CollectedShape {
            shape: Shape {
                path: Arc::new(square_path()),
                fill: None,
                stroke: Some(StrokeStyle {
                    color: Color::WHITE,
                    width: 2.0,
                    line_cap: LineCap::Butt,
                    line_join: LineJoin::Miter(4.0),
                    dash_array: vec![],
                    dash_offset: 0.0,
                    tolerance: 0.0,
                }),
                transform: Affine2::IDENTITY,
            },
            dimmed: false,
        }];

        let scene = prepare_scene(&shapes, 0.5);
        assert!(!scene.vertices.is_empty());
        assert!(!scene.indices.is_empty());
    }

    #[test]
    fn empty_shapes_produce_empty_scene() {
        let scene = prepare_scene(&[], 0.5);
        assert!(scene.vertices.is_empty());
        assert!(scene.indices.is_empty());
        assert!(scene.batches.is_empty());
    }

    #[test]
    fn normal_shapes_get_full_tint() {
        let shapes = vec![CollectedShape {
            shape: test_shape(),
            dimmed: false,
        }];

        let scene = prepare_scene(&shapes, 0.5);
        assert_eq!(scene.batches.len(), 1);
        for (a, b) in scene.batches[0].color.iter().zip(&[1.0f32, 1.0, 1.0, 1.0]) {
            assert!((a - b).abs() < 1e-6, "expected ~{b}, got {a}");
        }
    }

    #[test]
    fn circle_marker_path_has_correct_structure() {
        let path = circle_marker_path(10.0, 20.0, 3.0);
        assert!(path.closed);
        // MoveTo + 4 CubicTo + Close = 6 verbs
        assert_eq!(path.verbs.len(), 6);
        assert!(matches!(path.verbs[0], PathVerb::MoveTo(_)));
        assert!(matches!(path.verbs[5], PathVerb::Close));
    }

    #[test]
    fn points_to_marker_shapes_count() {
        let pts = PointBatch {
            xs: vec![0.0, 10.0, 20.0],
            ys: vec![0.0, 10.0, 20.0],
        };
        let markers = points_to_marker_shapes(&pts);
        assert_eq!(markers.len(), 3);
        for m in &markers {
            assert!(m.fill.is_some());
            assert!(m.stroke.is_none());
        }
    }

    #[test]
    fn points_to_marker_shapes_empty() {
        let pts = PointBatch::new();
        let markers = points_to_marker_shapes(&pts);
        assert!(markers.is_empty());
    }

    #[test]
    fn collect_scene_includes_points() {
        let pts = PointBatch {
            xs: vec![0.0, 5.0],
            ys: vec![0.0, 5.0],
        };
        let mut outputs = HashMap::new();
        outputs.insert(NodeId(1), vec![NodeData::Points(Arc::new(pts))]);
        let result = EvalResult { outputs, errors: HashMap::new() };

        let scene = collect_scene(&result, None);
        assert_eq!(scene.shapes.len(), 2);
    }
}
