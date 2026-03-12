mod color_math;
mod color_ops;
mod generators;
mod noise;
pub(crate) mod path_ops;
mod perturb;
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
            NodeOp::Arc => {
                let outer_radius = get_scalar(inputs, 0);
                let inner_radius = get_scalar(inputs, 1);
                let start_angle = get_scalar(inputs, 2);
                let sweep_angle = get_scalar(inputs, 3);
                let close = get_bool(inputs, 4);
                if let Some(pts) = get_points_batch(inputs, 5) {
                    batch_generate(&pts, |c| {
                        generators::arc(outer_radius, inner_radius, start_angle, sweep_angle, close, c)
                    })
                } else {
                    generators::arc(outer_radius, inner_radius, start_angle, sweep_angle, close, get_vec2(inputs, 5))
                }
            }
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
            NodeOp::Noise => {
                let points = get_points(inputs, 0);
                let seed = get_int(inputs, 1) as u32;
                let frequency = get_scalar(inputs, 2);
                let octaves = get_int(inputs, 3).clamp(1, 32) as usize;
                let lacunarity = get_scalar(inputs, 4);
                let amplitude = get_scalar(inputs, 5);
                let offset_x = get_scalar(inputs, 6);
                let offset_y = get_scalar(inputs, 7);
                let values = noise::sample_noise_batch(
                    &points.xs, &points.ys,
                    seed, frequency, octaves, lacunarity, amplitude, offset_x, offset_y,
                );
                NodeData::Scalars(Arc::new(values))
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
            NodeOp::WarpToCurve { mode } => {
                let geometry = get_any(inputs, 0);
                let curve = get_path(inputs, 1);
                let mode = *mode as i64;
                let smoothing = get_scalar(inputs, 2).clamp(0.0, 1.0) as f32;
                let tolerance = get_scalar(inputs, 3) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                path_ops::warp_to_curve(&geometry, &curve, mode, smoothing, tolerance)
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
                let tolerance = get_scalar(inputs, 2) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                path_ops::resample_path(&path, count, tolerance)
            }
            NodeOp::PathOffset => {
                let path = get_path(inputs, 0);
                let distance = get_scalar(inputs, 1);
                let tolerance = get_scalar(inputs, 2) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                NodeData::Path(Arc::new(path_ops::path_offset(&path, distance, tolerance)))
            }
            NodeOp::PathBoolean { operation } => {
                let a = get_path(inputs, 0);
                let b = get_path(inputs, 1);
                let tolerance = get_scalar(inputs, 2) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                let (combined, parts) = path_ops::path_boolean_with_parts(&a, &b, *operation, tolerance);
                if !outputs.data.is_empty() {
                    outputs.data[0] = Some(NodeData::Path(Arc::new(combined)));
                }
                if outputs.data.len() > 1 {
                    outputs.data[1] = Some(NodeData::Paths(Arc::new(parts)));
                }
                return Ok(());
            }
            NodeOp::PathIntersectionPoints => {
                let a = get_path(inputs, 0);
                let b = get_path(inputs, 1);
                let tolerance = get_scalar(inputs, 2) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                let (points, t_a, t_b) = path_ops::path_intersection_points(&a, &b, tolerance);
                let count = points.len() as i64;
                if !outputs.data.is_empty() {
                    outputs.data[0] = Some(NodeData::Points(Arc::new(points)));
                }
                if outputs.data.len() > 1 {
                    outputs.data[1] = Some(NodeData::Scalars(Arc::new(t_a)));
                }
                if outputs.data.len() > 2 {
                    outputs.data[2] = Some(NodeData::Scalars(Arc::new(t_b)));
                }
                if outputs.data.len() > 3 {
                    outputs.data[3] = Some(NodeData::Int(count));
                }
                return Ok(());
            }
            NodeOp::SplitPathAtT => {
                let path = get_path(inputs, 0);
                let t_values = get_scalars(inputs, 1);
                let tolerance = get_scalar(inputs, 2) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                let close = get_bool(inputs, 3);
                let parts = path_ops::split_path_at_t(&path, &t_values, tolerance, close);
                let count = parts.len() as i64;
                if !outputs.data.is_empty() {
                    outputs.data[0] = Some(NodeData::Paths(Arc::new(parts)));
                }
                if outputs.data.len() > 1 {
                    outputs.data[1] = Some(NodeData::Int(count));
                }
                return Ok(());
            }
            NodeOp::PerturbPoints { method, target, per_axis, preserve_smoothness } => {
                let geometry = get_any(inputs, 0);
                let seed = get_int(inputs, 1);
                let amount = get_scalar(inputs, 2);
                let amount_x = get_scalar(inputs, 3);
                let amount_y = get_scalar(inputs, 4);
                let frequency = get_scalar(inputs, 5);
                let octaves = get_int(inputs, 6);
                let lacunarity = get_scalar(inputs, 7);
                let handle_scale = get_scalar(inputs, 8);
                perturb::perturb_points(
                    &geometry, seed, *method, *target, *per_axis, *preserve_smoothness,
                    amount, amount_x, amount_y, frequency, octaves, lacunarity, handle_scale,
                )
            }
            NodeOp::ClosePath => {
                let data = get_any(inputs, 0);
                match &data {
                    NodeData::Path(p) => NodeData::Path(Arc::new(path_ops::close_path(p))),
                    NodeData::Paths(paths) => {
                        let closed: Vec<PathData> = paths.iter().map(path_ops::close_path).collect();
                        NodeData::Paths(Arc::new(closed))
                    }
                    NodeData::Shape(s) => {
                        let closed_path = Arc::new(path_ops::close_path(&s.path));
                        NodeData::Shape(Arc::new(Shape { path: closed_path, ..(**s).clone() }))
                    }
                    NodeData::Shapes(shapes) => {
                        let closed: Vec<Shape> = shapes.iter().map(|s| {
                            let closed_path = Arc::new(path_ops::close_path(&s.path));
                            Shape { path: closed_path, ..s.clone() }
                        }).collect();
                        NodeData::Shapes(Arc::new(closed))
                    }
                    other => other.clone(),
                }
            }
            NodeOp::PolygonFromPoints => {
                let points = get_points(inputs, 0);
                let close = get_bool(inputs, 1);
                NodeData::Path(Arc::new(path_ops::polygon_from_points(&points, close)))
            }
            NodeOp::SplineFromPoints => {
                let points = get_points(inputs, 0);
                let close = get_bool(inputs, 1);
                let tension = get_scalar(inputs, 2);
                NodeData::Path(Arc::new(path_ops::spline_from_points(&points, close, tension)))
            }

            // ── Styling ─────────────────────────────────────────────
            NodeOp::SetFill => {
                let shape = get_any(inputs, 0);
                let color_data = &inputs.data[1];
                styling::set_fill(&shape, color_data)
            }
            NodeOp::SetStroke { ref dash_pattern } => {
                let shape = get_any(inputs, 0);
                let color_data = &inputs.data[1];
                let width = get_scalar(inputs, 2);
                let cap = int_to_line_cap(get_int(inputs, 3));
                let join = int_to_line_join(get_int(inputs, 4), get_scalar(inputs, 5) as f32);
                let dash_offset = get_scalar(inputs, 6) as f32;
                let tolerance = get_scalar(inputs, 7) as f32;
                let dash_array = parse_dash_pattern(dash_pattern);
                styling::set_stroke(&shape, color_data, width, cap, join, dash_array, dash_offset, tolerance)
            }
            NodeOp::SetStyle { ref dash_pattern } => {
                let shape = get_any(inputs, 0);
                // Fill params
                let fill_color_data = &inputs.data[1];
                let fill_opacity = get_scalar(inputs, 2) as f32;
                let has_fill = get_bool(inputs, 3);
                // Stroke params
                let stroke_color_data = &inputs.data[4];
                let stroke_width = get_scalar(inputs, 5);
                let stroke_opacity = get_scalar(inputs, 6) as f32;
                let has_stroke = get_bool(inputs, 7);
                let cap = int_to_line_cap(get_int(inputs, 8));
                let join = int_to_line_join(get_int(inputs, 9), get_scalar(inputs, 10) as f32);
                let dash_offset = get_scalar(inputs, 11) as f32;
                let tolerance = get_scalar(inputs, 12) as f32;
                let dash_array = parse_dash_pattern(dash_pattern);

                // Apply fill first (if enabled), then stroke.
                let mut result = shape;
                if has_fill {
                    // Apply fill opacity to color.
                    let fill_data = apply_opacity_to_color(fill_color_data, fill_opacity);
                    result = styling::set_fill(&result, &fill_data);
                }
                if has_stroke {
                    let stroke_data = apply_opacity_to_color(stroke_color_data, stroke_opacity);
                    result = styling::set_stroke(
                        &result, &stroke_data, stroke_width, cap, join,
                        dash_array, dash_offset, tolerance,
                    );
                }
                result
            }
            NodeOp::StrokeToPath { ref dash_pattern } => {
                let shape = get_any(inputs, 0);
                let width = get_scalar(inputs, 1) as f32;
                let cap = int_to_line_cap(get_int(inputs, 2));
                let join = int_to_line_join(get_int(inputs, 3), get_scalar(inputs, 4) as f32);
                let dash_offset = get_scalar(inputs, 5) as f32;
                let tolerance = get_scalar(inputs, 6) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                let dash_array = parse_dash_pattern(dash_pattern);
                let stroke = StrokeStyle {
                    color: Color::BLACK,
                    width,
                    line_cap: cap,
                    line_join: join,
                    dash_array,
                    dash_offset,
                    tolerance: 0.0,
                };
                styling::stroke_to_path(&shape, &stroke, tolerance)
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
            NodeOp::AdjustAlpha => {
                let data = get_any(inputs, 0);
                let amount = get_scalar(inputs, 1);
                let absolute = get_bool(inputs, 2);
                color_ops::adjust_alpha(&data, amount, absolute)
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
            NodeOp::Merge { keep_separate } => {
                utility::merge_n(inputs, *keep_separate)
            }
            NodeOp::PackPoints => {
                let xs = get_scalars(inputs, 0);
                let ys = get_scalars(inputs, 1);
                let len = xs.len().min(ys.len());
                let mut px = Vec::with_capacity(len);
                let mut py = Vec::with_capacity(len);
                for i in 0..len {
                    let x = xs[i];
                    let y = ys[i];
                    if x.is_finite() && y.is_finite() {
                        px.push(x as f32);
                        py.push(y as f32);
                    }
                }
                let points = PointBatch { xs: px, ys: py };
                NodeData::Points(Arc::new(points))
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
                let tolerance = get_scalar(inputs, 4) as f32;
                let tolerance = if tolerance <= 0.0 { time_ctx.tolerance } else { tolerance };
                let (points, tangent_angles) =
                    path_ops::resample_with_tangents(&target_path, count, tolerance);
                let (shapes, angles, indices, total) =
                    utility::copy_to_points(&geometry, &points, &tangent_angles, align);
                // Multi-output: write all outputs and return early.
                if !outputs.data.is_empty() {
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
            NodeOp::PlaceAtPoints => {
                let geometry = get_any(inputs, 0);
                let points = get_points_batch(inputs, 1);
                let cycle = get_bool(inputs, 2);
                match points {
                    Some(pts) => utility::place_at_points(&geometry, &pts, cycle),
                    None => NodeData::Shapes(Arc::new(Vec::new())),
                }
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

            NodeOp::Map { source, script_inputs, script_outputs } => {
                let mut compiler = self.dsl_compiler.lock();
                utility::map_batch(
                    source,
                    script_inputs,
                    script_outputs,
                    inputs,
                    &mut compiler,
                    &self.dsl_cache,
                    time_ctx,
                    outputs,
                )?;
                return Ok(());
            }

            NodeOp::Generate { source, script_inputs, script_outputs } => {
                let mut compiler = self.dsl_compiler.lock();
                utility::generate_range(
                    source,
                    script_inputs,
                    script_outputs,
                    inputs,
                    &mut compiler,
                    &self.dsl_cache,
                    time_ctx,
                    outputs,
                )?;
                return Ok(());
            }

            // ── Image ───────────────────────────────────────────────
            NodeOp::LoadImage { path } => {
                // Empty path means no image configured yet — skip silently
                if path.is_empty() {
                    return Ok(());
                }

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
                        path: Arc::new(path),
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
fn get_points(inputs: &ResolvedInputs, idx: usize) -> Arc<PointBatch> {
    match inputs.data.get(idx) {
        Some(NodeData::Points(pts)) => Arc::clone(pts),
        _ => Arc::new(PointBatch::new()),
    }
}

fn get_scalars(inputs: &ResolvedInputs, idx: usize) -> Vec<f64> {
    match inputs.data.get(idx) {
        Some(NodeData::Scalars(v)) => (**v).clone(),
        Some(NodeData::Scalar(v)) => vec![*v],
        Some(NodeData::Ints(v)) => v.iter().map(|&i| i as f64).collect(),
        Some(NodeData::Int(v)) => vec![*v as f64],
        _ => vec![],
    }
}

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

/// Apply opacity multiplier to a color NodeData (single or batch).
fn apply_opacity_to_color(color_data: &NodeData, opacity: f32) -> NodeData {
    if (opacity - 1.0).abs() < 1e-6 {
        return color_data.clone();
    }
    match color_data {
        NodeData::Color(c) => NodeData::Color(Color {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a * opacity,
        }),
        NodeData::Colors(colors) => {
            let modified: Vec<Color> = colors.iter().map(|c| Color {
                r: c.r,
                g: c.g,
                b: c.b,
                a: c.a * opacity,
            }).collect();
            NodeData::Colors(Arc::new(modified))
        }
        other => other.clone(),
    }
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
    s.split([',', ' '])
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
    use vector_flow_core::types::{DataType, PathVerb, Point, EvalContext};

    fn time_ctx() -> EvalContext {
        EvalContext::default()
    }

    #[test]
    fn evaluate_arc_node() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalar(100.0),         // outer_radius
                NodeData::Scalar(50.0),          // inner_radius
                NodeData::Scalar(0.0),           // start_angle
                NodeData::Scalar(90.0),          // sweep_angle
                NodeData::Bool(true),            // close
                NodeData::Vec2(Vec2::ZERO),      // center
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::Arc, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        let result = outputs.data[0].as_ref().unwrap();
        if let NodeData::Path(p) = result {
            assert!(p.closed);
            // Donut wedge: should have cubics for both arcs
            let cubic_count = p.verbs.iter().filter(|v| matches!(v, PathVerb::CubicTo { .. })).count();
            assert!(cubic_count >= 2); // at least one outer + one inner
        } else {
            panic!("expected Path");
        }
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

    #[test]
    fn map_scalars_double() {
        // Map over [1.0, 2.0, 3.0], doubling each element.
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![1.0, 2.0, 3.0])),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Map {
                    source: "result = element * 2.0;".into(),
                    script_inputs: vec![
                        ("element".into(), DataType::Scalar),
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 3);
            assert!((v[0] - 2.0).abs() < 1e-10);
            assert!((v[1] - 4.0).abs() < 1e-10);
            assert!((v[2] - 6.0).abs() < 1e-10);
        } else {
            panic!("expected Scalars output, got {:?}", outputs.data[0]);
        }
    }

    #[test]
    fn map_with_index() {
        // Map over [10.0, 20.0, 30.0], output = element + index.
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![10.0, 20.0, 30.0])),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Map {
                    source: "result = element + index;".into(),
                    script_inputs: vec![
                        ("element".into(), DataType::Scalar),
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 3);
            assert!((v[0] - 10.0).abs() < 1e-10); // 10 + 0
            assert!((v[1] - 21.0).abs() < 1e-10); // 20 + 1
            assert!((v[2] - 32.0).abs() < 1e-10); // 30 + 2
        } else {
            panic!("expected Scalars output");
        }
    }

    #[test]
    fn map_scalars_to_colors() {
        // Map over indices, producing colors via hsl().
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![0.0, 1.0, 2.0])),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Map {
                    source: "result = hsl(index * 120.0, 100.0, 50.0);".into(),
                    script_inputs: vec![
                        ("element".into(), DataType::Scalar),
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                    ],
                    script_outputs: vec![("result".into(), DataType::Color)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Colors(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 3);
            // hsl(0, 100, 50) = red
            assert!((v[0].r - 1.0).abs() < 0.01, "red r={}", v[0].r);
            assert!(v[0].g < 0.01, "red g={}", v[0].g);
            // hsl(120, 100, 50) = green
            assert!(v[1].r < 0.01, "green r={}", v[1].r);
            assert!((v[1].g - 1.0).abs() < 0.01, "green g={}", v[1].g);
            // hsl(240, 100, 50) = blue
            assert!(v[2].r < 0.01, "blue r={}", v[2].r);
            assert!((v[2].b - 1.0).abs() < 0.01, "blue b={}", v[2].b);
        } else {
            panic!("expected Colors output, got {:?}", outputs.data[0]);
        }
    }

    #[test]
    fn map_empty_batch() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![])),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Map {
                    source: "result = element;".into(),
                    script_inputs: vec![
                        ("element".into(), DataType::Scalar),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 0);
        } else {
            panic!("expected empty Scalars output");
        }
    }

    #[test]
    fn map_with_extra_input() {
        // Map with an extra input "offset" connected via graph port 1.
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![1.0, 2.0])),
                NodeData::Scalar(100.0), // extra input: offset
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Map {
                    source: "result = element + offset;".into(),
                    script_inputs: vec![
                        ("element".into(), DataType::Scalar),
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                        ("offset".into(), DataType::Scalar),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 2);
            assert!((v[0] - 101.0).abs() < 1e-10);
            assert!((v[1] - 102.0).abs() < 1e-10);
        } else {
            panic!("expected Scalars output");
        }
    }

    #[test]
    fn generate_basic() {
        // Generate 0..5, output = index as scalar.
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Int(0),  // start
                NodeData::Int(5),  // end
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Generate {
                    source: "result = index;".into(),
                    script_inputs: vec![
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 5);
            for i in 0..5 {
                assert!((v[i] - i as f64).abs() < 1e-10);
            }
        } else {
            panic!("expected Scalars output, got {:?}", outputs.data[0]);
        }
    }

    #[test]
    fn generate_empty_range() {
        // start >= end → empty batch.
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Int(5),  // start
                NodeData::Int(5),  // end
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Generate {
                    source: "result = index;".into(),
                    script_inputs: vec![
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 0);
        } else {
            panic!("expected empty Scalars output, got {:?}", outputs.data[0]);
        }
    }

    #[test]
    fn generate_negative_start() {
        // Generate -2..3 → produces indices [-2, -1, 0, 1, 2].
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Int(-2), // start
                NodeData::Int(3),  // end
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Generate {
                    source: "result = index;".into(),
                    script_inputs: vec![
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 5);
            assert!((v[0] - (-2.0)).abs() < 1e-10);
            assert!((v[1] - (-1.0)).abs() < 1e-10);
            assert!((v[2] - 0.0).abs() < 1e-10);
            assert!((v[3] - 1.0).abs() < 1e-10);
            assert!((v[4] - 2.0).abs() < 1e-10);
        } else {
            panic!("expected Scalars output");
        }
    }

    #[test]
    fn generate_with_extra_input() {
        // Generate 0..3 with extra input "scale" = 10.0.
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Int(0),      // start
                NodeData::Int(3),      // end
                NodeData::Scalar(10.0), // extra input: scale (graph port 2)
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(
                &NodeOp::Generate {
                    source: "result = index * scale;".into(),
                    script_inputs: vec![
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                        ("scale".into(), DataType::Scalar),
                    ],
                    script_outputs: vec![("result".into(), DataType::Scalar)],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(v)) = &outputs.data[0] {
            assert_eq!(v.len(), 3);
            assert!((v[0] - 0.0).abs() < 1e-10);
            assert!((v[1] - 10.0).abs() < 1e-10);
            assert!((v[2] - 20.0).abs() < 1e-10);
        } else {
            panic!("expected Scalars output");
        }
    }

    #[test]
    fn generate_multi_output() {
        // Generate 0..4 with two outputs: out_x = index * 10, out_y = index * 100.
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Int(0), // start
                NodeData::Int(4), // end
            ],
        };
        let mut outputs = NodeOutputs::new(2);
        backend
            .evaluate_node(
                &NodeOp::Generate {
                    source: "out_x = 1.0 * index * 10;\nout_y = 1.0 * index * 100;".into(),
                    script_inputs: vec![
                        ("index".into(), DataType::Int),
                        ("count".into(), DataType::Int),
                    ],
                    script_outputs: vec![
                        ("out_x".into(), DataType::Scalar),
                        ("out_y".into(), DataType::Scalar),
                    ],
                },
                &inputs,
                &time_ctx(),
                &mut outputs,
            )
            .unwrap();

        if let Some(NodeData::Scalars(xs)) = &outputs.data[0] {
            assert_eq!(xs.len(), 4);
            assert!((xs[0] - 0.0).abs() < 1e-10);
            assert!((xs[1] - 10.0).abs() < 1e-10);
            assert!((xs[2] - 20.0).abs() < 1e-10);
            assert!((xs[3] - 30.0).abs() < 1e-10);
        } else {
            panic!("expected Scalars for output 0, got {:?}", outputs.data[0]);
        }

        if let Some(NodeData::Scalars(ys)) = &outputs.data[1] {
            assert_eq!(ys.len(), 4);
            assert!((ys[0] - 0.0).abs() < 1e-10);
            assert!((ys[1] - 100.0).abs() < 1e-10);
            assert!((ys[2] - 200.0).abs() < 1e-10);
            assert!((ys[3] - 300.0).abs() < 1e-10);
        } else {
            panic!("expected Scalars for output 1, got {:?}", outputs.data[1]);
        }
    }

    #[test]
    fn evaluate_set_style_fill_and_stroke() {
        let backend = CpuBackend::new().unwrap();
        let path = PathData::new();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Path(Arc::new(path)),     // 0: path
                NodeData::Color(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }), // 1: fill_color
                NodeData::Scalar(1.0),               // 2: fill_opacity
                NodeData::Bool(true),                // 3: has_fill
                NodeData::Color(Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }), // 4: stroke_color
                NodeData::Scalar(3.0),               // 5: stroke_width
                NodeData::Scalar(1.0),               // 6: stroke_opacity
                NodeData::Bool(true),                // 7: has_stroke
                NodeData::Int(0),                    // 8: cap
                NodeData::Int(0),                    // 9: join
                NodeData::Scalar(4.0),               // 10: miter_limit
                NodeData::Scalar(0.0),               // 11: dash_offset
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        let op = NodeOp::SetStyle { dash_pattern: String::new() };
        backend
            .evaluate_node(&op, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        if let Some(NodeData::Shape(s)) = &outputs.data[0] {
            assert_eq!(s.fill, Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }));
            let stroke = s.stroke.as_ref().expect("expected stroke");
            assert_eq!(stroke.color, Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 });
            assert!((stroke.width - 3.0).abs() < 1e-6);
        } else {
            panic!("expected Shape output");
        }
    }

    #[test]
    fn evaluate_set_style_fill_only() {
        let backend = CpuBackend::new().unwrap();
        let path = PathData::new();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Path(Arc::new(path)),
                NodeData::Color(Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 }),
                NodeData::Scalar(0.5),  // fill_opacity
                NodeData::Bool(true),   // has_fill
                NodeData::Color(Color::BLACK),
                NodeData::Scalar(2.0),
                NodeData::Scalar(1.0),
                NodeData::Bool(false),  // has_stroke = false
                NodeData::Int(0),
                NodeData::Int(0),
                NodeData::Scalar(4.0),
                NodeData::Scalar(0.0),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        let op = NodeOp::SetStyle { dash_pattern: String::new() };
        backend
            .evaluate_node(&op, &inputs, &time_ctx(), &mut outputs)
            .unwrap();

        if let Some(NodeData::Shape(s)) = &outputs.data[0] {
            // Fill color with opacity applied
            let fill = s.fill.expect("expected fill");
            assert!((fill.a - 0.5).abs() < 1e-6);
            // No stroke
            assert!(s.stroke.is_none());
        } else {
            panic!("expected Shape output");
        }
    }

    #[test]
    fn apply_opacity_modifies_color() {
        let color = NodeData::Color(Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
        let result = super::apply_opacity_to_color(&color, 0.5);
        if let NodeData::Color(c) = result {
            assert!((c.a - 0.5).abs() < 1e-6);
            assert!((c.r - 1.0).abs() < 1e-6); // RGB unchanged
        } else {
            panic!("expected Color");
        }

        // Opacity 1.0 returns clone (no modification)
        let same = super::apply_opacity_to_color(&color, 1.0);
        if let NodeData::Color(c) = same {
            assert!((c.a - 1.0).abs() < 1e-6);
        } else {
            panic!("expected Color");
        }
    }

    #[test]
    fn evaluate_pack_points() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![1.0, 2.0, 3.0])),
                NodeData::Scalars(Arc::new(vec![4.0, 5.0, 6.0])),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::PackPoints, &inputs, &time_ctx(), &mut outputs)
            .unwrap();
        if let Some(NodeData::Points(pts)) = &outputs.data[0] {
            assert_eq!(pts.len(), 3);
            assert!((pts.xs[0] - 1.0).abs() < 1e-6);
            assert!((pts.ys[2] - 6.0).abs() < 1e-6);
        } else {
            panic!("expected Points output");
        }
    }

    #[test]
    fn pack_points_mismatched_lengths() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![1.0, 2.0, 3.0])),
                NodeData::Scalars(Arc::new(vec![4.0, 5.0])), // shorter
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::PackPoints, &inputs, &time_ctx(), &mut outputs)
            .unwrap();
        if let Some(NodeData::Points(pts)) = &outputs.data[0] {
            assert_eq!(pts.len(), 2, "should use min length");
        } else {
            panic!("expected Points output");
        }
    }

    #[test]
    fn pack_points_filters_non_finite() {
        let backend = CpuBackend::new().unwrap();
        let inputs = ResolvedInputs {
            data: vec![
                NodeData::Scalars(Arc::new(vec![1.0, f64::INFINITY, 3.0, f64::NAN])),
                NodeData::Scalars(Arc::new(vec![10.0, 20.0, 30.0, 40.0])),
            ],
        };
        let mut outputs = NodeOutputs::new(1);
        backend
            .evaluate_node(&NodeOp::PackPoints, &inputs, &time_ctx(), &mut outputs)
            .unwrap();
        if let Some(NodeData::Points(pts)) = &outputs.data[0] {
            // Only points with finite coords should survive
            assert_eq!(pts.len(), 2, "non-finite points should be filtered out");
            assert_eq!(pts.xs[0], 1.0);
            assert_eq!(pts.xs[1], 3.0);
        } else {
            panic!("expected Points output");
        }
    }

    #[test]
    fn stroke_to_path_uses_eval_context_tolerance() {
        // When the tolerance port is 0 (default), StrokeToPath should use
        // EvalContext.tolerance — the zoom-aware value set by the app.
        let backend = CpuBackend::new().unwrap();

        // A circle path with curves — tolerance differences are visible.
        let circle = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 50.0, y: 0.0 }),
                PathVerb::CubicTo {
                    ctrl1: Point { x: 50.0, y: 27.6 },
                    ctrl2: Point { x: 27.6, y: 50.0 },
                    to: Point { x: 0.0, y: 50.0 },
                },
                PathVerb::CubicTo {
                    ctrl1: Point { x: -27.6, y: 50.0 },
                    ctrl2: Point { x: -50.0, y: 27.6 },
                    to: Point { x: -50.0, y: 0.0 },
                },
                PathVerb::CubicTo {
                    ctrl1: Point { x: -50.0, y: -27.6 },
                    ctrl2: Point { x: -27.6, y: -50.0 },
                    to: Point { x: 0.0, y: -50.0 },
                },
                PathVerb::CubicTo {
                    ctrl1: Point { x: 27.6, y: -50.0 },
                    ctrl2: Point { x: 50.0, y: -27.6 },
                    to: Point { x: 50.0, y: 0.0 },
                },
                PathVerb::Close,
            ],
            closed: true,
        };

        let make_inputs = || ResolvedInputs {
            data: vec![
                NodeData::Path(Arc::new(circle.clone())), // geometry
                NodeData::Scalar(5.0),                     // width
                NodeData::Int(0),                          // cap (Butt)
                NodeData::Int(0),                          // join (Miter)
                NodeData::Scalar(4.0),                     // miter_limit
                NodeData::Scalar(0.0),                     // dash_offset
                NodeData::Scalar(0.0),                     // tolerance = 0 → use context
            ],
        };

        let op = NodeOp::StrokeToPath { dash_pattern: String::new() };

        // Coarse tolerance (zoomed out): e.g. 0.5 / 0.1 = 5.0
        let mut ctx_coarse = EvalContext::default();
        ctx_coarse.tolerance = 5.0;
        let mut out_coarse = NodeOutputs::new(1);
        backend.evaluate_node(&op, &make_inputs(), &ctx_coarse, &mut out_coarse).unwrap();

        // Fine tolerance (zoomed in): e.g. 0.5 / 10.0 = 0.05
        let mut ctx_fine = EvalContext::default();
        ctx_fine.tolerance = 0.05;
        let mut out_fine = NodeOutputs::new(1);
        backend.evaluate_node(&op, &make_inputs(), &ctx_fine, &mut out_fine).unwrap();

        let verbs_coarse = match out_coarse.data[0].as_ref().unwrap() {
            NodeData::Path(p) => p.verbs.len(),
            _ => panic!("expected Path"),
        };
        let verbs_fine = match out_fine.data[0].as_ref().unwrap() {
            NodeData::Path(p) => p.verbs.len(),
            _ => panic!("expected Path"),
        };

        assert!(
            verbs_fine > verbs_coarse,
            "finer context tolerance should produce more verbs: fine={verbs_fine} coarse={verbs_coarse}"
        );
    }
}
