use std::sync::Arc;

use glam::{Affine2, Vec2};

use vector_flow_core::compute::{DslContext, NodeOutputs, ResolvedInputs};
use vector_flow_core::error::ComputeError;
use vector_flow_core::types::{Color, DataType, NodeData, PathData, PointBatch, Shape, EvalContext};

use vector_flow_dsl::ast::DslType;
use vector_flow_dsl::cache::DslFunctionCache;
use vector_flow_dsl::codegen::{DslCompiler, ExprFnPtr};

use super::transforms;

/// Returns true if two NodeData values are the same "kind" and can be merged.
fn is_mergeable(a: &NodeData, b: &NodeData) -> bool {
    matches!(
        (a, b),
        (NodeData::Path(_), NodeData::Path(_))
            | (NodeData::Path(_), NodeData::Paths(_))
            | (NodeData::Paths(_), NodeData::Path(_))
            | (NodeData::Paths(_), NodeData::Paths(_))
            | (NodeData::Shape(_), NodeData::Shape(_))
            | (NodeData::Shape(_), NodeData::Shapes(_))
            | (NodeData::Shapes(_), NodeData::Shape(_))
            | (NodeData::Shapes(_), NodeData::Shapes(_))
    )
}

/// Merge two compatible geometry inputs into one.
fn merge_pair(a: &NodeData, b: &NodeData) -> NodeData {
    match (a, b) {
        // Path + Path → merged Path
        (NodeData::Path(pa), NodeData::Path(pb)) => {
            let mut merged = PathData::new();
            merged.verbs.extend_from_slice(&pa.verbs);
            merged.verbs.extend_from_slice(&pb.verbs);
            merged.closed = pa.closed || pb.closed;
            NodeData::Path(Arc::new(merged))
        }
        // Shape + Shape → Shapes batch
        (NodeData::Shape(sa), NodeData::Shape(sb)) => {
            NodeData::Shapes(Arc::new(vec![(**sa).clone(), (**sb).clone()]))
        }
        // Shapes + Shape → append
        (NodeData::Shapes(sa), NodeData::Shape(sb)) => {
            let mut out = (**sa).clone();
            out.push((**sb).clone());
            NodeData::Shapes(Arc::new(out))
        }
        // Shape + Shapes → prepend
        (NodeData::Shape(sa), NodeData::Shapes(sb)) => {
            let mut out = vec![(**sa).clone()];
            out.extend_from_slice(sb);
            NodeData::Shapes(Arc::new(out))
        }
        // Shapes + Shapes → concatenate
        (NodeData::Shapes(sa), NodeData::Shapes(sb)) => {
            let mut out = (**sa).clone();
            out.extend_from_slice(sb);
            NodeData::Shapes(Arc::new(out))
        }
        // Paths + Path → append
        (NodeData::Paths(pa), NodeData::Path(pb)) => {
            let mut out = (**pa).clone();
            out.push((**pb).clone());
            NodeData::Paths(Arc::new(out))
        }
        // Path + Paths → prepend
        (NodeData::Path(pa), NodeData::Paths(pb)) => {
            let mut out = vec![(**pa).clone()];
            out.extend_from_slice(pb);
            NodeData::Paths(Arc::new(out))
        }
        // Paths + Paths → concatenate
        (NodeData::Paths(pa), NodeData::Paths(pb)) => {
            let mut out = (**pa).clone();
            out.extend_from_slice(pb);
            NodeData::Paths(Arc::new(out))
        }
        _ => a.clone(),
    }
}

/// Returns true if a NodeData is a trivial default (unconnected port placeholder).
fn is_default_value(d: &NodeData) -> bool {
    matches!(d, NodeData::Scalar(v) if *v == 0.0)
}

/// Merge N inputs into a single NodeData.
/// Compatible geometry types (Path/Paths, Shape/Shapes) are merged together.
/// Incompatible types (Text, Image, etc.) are bundled into a `Mixed` value
/// so `collect_scene` can render all of them.
pub fn merge_n(inputs: &ResolvedInputs) -> NodeData {
    // Filter out unconnected default placeholders.
    let items: Vec<&NodeData> = inputs.data.iter().filter(|d| !is_default_value(d)).collect();
    if items.is_empty() {
        return NodeData::Scalar(0.0);
    }

    // Group mergeable items together; keep others as separate entries.
    let mut groups: Vec<NodeData> = Vec::new();
    for item in &items {
        let mut merged = false;
        for existing in &mut groups {
            if is_mergeable(existing, item) {
                *existing = merge_pair(existing, item);
                merged = true;
                break;
            }
        }
        if !merged {
            groups.push((*item).clone());
        }
    }

    if groups.len() == 1 {
        groups.remove(0)
    } else {
        NodeData::Mixed(Arc::new(groups))
    }
}

