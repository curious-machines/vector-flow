mod color_math;
mod color_ops;
mod generators;
mod path_ops;
mod styling;
pub(crate) mod svg_path;
pub(crate) mod text;
mod tessellation;
mod transforms;
mod utility;

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Affine2, Vec2};
use parking_lot::Mutex;

use vector_flow_core::compute::{
    ComputeBackend, DslContext, NodeOutputs, ResolvedInputs, TessellationOutput,
};
use vector_flow_core::error::ComputeError;
use vector_flow_core::node::NodeOp;
use vector_flow_core::types::{
    Color, ImageData, ImageInstance, LineCap, LineJoin, NodeData, PathData, PointBatch, Shape,
    StrokeStyle, EvalContext,
};

use vector_flow_dsl::cache::DslFunctionCache;
use vector_flow_dsl::codegen::{DslCompiler, ExprFnPtr};

/// CPU-based compute backend.
pub struct CpuBackend {
    dsl_compiler: Mutex<DslCompiler>,
    dsl_cache: DslFunctionCache,
    image_cache: Mutex<HashMap<String, Arc<ImageData>>>,
    svg_path_cache: svg_path::SvgPathCache,
    font_cache: Mutex<text::FontCache>,
}

impl CpuBackend {
    /// Create a new CPU backend with a fresh DSL compiler.
    pub fn new() -> Result<Self, ComputeError> {
        let compiler =
            DslCompiler::new().map_err(|e| ComputeError::BackendError(e.to_string()))?;
        Ok(Self {
            dsl_compiler: Mutex::new(compiler),
            dsl_cache: DslFunctionCache::new(),
            image_cache: Mutex::new(HashMap::new()),
            svg_path_cache: svg_path::SvgPathCache::new(),
            font_cache: Mutex::new(text::FontCache::new()),
        })
    }
}

impl CpuBackend {
    fn load_image_cached(&self, path: &str) -> Result<Arc<ImageData>, ComputeError> {
        let mut cache = self.image_cache.lock();
        if let Some(cached) = cache.get(path) {
            return Ok(Arc::clone(cached));
        }
        let img = image::open(path)
            .map_err(|e| ComputeError::BackendError(format!("Failed to load image '{}': {}", path, e)))?
            .into_rgba8();
        let (width, height) = img.dimensions();
        let pixels = img.into_raw();
        let data = Arc::new(ImageData {
            width,
            height,
            pixels,
            source_path: path.to_string(),
        });
        cache.insert(path.to_string(), Arc::clone(&data));
        Ok(data)
    }
}

