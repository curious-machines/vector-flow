mod generators;
mod path_ops;
mod styling;
mod tessellation;
mod transforms;
mod utility;

use std::sync::Arc;

use glam::{Affine2, Vec2};
use parking_lot::Mutex;

use vector_flow_core::compute::{
    ComputeBackend, DslContext, NodeOutputs, ResolvedInputs, TessellationOutput,
};
use vector_flow_core::error::ComputeError;
use vector_flow_core::node::NodeOp;
use vector_flow_core::types::{Color, NodeData, PathData, PointBatch, Shape, TimeContext};

use vector_flow_dsl::cache::DslFunctionCache;
use vector_flow_dsl::codegen::{DslCompiler, ExprFnPtr};

/// CPU-based compute backend.
pub struct CpuBackend {
    dsl_compiler: Mutex<DslCompiler>,
    dsl_cache: DslFunctionCache,
}

impl CpuBackend {
    /// Create a new CPU backend with a fresh DSL compiler.
    pub fn new() -> Result<Self, ComputeError> {
        let compiler =
            DslCompiler::new().map_err(|e| ComputeError::BackendError(e.to_string()))?;
        Ok(Self {
            dsl_compiler: Mutex::new(compiler),
            dsl_cache: DslFunctionCache::new(),
        })
    }
}