/// Duplicate geometry `count` times, applying `step_transform` cumulatively.
pub fn duplicate(data: &NodeData, count: i64, step_transform: &Affine2) -> NodeData {
    let n = count.max(0) as usize;
    if n == 0 {
        return data.clone();
    }

    match data {
        NodeData::Path(base_path) => {
            let mut merged = PathData::new();
            let mut current_xform = Affine2::IDENTITY;
            for _ in 0..n {
                let transformed = transforms::transform_path(base_path, &current_xform);
                merged.verbs.extend_from_slice(&transformed.verbs);
                current_xform = *step_transform * current_xform;
            }
            merged.closed = base_path.closed;
            NodeData::Path(Arc::new(merged))
        }
        NodeData::Shape(base_shape) => {
            let shapes: Vec<Shape> = (0..n)
                .scan(Affine2::IDENTITY, |xform, _| {
                    let s = Shape {
                        path: base_shape.path.clone(),
                        fill: base_shape.fill,
                        stroke: base_shape.stroke.clone(),
                        transform: *xform * base_shape.transform,
                    };
                    *xform = *step_transform * *xform;
                    Some(s)
                })
                .collect();
            NodeData::Shapes(Arc::new(shapes))
        }
        NodeData::Shapes(base_shapes) => {
            let mut all = Vec::new();
            let mut current_xform = Affine2::IDENTITY;
            for _ in 0..n {
                for s in base_shapes.iter() {
                    all.push(Shape {
                        path: s.path.clone(),
                        fill: s.fill,
                        stroke: s.stroke.clone(),
                        transform: current_xform * s.transform,
                    });
                }
                current_xform = *step_transform * current_xform;
            }
            NodeData::Shapes(Arc::new(all))
        }
        NodeData::Paths(base_paths) => {
            let mut all = Vec::new();
            let mut current_xform = Affine2::IDENTITY;
            for _ in 0..n {
                for p in base_paths.iter() {
                    all.push(transforms::transform_path(p, &current_xform));
                }
                current_xform = *step_transform * current_xform;
            }
            NodeData::Paths(Arc::new(all))
        }
        _ => {
            let mut result = data.clone();
            let mut current_xform = Affine2::IDENTITY;
            for _ in 0..n {
                result = transforms::apply_transform(data, &current_xform);
                current_xform = *step_transform * current_xform;
            }
            result
        }
    }
}

/// Copy geometry to each sampled point along a target path.
/// Returns (shapes, tangent_angles_degrees, indices, count).
pub fn copy_to_points(
    data: &NodeData,
    points: &PointBatch,
    tangent_angles: &[f64],
    align: bool,
) -> (NodeData, Vec<f64>, Vec<f64>, f64) {
    let n = points.len();
    if n == 0 {
        return (
            NodeData::Shapes(Arc::new(Vec::new())),
            Vec::new(),
            Vec::new(),
            0.0,
        );
    }

    let indices: Vec<f64> = (0..n).map(|i| i as f64).collect();

    let make_transform = |i: usize| -> Affine2 {
        let pos = Vec2::new(points.xs[i], points.ys[i]);
        let mut xform = Affine2::from_translation(pos);
        if align {
            let angle_rad = (tangent_angles[i] as f32).to_radians();
            xform *= Affine2::from_angle(angle_rad);
        }
        xform
    };

    let shapes = match data {
        NodeData::Shape(base_shape) => {
            let out: Vec<Shape> = (0..n)
                .map(|i| Shape {
                    path: base_shape.path.clone(),
                    fill: base_shape.fill,
                    stroke: base_shape.stroke.clone(),
                    transform: make_transform(i) * base_shape.transform,
                })
                .collect();
            NodeData::Shapes(Arc::new(out))
        }
        NodeData::Path(base_path) => {
            let base = Shape {
                path: (**base_path).clone(),
                fill: None,
                stroke: None,
                transform: Affine2::IDENTITY,
            };
            let out: Vec<Shape> = (0..n)
                .map(|i| Shape {
                    path: base.path.clone(),
                    fill: base.fill,
                    stroke: base.stroke.clone(),
                    transform: make_transform(i),
                })
                .collect();
            NodeData::Shapes(Arc::new(out))
        }
        NodeData::Shapes(base_shapes) => {
            let mut all = Vec::with_capacity(n * base_shapes.len());
            for i in 0..n {
                let xform = make_transform(i);
                for s in base_shapes.iter() {
                    all.push(Shape {
                        path: s.path.clone(),
                        fill: s.fill,
                        stroke: s.stroke.clone(),
                        transform: xform * s.transform,
                    });
                }
            }
            NodeData::Shapes(Arc::new(all))
        }
        _ => {
            // For non-geometry types, apply transforms to whatever we can
            let out: Vec<Shape> = (0..n)
                .map(|i| Shape {
                    path: PathData::new(),
                    fill: None,
                    stroke: None,
                    transform: make_transform(i),
                })
                .collect();
            NodeData::Shapes(Arc::new(out))
        }
    };

    (shapes, tangent_angles.to_vec(), indices, n as f64)
}

