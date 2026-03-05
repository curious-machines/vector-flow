use std::sync::Arc;

use glam::Affine2;

use vector_flow_core::compute::DslContext;
use vector_flow_core::error::ComputeError;
use vector_flow_core::types::{NodeData, PathData, Shape, TimeContext};

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

/// Compile and execute a DSL expression, returning the scalar result.
pub fn dsl_code(
    source: &str,
    compiler: &mut DslCompiler,
    cache: &DslFunctionCache,
    time_ctx: &TimeContext,
) -> Result<NodeData, ComputeError> {
    let ptr = cache
        .get_or_compile_expr(source, compiler)
        .map_err(|e| ComputeError::DslError(e.to_string()))?;

    let func: ExprFnPtr = unsafe { std::mem::transmute(ptr) };
    let mut ctx = DslContext::new(time_ctx);
    let result = unsafe { func(&mut ctx) };

    Ok(NodeData::Scalar(result))
}
