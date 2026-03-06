use std::sync::Arc;

use glam::Affine2;

use vector_flow_core::compute::{DslContext, NodeOutputs, ResolvedInputs};
use vector_flow_core::error::ComputeError;
use vector_flow_core::types::{DataType, NodeData, PathData, Shape, TimeContext};

use vector_flow_dsl::ast::DslType;
use vector_flow_dsl::cache::DslFunctionCache;
use vector_flow_dsl::codegen::{DslCompiler, ExprFnPtr};

use super::transforms;

/// Merge two geometry inputs into one. Handles combinations of Path, Paths,
/// Shape, and Shapes by collecting into the appropriate batch type.
pub fn merge(a: &NodeData, b: &NodeData) -> NodeData {
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
        // Fallback
        _ => a.clone(),
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
                        stroke: base_shape.stroke,
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
                        stroke: s.stroke,
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

/// Convert a DataType to a DslType for the compiler (phase 1: Scalar and Int only).
fn data_type_to_dsl(dt: &DataType) -> DslType {
    match dt {
        DataType::Int => DslType::Int,
        _ => DslType::Scalar,
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

/// Convert a DslContext slot value back to NodeData based on port DataType.
fn f64_to_node_data(val: f64, dt: &DataType) -> NodeData {
    match dt {
        DataType::Int => NodeData::Int(val as i64),
        DataType::Bool => NodeData::Bool(val != 0.0),
        _ => NodeData::Scalar(val),
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
    time_ctx: &TimeContext,
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

    // Load inputs into ctx.slots[0..n_inputs].
    for (i, (_name, _dt)) in script_inputs.iter().enumerate() {
        if i < 8 {
            ctx.slots[i] = inputs.data.get(i).map(node_data_to_f64).unwrap_or(0.0);
        }
    }

    // Allocate overflow if needed (inputs + outputs > 8).
    let total_slots = script_inputs.len() + script_outputs.len();
    let _overflow = if total_slots > 8 {
        Some(ctx.alloc_overflow(total_slots - 8))
    } else {
        None
    };

    // Execute.
    unsafe { func(&mut ctx) };

    // Read outputs from ctx.slots[n_inputs..].
    let n_inputs = script_inputs.len();
    for (i, (_name, dt)) in script_outputs.iter().enumerate() {
        let slot_idx = n_inputs + i;
        let val = if slot_idx < 8 {
            ctx.slots[slot_idx]
        } else {
            // Read from overflow (not yet supported beyond 8 slots)
            0.0
        };
        if i < outputs.data.len() {
            outputs.data[i] = Some(f64_to_node_data(val, dt));
        }
    }

    Ok(())
}
