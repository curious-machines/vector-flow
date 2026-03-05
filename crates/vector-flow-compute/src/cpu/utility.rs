use std::sync::Arc;

use glam::{Affine2, Vec2};

use vector_flow_core::compute::DslContext;
use vector_flow_core::error::ComputeError;
use vector_flow_core::types::{NodeData, PathData, PointBatch, Shape, TimeContext};

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

/// Copy geometry to each position in a PointBatch.
/// Produces a batch of paths/shapes translated to each point.
pub fn copy_to_points(data: &NodeData, points: &PointBatch) -> NodeData {
    let n = points.len();
    if n == 0 {
        return data.clone();
    }

    match data {
        NodeData::Path(base_path) => {
            let mut merged = PathData::new();
            for i in 0..n {
                let offset = Vec2::new(points.xs[i], points.ys[i]);
                let xform = Affine2::from_translation(offset);
                let translated = transforms::transform_path(base_path, &xform);
                merged.verbs.extend_from_slice(&translated.verbs);
            }
            merged.closed = base_path.closed;
            NodeData::Path(Arc::new(merged))
        }
        NodeData::Shape(base_shape) => {
            let shapes: Vec<Shape> = (0..n)
                .map(|i| {
                    let offset = Vec2::new(points.xs[i], points.ys[i]);
                    let xform = Affine2::from_translation(offset);
                    Shape {
                        path: base_shape.path.clone(),
                        fill: base_shape.fill,
                        stroke: base_shape.stroke,
                        transform: xform * base_shape.transform,
                    }
                })
                .collect();
            NodeData::Shapes(Arc::new(shapes))
        }
        NodeData::Paths(base_paths) => {
            let mut all = Vec::new();
            for i in 0..n {
                let offset = Vec2::new(points.xs[i], points.ys[i]);
                let xform = Affine2::from_translation(offset);
                for p in base_paths.iter() {
                    all.push(transforms::transform_path(p, &xform));
                }
            }
            NodeData::Paths(Arc::new(all))
        }
        NodeData::Shapes(base_shapes) => {
            let mut all = Vec::new();
            for i in 0..n {
                let offset = Vec2::new(points.xs[i], points.ys[i]);
                let xform = Affine2::from_translation(offset);
                for s in base_shapes.iter() {
                    all.push(Shape {
                        path: s.path.clone(),
                        fill: s.fill,
                        stroke: s.stroke,
                        transform: xform * s.transform,
                    });
                }
            }
            NodeData::Shapes(Arc::new(all))
        }
        other => other.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::types::{PathVerb, Point};

    fn square_path() -> PathData {
        PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 10.0 }),
                PathVerb::LineTo(Point { x: 0.0, y: 10.0 }),
                PathVerb::Close,
            ],
            closed: true,
        }
    }

    #[test]
    fn copy_to_points_produces_multiple_subpaths() {
        let path = Arc::new(square_path());
        let points = PointBatch {
            xs: vec![0.0, 100.0, 200.0],
            ys: vec![0.0, 0.0, 0.0],
        };

        let result = copy_to_points(&NodeData::Path(path), &points);
        if let NodeData::Path(merged) = result {
            // 3 copies × 5 verbs each = 15 verbs
            assert_eq!(merged.verbs.len(), 15);
            // 3 MoveTo verbs (one per copy)
            let move_count = merged
                .verbs
                .iter()
                .filter(|v| matches!(v, PathVerb::MoveTo(_)))
                .count();
            assert_eq!(move_count, 3);
            // 3 Close verbs
            let close_count = merged
                .verbs
                .iter()
                .filter(|v| matches!(v, PathVerb::Close))
                .count();
            assert_eq!(close_count, 3);
            // First copy at (0,0), second at (100,0), third at (200,0)
            match merged.verbs[0] {
                PathVerb::MoveTo(p) => assert!((p.x - 0.0).abs() < 1e-5),
                _ => panic!("expected MoveTo"),
            }
            match merged.verbs[5] {
                PathVerb::MoveTo(p) => assert!((p.x - 100.0).abs() < 1e-5),
                _ => panic!("expected MoveTo"),
            }
            match merged.verbs[10] {
                PathVerb::MoveTo(p) => assert!((p.x - 200.0).abs() < 1e-5),
                _ => panic!("expected MoveTo"),
            }
        } else {
            panic!("expected Path, got {:?}", result.data_type());
        }
    }

    #[test]
    fn copy_to_points_empty_points_returns_original() {
        let path = Arc::new(square_path());
        let points = PointBatch::new();
        let result = copy_to_points(&NodeData::Path(path.clone()), &points);
        if let NodeData::Path(p) = result {
            assert_eq!(p.verbs.len(), path.verbs.len());
        } else {
            panic!("expected Path");
        }
    }
}