/// Convert a DataType to a DslType for the compiler.
fn data_type_to_dsl(dt: &DataType) -> DslType {
    match dt {
        DataType::Int => DslType::Int,
        DataType::Color => DslType::Color,
        _ => DslType::Scalar,
    }
}

/// Number of f64 slots a DslType occupies.
fn slots_for_dsl_type(ty: DslType) -> usize {
    match ty {
        DslType::Color => 4,
        _ => 1,
    }
}

/// Extract a scalar f64 from NodeData for loading into a DslContext slot.
fn node_data_to_f64(data: &NodeData) -> f64 {
    match data {
        NodeData::Scalar(v) => *v,
        NodeData::Int(v) => *v as f64,
        NodeData::Bool(v) => if *v { 1.0 } else { 0.0 },
        _ => 0.0,
    }
}

/// Wrap DSL compilation in catch_unwind to prevent Cranelift panics from
/// crashing the app (e.g. when the user is mid-edit and source is invalid).
fn compile_catching_panics(
    f: impl FnOnce() -> Result<*const u8, vector_flow_dsl::error::DslError>,
) -> Result<*const u8, ComputeError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result.map_err(|e| ComputeError::DslError(e.to_string())),
        Err(_) => Err(ComputeError::DslError(
            "DSL compilation panicked (likely invalid source)".to_string(),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
/// Compile and execute a DSL node script.
/// If the node has no script ports, falls back to expression evaluation.
pub fn dsl_code(
    source: &str,
    script_inputs: &[(String, DataType)],
    script_outputs: &[(String, DataType)],
    inputs: &ResolvedInputs,
    compiler: &mut DslCompiler,
    cache: &DslFunctionCache,
    time_ctx: &EvalContext,
    outputs: &mut NodeOutputs,
) -> Result<(), ComputeError> {
    if source.trim().is_empty() {
        return Ok(());
    }

    if script_inputs.is_empty() && script_outputs.is_empty() {
        // Legacy: no ports defined, treat as simple expression.
        let ptr = match compile_catching_panics(|| {
            cache.get_or_compile_expr(source, compiler)
        }) {
            Ok(p) => p,
            Err(e) => {
                outputs.error = Some(e.to_string());
                return Ok(());
            }
        };
        let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
        let mut ctx = DslContext::new(time_ctx);
        let result = unsafe { func(&mut ctx) };
        if !outputs.data.is_empty() {
            outputs.data[0] = Some(NodeData::Scalar(result));
        }
        return Ok(());
    }

    // Build compiler port lists.
    let dsl_inputs: Vec<(String, DslType)> = script_inputs
        .iter()
        .map(|(name, dt)| (name.clone(), data_type_to_dsl(dt)))
        .collect();
    let dsl_outputs: Vec<(String, DslType)> = script_outputs
        .iter()
        .map(|(name, dt)| (name.clone(), data_type_to_dsl(dt)))
        .collect();

    // Compile (cached). On error, output defaults and report the error.
    let ptr = match compile_catching_panics(|| {
        cache.get_or_compile_node_script(source, &dsl_inputs, &dsl_outputs, compiler)
    }) {
        Ok(p) => p,
        Err(e) => {
            outputs.error = Some(e.to_string());
            return Ok(());
        }
    };

    let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
    let mut ctx = DslContext::new(time_ctx);

    // Load inputs into ctx.slots (Color takes 4 slots, others take 1).
    let mut slot_idx = 0usize;
    for (i, (_name, dt)) in script_inputs.iter().enumerate() {
        if let Some(data) = inputs.data.get(i) {
            load_value_into_slots(&mut ctx, slot_idx, data);
        }
        slot_idx += slots_for_dsl_type(data_type_to_dsl(dt));
    }
    let input_slot_count = slot_idx;

    // Allocate overflow if needed.
    let output_slot_count: usize = script_outputs.iter()
        .map(|(_, dt)| slots_for_dsl_type(data_type_to_dsl(dt)))
        .sum();
    let total_slots = input_slot_count + output_slot_count;
    let _overflow = if total_slots > 16 {
        Some(ctx.alloc_overflow(total_slots - 16))
    } else {
        None
    };

    // Execute.
    unsafe { func(&mut ctx) };

    // Read outputs from ctx.slots[input_slot_count..].
    let mut out_slot = input_slot_count;
    for (i, (_name, dt)) in script_outputs.iter().enumerate() {
        if i < outputs.data.len() {
            outputs.data[i] = Some(read_value_from_slots(&ctx, out_slot, dt));
        }
        out_slot += slots_for_dsl_type(data_type_to_dsl(dt));
    }

    Ok(())
}

/// Load a NodeData value into DslContext slots starting at `slot_idx`.
/// Returns how many slots were consumed.
fn load_value_into_slots(ctx: &mut DslContext, slot_idx: usize, data: &NodeData) -> usize {
    match data {
        NodeData::Color(c) => {
            ctx.slots[slot_idx] = c.r as f64;
            ctx.slots[slot_idx + 1] = c.g as f64;
            ctx.slots[slot_idx + 2] = c.b as f64;
            ctx.slots[slot_idx + 3] = c.a as f64;
            4
        }
        _ => {
            ctx.slots[slot_idx] = node_data_to_f64(data);
            1
        }
    }
}

/// Read a value from DslContext slots based on the output DataType.
fn read_value_from_slots(ctx: &DslContext, slot_idx: usize, dt: &DataType) -> NodeData {
    match dt {
        DataType::Color => {
            let c = Color {
                r: ctx.slots[slot_idx] as f32,
                g: ctx.slots[slot_idx + 1] as f32,
                b: ctx.slots[slot_idx + 2] as f32,
                a: ctx.slots[slot_idx + 3] as f32,
            };
            NodeData::Color(c)
        }
        DataType::Int => NodeData::Int(ctx.slots[slot_idx] as i64),
        DataType::Bool => NodeData::Bool(ctx.slots[slot_idx] != 0.0),
        _ => NodeData::Scalar(ctx.slots[slot_idx]),
    }
}

/// Unwrap a batch NodeData into its element count.
fn batch_len(data: &NodeData) -> usize {
    match data {
        NodeData::Scalars(v) => v.len(),
        NodeData::Ints(v) => v.len(),
        NodeData::Colors(v) => v.len(),
        NodeData::Shapes(v) => v.len(),
        NodeData::Paths(v) => v.len(),
        // Single values treated as batch of 1
        _ => 1,
    }
}

/// Get the i-th element from a batch as a NodeData single value.
fn batch_element(data: &NodeData, i: usize) -> NodeData {
    match data {
        NodeData::Scalars(v) => NodeData::Scalar(v[i]),
        NodeData::Ints(v) => NodeData::Int(v[i]),
        NodeData::Colors(v) => NodeData::Color(v[i]),
        NodeData::Shapes(v) => NodeData::Shape(Arc::new(v[i].clone())),
        NodeData::Paths(v) => NodeData::Path(Arc::new(v[i].clone())),
        // Single value: return as-is regardless of index
        other => other.clone(),
    }
}

/// Collect per-element results into a batch NodeData.
fn collect_into_batch(results: Vec<NodeData>, dt: &DataType) -> NodeData {
    match dt {
        DataType::Scalar => {
            let vals: Vec<f64> = results.iter().map(|r| match r {
                NodeData::Scalar(v) => *v,
                _ => 0.0,
            }).collect();
            NodeData::Scalars(Arc::new(vals))
        }
        DataType::Int => {
            let vals: Vec<i64> = results.iter().map(|r| match r {
                NodeData::Int(v) => *v,
                _ => 0,
            }).collect();
            NodeData::Ints(Arc::new(vals))
        }
        DataType::Color => {
            let vals: Vec<Color> = results.iter().map(|r| match r {
                NodeData::Color(c) => *c,
                _ => Color::BLACK,
            }).collect();
            NodeData::Colors(Arc::new(vals))
        }
        // Default: collect as scalars
        _ => {
            let vals: Vec<f64> = results.iter().map(|r| match r {
                NodeData::Scalar(v) => *v,
                NodeData::Int(v) => *v as f64,
                _ => 0.0,
            }).collect();
            NodeData::Scalars(Arc::new(vals))
        }
    }
}

/// Map: iterate a batch, run DSL code per element, collect results.
#[allow(clippy::too_many_arguments)]
pub fn map_batch(
    source: &str,
    script_inputs: &[(String, DataType)],
    script_outputs: &[(String, DataType)],
    inputs: &ResolvedInputs,
    compiler: &mut DslCompiler,
    cache: &DslFunctionCache,
    time_ctx: &EvalContext,
    outputs: &mut NodeOutputs,
) -> Result<(), ComputeError> {
    if source.trim().is_empty() {
        return Ok(());
    }

    // The first graph input is the batch to iterate over.
    let batch_data = &inputs.data[0];
    let count = batch_len(batch_data);

    // Build DSL compiler port lists.
    let dsl_inputs: Vec<(String, DslType)> = script_inputs
        .iter()
        .map(|(name, dt)| (name.clone(), data_type_to_dsl(dt)))
        .collect();
    let dsl_outputs: Vec<(String, DslType)> = script_outputs
        .iter()
        .map(|(name, dt)| (name.clone(), data_type_to_dsl(dt)))
        .collect();

    // Compile (cached).
    let ptr = match compile_catching_panics(|| {
        cache.get_or_compile_node_script(source, &dsl_inputs, &dsl_outputs, compiler)
    }) {
        Ok(p) => p,
        Err(e) => {
            outputs.error = Some(e.to_string());
            return Ok(());
        }
    };

    let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };

    // Compute slot layout for inputs.
    let input_slot_count: usize = dsl_inputs.iter().map(|(_, t)| slots_for_dsl_type(*t)).sum();
    let output_slot_count: usize = dsl_outputs.iter().map(|(_, t)| slots_for_dsl_type(*t)).sum();
    let total_slots = input_slot_count + output_slot_count;

    // Single context, reused across iterations.
    let mut ctx = DslContext::new(time_ctx);
    let _overflow = if total_slots > 16 {
        Some(ctx.alloc_overflow(total_slots - 16))
    } else {
        None
    };

    // Pre-load extra graph inputs (beyond the batch) into their script input slots.
    // Script inputs: element (slot 0), then others (index, count, user extras).
    // Graph inputs: batch (port 0), then user extras (port 1+).
    // The extra graph inputs map to script inputs that are NOT element/index/count.
    // We compute slot offsets for each script input.
    let mut script_input_slots: Vec<usize> = Vec::with_capacity(script_inputs.len());
    {
        let mut slot = 0;
        for (_, dt) in script_inputs.iter() {
            script_input_slots.push(slot);
            slot += slots_for_dsl_type(data_type_to_dsl(dt));
        }
    }

    // Find which script inputs are "element", "index", "count" by name.
    let element_idx = script_inputs.iter().position(|(n, _)| n == "element");
    let index_idx = script_inputs.iter().position(|(n, _)| n == "index");
    let count_idx = script_inputs.iter().position(|(n, _)| n == "count");

    // Extra inputs: script inputs that are not element/index/count.
    // These get their values from graph input ports 1, 2, 3, ...
    let mut graph_port = 1usize; // port 0 is the batch
    for (si, (name, _dt)) in script_inputs.iter().enumerate() {
        if name == "element" || name == "index" || name == "count" {
            continue;
        }
        if graph_port < inputs.data.len() {
            let slot = script_input_slots[si];
            load_value_into_slots(&mut ctx, slot, &inputs.data[graph_port]);
        }
        graph_port += 1;
    }

    // Load count once (it doesn't change per iteration).
    if let Some(ci) = count_idx {
        ctx.slots[script_input_slots[ci]] = count as f64;
    }

    // Output slot offset.
    let output_slot_offset = input_slot_count;

    // Iterate.
    let mut results = Vec::with_capacity(count);
    for i in 0..count {
        // Load element.
        if let Some(ei) = element_idx {
            let elem = batch_element(batch_data, i);
            load_value_into_slots(&mut ctx, script_input_slots[ei], &elem);
        }

        // Load index.
        if let Some(ii) = index_idx {
            ctx.slots[script_input_slots[ii]] = i as f64;
        }

        // Execute.
        unsafe { func(&mut ctx) };

        // Read first output.
        if !script_outputs.is_empty() {
            let result = read_value_from_slots(&ctx, output_slot_offset, &script_outputs[0].1);
            results.push(result);
        }
    }

    // Write collected output.
    if !outputs.data.is_empty() && !script_outputs.is_empty() {
        let out_dt = &script_outputs[0].1;
        outputs.data[0] = Some(collect_into_batch(results, out_dt));
    }

    // Handle additional outputs (if any).
    // For now, only the first output is collected into a batch.

    Ok(())
}