impl ComputeBackend for CpuBackend {
    fn evaluate_node(
        &self,
        op: &NodeOp,
        inputs: &ResolvedInputs,
        time_ctx: &TimeContext,
        outputs: &mut NodeOutputs,
    ) -> Result<(), ComputeError> {
        let result = match op {
            // ── Generators ──────────────────────────────────────────
            NodeOp::RegularPolygon => {
                let sides = get_int(inputs, 0);
                let radius = get_scalar(inputs, 1);
                if let Some(pts) = get_points_batch(inputs, 2) {
                    batch_generate(&pts, |c| generators::regular_polygon(sides, radius, c))
                } else {
                    generators::regular_polygon(sides, radius, get_vec2(inputs, 2))
                }
            }
            NodeOp::Circle => {
                let radius = get_scalar(inputs, 0);
                let segments = get_int(inputs, 2);
                if let Some(pts) = get_points_batch(inputs, 1) {
                    batch_generate(&pts, |c| generators::circle(radius, c, segments))
                } else {
                    generators::circle(radius, get_vec2(inputs, 1), segments)
                }
            }
            NodeOp::Rectangle => {
                let width = get_scalar(inputs, 0);
                let height = get_scalar(inputs, 1);
                if let Some(pts) = get_points_batch(inputs, 2) {
                    batch_generate(&pts, |c| generators::rectangle(width, height, c))
                } else {
                    generators::rectangle(width, height, get_vec2(inputs, 2))
                }
            }
            NodeOp::Line => {
                let from = get_vec2(inputs, 0);
                let to = get_vec2(inputs, 1);
                generators::line(from, to)
            }
            NodeOp::PointGrid => {
                let cols = get_int(inputs, 0);
                let rows = get_int(inputs, 1);
                let spacing = get_scalar(inputs, 2);
                generators::point_grid(cols, rows, spacing)
            }
            NodeOp::ScatterPoints => {
                let count = get_int(inputs, 0);
                let width = get_scalar(inputs, 1);
                let height = get_scalar(inputs, 2);
                let seed = get_int(inputs, 3);
                generators::scatter_points(count, width, height, seed)
            }

            // ── Transforms ──────────────────────────────────────────
            NodeOp::Translate => {
                let geometry = get_any(inputs, 0);
                let offset = get_vec2(inputs, 1);
                transforms::translate(&geometry, offset)
            }
            NodeOp::Rotate => {
                let geometry = get_any(inputs, 0);
                let angle = get_scalar(inputs, 1);
                let center = get_vec2(inputs, 2);
                transforms::rotate(&geometry, angle, center)
            }
            NodeOp::Scale => {
                let geometry = get_any(inputs, 0);
                let factor = get_vec2(inputs, 1);
                let center = get_vec2(inputs, 2);
                transforms::scale(&geometry, factor, center)
            }
            NodeOp::ApplyTransform => {
                let geometry = get_any(inputs, 0);
                let xform = get_transform(inputs, 1);
                transforms::apply_transform(&geometry, &xform)
            }

            // ── Path ops ────────────────────────────────────────────
            NodeOp::PathReverse => {
                let path = get_path(inputs, 0);
                NodeData::Path(Arc::new(path_ops::path_reverse(&path)))
            }
            NodeOp::PathSubdivide => {
                let path = get_path(inputs, 0);
                let levels = get_int(inputs, 1);
                NodeData::Path(Arc::new(path_ops::path_subdivide(&path, levels)))
            }
            NodeOp::ResamplePath => {
                let path = get_path(inputs, 0);
                let count = get_int(inputs, 1);
                path_ops::resample_path(&path, count)
            }
            NodeOp::PathOffset => {
                let path = get_path(inputs, 0);
                let distance = get_scalar(inputs, 1);
                NodeData::Path(Arc::new(path_ops::path_offset(&path, distance)))
            }
            NodeOp::PathUnion => {
                // Collect all non-empty inputs as shapes, preserving fill/stroke.
                let mut all_shapes: Vec<Shape> = Vec::new();
                for i in 0..inputs.data.len() {
                    match inputs.data.get(i) {
                        Some(NodeData::Shape(s)) if !s.path.verbs.is_empty() => {
                            all_shapes.push((**s).clone());
                        }
                        Some(NodeData::Shapes(shapes)) => {
                            for s in shapes.iter() {
                                if !s.path.verbs.is_empty() {
                                    all_shapes.push(s.clone());
                                }
                            }
                        }
                        Some(NodeData::Path(p)) if !p.verbs.is_empty() => {
                            all_shapes.push(Shape {
                                path: (**p).clone(),
                                fill: None,
                                stroke: None,
                                transform: Affine2::IDENTITY,
                            });
                        }
                        Some(NodeData::Paths(paths)) => {
                            for p in paths.iter() {
                                if !p.verbs.is_empty() {
                                    all_shapes.push(Shape {
                                        path: p.clone(),
                                        fill: None,
                                        stroke: None,
                                        transform: Affine2::IDENTITY,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                NodeData::Shapes(Arc::new(all_shapes))
            }
            NodeOp::PathIntersect => {
                let a = get_path(inputs, 0);
                let b = get_path(inputs, 1);
                NodeData::Path(Arc::new(path_ops::path_intersect(&a, &b)))
            }
            NodeOp::PathDifference => {
                let a = get_path(inputs, 0);
                let b = get_path(inputs, 1);
                NodeData::Path(Arc::new(path_ops::path_difference(&a, &b)))
            }

            // ── Styling ─────────────────────────────────────────────
            NodeOp::SetFill => {
                let shape = get_any(inputs, 0);
                let color = get_color(inputs, 1);
                styling::set_fill(&shape, color)
            }
            NodeOp::SetStroke => {
                let shape = get_any(inputs, 0);
                let color = get_color(inputs, 1);
                let width = get_scalar(inputs, 2);
                styling::set_stroke(&shape, color, width)
            }

            // ── Utility ─────────────────────────────────────────────
            NodeOp::Merge => {
                let a = get_any(inputs, 0);
                let b = get_any(inputs, 1);
                utility::merge(&a, &b)
            }
            NodeOp::Duplicate => {
                let geometry = get_any(inputs, 0);
                let count = get_int(inputs, 1);
                let xform = get_transform(inputs, 2);
                utility::duplicate(&geometry, count, &xform)
            }
            // ── DSL ─────────────────────────────────────────────────
            NodeOp::DslCode { source } => {
                let mut compiler = self.dsl_compiler.lock();
                utility::dsl_code(source, &mut compiler, &self.dsl_cache, time_ctx)?
            }

            // ── Graph I/O ───────────────────────────────────────────
            NodeOp::GraphInput { .. } => {
                // Pass through — scheduler resolves graph inputs
                if !inputs.data.is_empty() {
                    inputs.data[0].clone()
                } else {
                    NodeData::Scalar(0.0)
                }
            }
            NodeOp::GraphOutput { .. } => {
                // Pass through input
                if !inputs.data.is_empty() {
                    inputs.data[0].clone()
                } else {
                    NodeData::Scalar(0.0)
                }
            }
        };

        if !outputs.data.is_empty() {
            outputs.data[0] = Some(result);
        }

        Ok(())
    }

    fn transform_points(&self, points: &PointBatch, transform: &Affine2) -> PointBatch {
        transforms::transform_point_batch(points, transform)
    }

    fn tessellate_path(&self, path: &PathData, fill: bool, tolerance: f32) -> TessellationOutput {
        tessellation::tessellate_path_lyon(path, fill, tolerance)
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)] // Safety contract is on the trait definition
    fn execute_dsl(
        &self,
        func_ptr: *const u8,
        ctx: &mut DslContext,
    ) -> Result<(), ComputeError> {
        let func: ExprFnPtr = unsafe { std::mem::transmute(func_ptr) };
        let _result = unsafe { func(ctx) };
        Ok(())
    }

    fn name(&self) -> &str {
        "CPU"
    }
}

// ---------------------------------------------------------------------------
// Input extraction helpers
// ---------------------------------------------------------------------------

fn get_scalar(inputs: &ResolvedInputs, idx: usize) -> f64 {
    match inputs.data.get(idx) {
        Some(NodeData::Scalar(v)) => *v,
        Some(NodeData::Int(v)) => *v as f64,
        _ => 0.0,
    }
}

fn get_int(inputs: &ResolvedInputs, idx: usize) -> i64 {
    match inputs.data.get(idx) {
        Some(NodeData::Int(v)) => *v,
        Some(NodeData::Scalar(v)) => *v as i64,
        _ => 0,
    }
}

fn get_vec2(inputs: &ResolvedInputs, idx: usize) -> Vec2 {
    match inputs.data.get(idx) {
        Some(NodeData::Vec2(v)) => *v,
        Some(NodeData::Scalar(v)) => Vec2::splat(*v as f32),
        Some(NodeData::Points(pts)) if !pts.xs.is_empty() => {
            Vec2::new(pts.xs[0], pts.ys[0])
        }
        _ => Vec2::ZERO,
    }
}

/// If the input at `idx` is a Points batch, return the batch. Used to detect
/// when a generator should auto-iterate instead of producing a single output.
fn get_points_batch(inputs: &ResolvedInputs, idx: usize) -> Option<Arc<PointBatch>> {
    match inputs.data.get(idx) {
        Some(NodeData::Points(pts)) if !pts.is_empty() => Some(Arc::clone(pts)),
        _ => None,
    }
}

fn get_path(inputs: &ResolvedInputs, idx: usize) -> Arc<PathData> {
    match inputs.data.get(idx) {
        Some(NodeData::Path(p)) => Arc::clone(p),
        Some(NodeData::Shape(s)) => Arc::new(transforms::bake_shape_to_path(s)),
        Some(NodeData::Paths(paths)) => {
            let mut merged = PathData::new();
            for p in paths.iter() {
                merged.verbs.extend_from_slice(&p.verbs);
            }
            Arc::new(merged)
        }
        Some(NodeData::Shapes(shapes)) => {
            let mut merged = PathData::new();
            for s in shapes.iter() {
                let baked = transforms::bake_shape_to_path(s);
                merged.verbs.extend_from_slice(&baked.verbs);
            }
            Arc::new(merged)
        }
        _ => Arc::new(PathData::new()),
    }
}

fn get_color(inputs: &ResolvedInputs, idx: usize) -> Color {
    match inputs.data.get(idx) {
        Some(NodeData::Color(c)) => *c,
        _ => Color::WHITE,
    }
}

fn get_transform(inputs: &ResolvedInputs, idx: usize) -> Affine2 {
    match inputs.data.get(idx) {
        Some(NodeData::Transform(t)) => *t,
        _ => Affine2::IDENTITY,
    }
}

/// Run a generator function once per point in a batch, collecting all produced
/// paths into a single merged PathData. This is the auto-iteration path for
/// generators that receive Points where they expect a single Vec2.
fn batch_generate(pts: &PointBatch, mut f: impl FnMut(Vec2) -> NodeData) -> NodeData {
    let mut merged = PathData::new();
    for i in 0..pts.len() {
        let center = Vec2::new(pts.xs[i], pts.ys[i]);
        if let NodeData::Path(p) = f(center) {
            merged.verbs.extend_from_slice(&p.verbs);
        }
    }
    NodeData::Path(Arc::new(merged))
}

fn get_any(inputs: &ResolvedInputs, idx: usize) -> NodeData {
    inputs
        .data
        .get(idx)
        .cloned()
        .unwrap_or(NodeData::Scalar(0.0))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::compute::ResolvedInputs;
    use vector_flow_core::types::{PathVerb, TimeContext};

    fn time_ctx() -> TimeContext {
        TimeContext::default()
    }

    #[test]
    fn evaluate_circle_node() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalar(50.0),          // radius
                NodeData::Vec2(Vec2::ZERO),       // center
                NodeData::Int(16),                // segments
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::Circle, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        let result = outputs.data[0].as_ref().unwrap();
        if let NodeData::Path(p) = result {
            let vertex_count = p.verbs.iter().filter(|v| !matches!(v, PathVerb::Close)).count();
            assert_eq!(vertex_count, 16);
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn evaluate_translate_node() {
        let backend = CpuBackend::new().unwrap();

        // Create a single-point path at origin
        let path = PathData {
            verbs: vec![PathVerb::MoveTo(vector_flow_core::types::Point { x: 0.0, y: 0.0 })],
            closed: false,
        };
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Path(Arc::new(path)),
                NodeData::Vec2(Vec2::new(10.0, 20.0)),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::Translate, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        if let Some(NodeData::Path(p)) = &outputs.data[0] {
            match p.verbs[0] {
                PathVerb::MoveTo(pt) => {
                    assert!((pt.x - 10.0).abs() < 1e-5);
                    assert!((pt.y - 20.0).abs() < 1e-5);
                }
                _ => panic!("expected MoveTo"),
            }
        } else {
            panic!("expected Path output");
        }
    }

    #[test]
    fn evaluate_set_fill_node() {
        let backend = CpuBackend::new().unwrap();
        let path = PathData::new();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Path(Arc::new(path)),
                NodeData::Color(Color::BLACK),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::SetFill, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        if let Some(NodeData::Shape(s)) = &outputs.data[0] {
            assert_eq!(s.fill, Some(Color::BLACK));
        } else {
            panic!("expected Shape output");
        }
    }

    #[test]
    fn evaluate_dsl_code_node() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![NodeData::Scalar(0.0)],
        };
        let mut outputs = NodeOutputs::new(1);
        let op = NodeOp::DslCode {
            source: "2.0 + 3.0".into(),
        };
        backend
            .evaluate_node(&op, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        if let Some(NodeData::Scalar(v)) = &outputs.data[0] {
            assert!((*v - 5.0).abs() < 1e-10);
        } else {
            panic!("expected Scalar output");
        }
    }

    #[test]
    fn end_to_end_circle_translate() {
        let backend = CpuBackend::new().unwrap();
        let tc = time_ctx();

        // Step 1: Generate a circle
        let circle_inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalar(100.0),
                NodeData::Vec2(Vec2::ZERO),
                NodeData::Int(32),
            ],
        };
        let mut circle_outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::Circle, &circle_inputs, &tc, &mut circle_outputs)
            .unwrap();
        let circle_path = circle_outputs.data[0].take().unwrap();

        // Step 2: Translate it
        let translate_inputs = ResolvedInputs {
            data: vec![circle_path, NodeData::Vec2(Vec2::new(50.0, 0.0))],
        };
        let mut translate_outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::Translate, &translate_inputs, &tc, &mut translate_outputs)
            .unwrap();

        let result = translate_outputs.data[0].as_ref().unwrap();
        if let NodeData::Path(p) = result {
            // First vertex should be at (150, 0) — radius 100 + translation 50
            match p.verbs[0] {
                PathVerb::MoveTo(pt) => {
                    assert!(
                        (pt.x - 150.0).abs() < 1e-3,
                        "expected x~150, got {}",
                        pt.x
                    );
                }
                _ => panic!("expected MoveTo"),
            }
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn tessellate_path_nonempty() {
        let backend = CpuBackend::new().unwrap();

        // Generate a rectangle
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalar(100.0),
                NodeData::Scalar(100.0),
                NodeData::Vec2(Vec2::ZERO),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::Rectangle, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        if let Some(NodeData::Path(path)) = &outputs.data[0] {
            let tess = backend.tessellate_path(path, true, 0.1);
            assert!(!tess.vertices.is_empty());
            assert!(!tess.indices.is_empty());
        } else {
            panic!("expected Path");
        }
    }
}
