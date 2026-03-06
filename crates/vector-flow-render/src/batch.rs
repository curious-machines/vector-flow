use std::collections::HashSet;
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
    Color, ImageData, LineCap, LineJoin, NodeData, NodeId, PathData, PathVerb, Shape, StrokeStyle,
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

pub struct CollectedScene {
    pub shapes: Vec<CollectedShape>,
    pub images: Vec<CollectedImage>,
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

/// Extract renderable shapes and images from an EvalResult.
pub fn collect_scene(
    eval_result: &EvalResult,
    visible_nodes: Option<&HashSet<NodeId>>,
) -> CollectedScene {
    let mut shapes = Vec::new();
    let mut images = Vec::new();

    for (&node_id, outputs) in &eval_result.outputs {
        if let Some(vis) = visible_nodes {
            if !vis.contains(&node_id) {
                continue;
            }
        }
        let dimmed = false;

        for data in outputs {
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
                            path: (**p).clone(),
                            fill: Some(Color::WHITE),
                            stroke: None,
                            transform: Affine2::IDENTITY,
                        },
                        dimmed,
                    });
                }
                NodeData::Paths(paths) => {
                    for p in paths.iter() {
                        shapes.push(CollectedShape {
                            shape: Shape {
                                path: p.clone(),
                                fill: Some(Color::WHITE),
                                stroke: None,
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
                _ => {}
            }
        }
    }

    CollectedScene { shapes, images }
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

/// Prepare a full scene from shapes and images.
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

        // Stroke pass
        if let Some(ref stroke) = shape.stroke {
            tessellate_stroke(&shape.path, stroke, tol, transform, tint, &mut buf);
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

    let lyon_path = build_lyon_path(path);
    let mut geometry: VertexBuffers<CanvasVertex, u32> = VertexBuffers::new();
    let mut tessellator = StrokeTessellator::new();

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
                builder.end(true);
                in_subpath = false;
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
            path: square_path(),
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
                    path: square_path(),
                    fill: Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
                    stroke: None,
                    transform: Affine2::IDENTITY,
                },
                dimmed: false,
            },
            CollectedShape {
                shape: Shape {
                    path: square_path(),
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
                    path: square_path(),
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
                path: square_path(),
                fill: None,
                stroke: Some(StrokeStyle {
                    color: Color::WHITE,
                    width: 2.0,
                    line_cap: LineCap::Butt,
                    line_join: LineJoin::Miter(4.0),
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
}