impl ComputeBackend for CpuBackend {
    fn evaluate_node(
        &self,
        op: &NodeOp,
        inputs: &ResolvedInputs,
        time_ctx: &EvalContext,
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
                if let Some(pts) = get_points_batch(inputs, 1) {
                    batch_generate(&pts, |c| generators::circle(radius, c))
                } else {
                    generators::circle(radius, get_vec2(inputs, 1))
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
            NodeOp::SetStroke { ref dash_pattern } => {
                let shape = get_any(inputs, 0);
                let color = get_color(inputs, 1);
                let width = get_scalar(inputs, 2);
                let cap = int_to_line_cap(get_int(inputs, 3));
                let join = int_to_line_join(get_int(inputs, 4), get_scalar(inputs, 5) as f32);
                let dash_offset = get_scalar(inputs, 6) as f32;
                let dash_array = parse_dash_pattern(dash_pattern);
                styling::set_stroke(&shape, color, width, cap, join, dash_array, dash_offset)
            }
            NodeOp::StrokeToPath { ref dash_pattern } => {
                let shape = get_any(inputs, 0);
                let width = get_scalar(inputs, 1) as f32;
                let cap = int_to_line_cap(get_int(inputs, 2));
                let join = int_to_line_join(get_int(inputs, 3), get_scalar(inputs, 4) as f32);
                let dash_offset = get_scalar(inputs, 5) as f32;
                let dash_array = parse_dash_pattern(dash_pattern);
                let stroke = StrokeStyle {
                    color: Color::BLACK,
                    width,
                    line_cap: cap,
                    line_join: join,
                    dash_array,
                    dash_offset,
                };
                styling::stroke_to_path(&shape, &stroke)
            }

            // ── Color operations ───────────────────────────────────────
            NodeOp::AdjustHue => {
                let data = get_any(inputs, 0);
                let amount = get_scalar(inputs, 1);
                let absolute = get_bool(inputs, 2);
                color_ops::adjust_hue(&data, amount, absolute)
            }
            NodeOp::AdjustSaturation => {
                let data = get_any(inputs, 0);
                let amount = get_scalar(inputs, 1);
                let absolute = get_bool(inputs, 2);
                color_ops::adjust_saturation(&data, amount, absolute)
            }
            NodeOp::AdjustLightness => {
                let data = get_any(inputs, 0);
                let amount = get_scalar(inputs, 1);
                let absolute = get_bool(inputs, 2);
                color_ops::adjust_lightness(&data, amount, absolute)
            }
            NodeOp::AdjustLuminance => {
                let data = get_any(inputs, 0);
                let amount = get_scalar(inputs, 1);
                let absolute = get_bool(inputs, 2);
                color_ops::adjust_luminance(&data, amount, absolute)
            }
            NodeOp::InvertColor => {
                let data = get_any(inputs, 0);
                color_ops::invert_color(&data)
            }
            NodeOp::Grayscale => {
                let data = get_any(inputs, 0);
                color_ops::grayscale(&data)
            }
            NodeOp::MixColors => {
                let a = get_color(inputs, 0);
                let b = get_color(inputs, 1);
                let factor = get_scalar(inputs, 2);
                let lab_mode = get_bool(inputs, 3);
                NodeData::Color(color_ops::mix_colors(a, b, factor, lab_mode))
            }
            NodeOp::SetAlpha => {
                let data = get_any(inputs, 0);
                let alpha = get_scalar(inputs, 1);
                color_ops::set_alpha(&data, alpha)
            }
            NodeOp::ColorParse { text } => {
                NodeData::Color(color_ops::color_parse(text))
            }
            NodeOp::SvgPath { data } => {
                let path = self.svg_path_cache.get_or_parse(data);
                NodeData::Path(path)
            }

            // ── Constants ───────────────────────────────────────────
            NodeOp::ConstScalar => NodeData::Scalar(get_scalar(inputs, 0)),
            NodeOp::ConstInt => NodeData::Int(get_int(inputs, 0)),
            NodeOp::ConstVec2 => {
                let x = get_scalar(inputs, 0) as f32;
                let y = get_scalar(inputs, 1) as f32;
                NodeData::Vec2(glam::Vec2::new(x, y))
            }
            NodeOp::ConstColor => NodeData::Color(get_color(inputs, 0)),

            // ── Portals ────────────────────────────────────────────
            NodeOp::PortalSend { .. } => {
                // Pass through input.
                if !inputs.data.is_empty() {
                    inputs.data[0].clone()
                } else {
                    NodeData::Scalar(0.0)
                }
            }
            NodeOp::PortalReceive { .. } => {
                // Resolved by scheduler, not compute. Shouldn't reach here.
                NodeData::Scalar(0.0)
            }

            // ── Utility ─────────────────────────────────────────────
            NodeOp::Merge => {
                utility::merge_n(inputs)
            }
            NodeOp::Duplicate => {
                let geometry = get_any(inputs, 0);
                let count = get_int(inputs, 1);
                let xform = get_transform(inputs, 2);
                utility::duplicate(&geometry, count, &xform)
            }
            NodeOp::CopyToPoints => {
                let geometry = get_any(inputs, 0);
                let target_path = get_path(inputs, 1);
                let count = get_int(inputs, 2);
                let align = get_bool(inputs, 3);
                let (points, tangent_angles) =
                    path_ops::resample_with_tangents(&target_path, count);
                let (shapes, angles, indices, total) =
                    utility::copy_to_points(&geometry, &points, &tangent_angles, align);
                // Multi-output: write all outputs and return early.
                if outputs.data.len() > 0 {
                    outputs.data[0] = Some(shapes);
                }
                if outputs.data.len() > 1 {
                    outputs.data[1] = Some(NodeData::Scalars(Arc::new(angles)));
                }
                if outputs.data.len() > 2 {
                    outputs.data[2] = Some(NodeData::Scalars(Arc::new(indices)));
                }
                if outputs.data.len() > 3 {
                    outputs.data[3] = Some(NodeData::Scalar(total));
                }
                return Ok(());
            }
            // ── DSL ─────────────────────────────────────────────────
            NodeOp::DslCode { source, script_inputs, script_outputs } => {
                let mut compiler = self.dsl_compiler.lock();
                utility::dsl_code(
                    source,
                    script_inputs,
                    script_outputs,
                    inputs,
                    &mut compiler,
                    &self.dsl_cache,
                    time_ctx,
                    outputs,
                )?;
                // dsl_code writes outputs directly; skip the default assignment below.
                return Ok(());
            }

            // ── Image ───────────────────────────────────────────────
            NodeOp::LoadImage { path } => {
                let position = get_vec2(inputs, 0);
                let width = get_scalar(inputs, 1) as f32;
                let height = get_scalar(inputs, 2) as f32;
                let opacity = get_scalar(inputs, 3).clamp(0.0, 1.0) as f32;

                // Resolve relative paths against project directory
                let resolved_path = resolve_path(path, &time_ctx.project_dir);
                let image_data = self.load_image_cached(&resolved_path)?;

                // Compute scale: if width/height are 0, use native size
                let native_w = image_data.width as f32;
                let native_h = image_data.height as f32;
                let sx = if width > 0.0 { width / native_w } else { 1.0 };
                let sy = if height > 0.0 { height / native_h } else { 1.0 };

                let transform = Affine2::from_translation(position)
                    * Affine2::from_scale(Vec2::new(sx, sy));

                // Write all outputs directly: image, native_width, native_height
                if !outputs.data.is_empty() {
                    outputs.data[0] = Some(NodeData::Image(Arc::new(ImageInstance {
                        image: image_data,
                        transform,
                        opacity,
                    })));
                }
                if outputs.data.len() > 1 {
                    outputs.data[1] = Some(NodeData::Scalar(native_w as f64));
                }
                if outputs.data.len() > 2 {
                    outputs.data[2] = Some(NodeData::Scalar(native_h as f64));
                }
                return Ok(());
            }

            // ── Text ─────────────────────────────────────────────────
            NodeOp::Text { text, font_family, font_path } => {
                let mut font_cache = self.font_cache.lock();
                let (inst, w, h) = text::execute_text(
                    text,
                    font_family,
                    font_path,
                    inputs,
                    &mut font_cache,
                    &time_ctx.project_dir,
                )?;
                if !outputs.data.is_empty() {
                    outputs.data[0] = Some(NodeData::Text(inst));
                }
                if outputs.data.len() > 1 {
                    outputs.data[1] = Some(NodeData::Scalar(w));
                }
                if outputs.data.len() > 2 {
                    outputs.data[2] = Some(NodeData::Scalar(h));
                }
                return Ok(());
            }
            NodeOp::TextToPath => {
                let text_inst = inputs.data.first().and_then(|d| {
                    if let NodeData::Text(t) = d { Some(t) } else { None }
                });
                let result = if let Some(inst) = text_inst {
                    let (path, transform) = text::text_to_path(inst)?;
                    // Return as Shape so the font-unit path gets tessellated at
                    // high resolution, with the scale applied as a GPU transform.
                    NodeData::Shape(Arc::new(Shape {
                        path,
                        fill: Some(inst.color),
                        stroke: None,
                        transform,
                    }))
                } else {
                    NodeData::Path(Arc::new(PathData::new()))
                };
                if !outputs.data.is_empty() {
                    outputs.data[0] = Some(result);
                }
                return Ok(());
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

fn get_bool(inputs: &ResolvedInputs, idx: usize) -> bool {
    match inputs.data.get(idx) {
        Some(NodeData::Bool(v)) => *v,
        Some(NodeData::Int(v)) => *v != 0,
        _ => false,
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

/// Resolve a file path: if relative, prepend the project directory.
fn resolve_path(path: &str, project_dir: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() || project_dir.is_empty() {
        path.to_string()
    } else {
        let base = std::path::Path::new(project_dir);
        base.join(p).to_string_lossy().into_owned()
    }
}

fn get_any(inputs: &ResolvedInputs, idx: usize) -> NodeData {
    inputs
        .data
        .get(idx)
        .cloned()
        .unwrap_or(NodeData::Scalar(0.0))
}

fn int_to_line_cap(v: i64) -> LineCap {
    match v {
        1 => LineCap::Round,
        2 => LineCap::Square,
        _ => LineCap::Butt,
    }
}

fn int_to_line_join(v: i64, miter_limit: f32) -> LineJoin {
    match v {
        1 => LineJoin::Round,
        2 => LineJoin::Bevel,
        _ => LineJoin::Miter(miter_limit),
    }
}

fn parse_dash_pattern(s: &str) -> Vec<f32> {
    if s.trim().is_empty() {
        return vec![];
    }
    s.split(|c: char| c == ',' || c == ' ')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.trim().parse::<f32>().ok())
        .filter(|v| *v > 0.0)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::compute::ResolvedInputs;
    use vector_flow_core::types::{PathVerb, EvalContext};

    fn time_ctx() -> EvalContext {
        EvalContext::default()
    }

    #[test]
    fn evaluate_circle_node() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalar(50.0),          // radius
                NodeData::Vec2(Vec2::ZERO),       // center
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::Circle, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        let result = outputs.data[0].as_ref().unwrap();
        if let NodeData::Path(p) = result {
            // 1 MoveTo + 4 CubicTo = 5 non-Close verbs
            let vertex_count = p.verbs.iter().filter(|v| !matches!(v, PathVerb::Close)).count();
            assert_eq!(vertex_count, 5);
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
            script_inputs: Vec::new(),
            script_outputs: Vec::new(),
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

    #[test]
    fn parse_dash_pattern_tests() {
        assert_eq!(parse_dash_pattern(""), Vec::<f32>::new());
        assert_eq!(parse_dash_pattern("10,5"), vec![10.0, 5.0]);
        assert_eq!(parse_dash_pattern("10 5 3 5"), vec![10.0, 5.0, 3.0, 5.0]);
        assert_eq!(parse_dash_pattern("10, 5, 3"), vec![10.0, 5.0, 3.0]);
        assert_eq!(parse_dash_pattern("abc"), Vec::<f32>::new());
        // Negative values filtered out
        assert_eq!(parse_dash_pattern("10,-5,3"), vec![10.0, 3.0]);
    }

    #[test]
    fn int_to_cap_join_conversion() {
        assert_eq!(int_to_line_cap(0), LineCap::Butt);
        assert_eq!(int_to_line_cap(1), LineCap::Round);
        assert_eq!(int_to_line_cap(2), LineCap::Square);
        assert_eq!(int_to_line_cap(99), LineCap::Butt); // fallback

        assert_eq!(int_to_line_join(0, 4.0), LineJoin::Miter(4.0));
        assert_eq!(int_to_line_join(1, 4.0), LineJoin::Round);
        assert_eq!(int_to_line_join(2, 4.0), LineJoin::Bevel);
    }
}
