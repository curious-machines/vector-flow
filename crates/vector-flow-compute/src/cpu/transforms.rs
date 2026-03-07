use std::sync::Arc;

use glam::{Affine2, Vec2};

use vector_flow_core::types::{ImageInstance, NodeData, PathData, PathVerb, Point, PointBatch, Shape, TextInstance};

/// Apply an Affine2 transform to a Point.
fn transform_point(p: Point, xform: &Affine2) -> Point {
    let v = xform.transform_point2(Vec2::new(p.x, p.y));
    Point { x: v.x, y: v.y }
}

/// Apply an Affine2 transform to every vertex in a PathData.
pub fn transform_path(path: &PathData, xform: &Affine2) -> PathData {
    let verbs = path
        .verbs
        .iter()
        .map(|v| match *v {
            PathVerb::MoveTo(p) => PathVerb::MoveTo(transform_point(p, xform)),
            PathVerb::LineTo(p) => PathVerb::LineTo(transform_point(p, xform)),
            PathVerb::QuadTo { ctrl, to } => PathVerb::QuadTo {
                ctrl: transform_point(ctrl, xform),
                to: transform_point(to, xform),
            },
            PathVerb::CubicTo { ctrl1, ctrl2, to } => PathVerb::CubicTo {
                ctrl1: transform_point(ctrl1, xform),
                ctrl2: transform_point(ctrl2, xform),
                to: transform_point(to, xform),
            },
            PathVerb::Close => PathVerb::Close,
        })
        .collect();

    PathData {
        verbs,
        closed: path.closed,
    }
}

/// Apply an Affine2 transform to a PointBatch.
pub fn transform_point_batch(pts: &PointBatch, xform: &Affine2) -> PointBatch {
    let mut xs = Vec::with_capacity(pts.len());
    let mut ys = Vec::with_capacity(pts.len());
    for i in 0..pts.len() {
        let v = xform.transform_point2(Vec2::new(pts.xs[i], pts.ys[i]));
        xs.push(v.x);
        ys.push(v.y);
    }
    PointBatch { xs, ys }
}

/// Apply a transform to any geometry NodeData variant.
/// Non-geometry types pass through unchanged.
pub fn apply_transform(data: &NodeData, xform: &Affine2) -> NodeData {
    match data {
        NodeData::Path(path) => {
            NodeData::Path(Arc::new(transform_path(path, xform)))
        }
        NodeData::Points(pts) => {
            NodeData::Points(Arc::new(transform_point_batch(pts, xform)))
        }
        NodeData::Shape(shape) => {
            let new_shape = Shape {
                path: shape.path.clone(),
                fill: shape.fill,
                stroke: shape.stroke.clone(),
                transform: *xform * shape.transform,
            };
            NodeData::Shape(Arc::new(new_shape))
        }
        NodeData::Paths(paths) => {
            let transformed: Vec<PathData> = paths
                .iter()
                .map(|p| transform_path(p, xform))
                .collect();
            NodeData::Paths(Arc::new(transformed))
        }
        NodeData::Shapes(shapes) => {
            let transformed: Vec<Shape> = shapes
                .iter()
                .map(|s| Shape {
                    path: s.path.clone(),
                    fill: s.fill,
                    stroke: s.stroke.clone(),
                    transform: *xform * s.transform,
                })
                .collect();
            NodeData::Shapes(Arc::new(transformed))
        }
        NodeData::Image(img) => {
            NodeData::Image(Arc::new(ImageInstance {
                image: Arc::clone(&img.image),
                transform: *xform * img.transform,
                opacity: img.opacity,
            }))
        }
        NodeData::Text(txt) => {
            NodeData::Text(Arc::new(TextInstance {
                text: txt.text.clone(),
                style: txt.style.clone(),
                color: txt.color,
                transform: *xform * txt.transform,
                opacity: txt.opacity,
                layout: Arc::clone(&txt.layout),
            }))
        }
        NodeData::Mixed(items) => {
            let transformed: Vec<NodeData> = items.iter()
                .map(|item| apply_transform(item, xform))
                .collect();
            NodeData::Mixed(Arc::new(transformed))
        }
        other => other.clone(),
    }
}

/// Flatten a Shape into a Path by baking its transform into the vertices.
pub fn bake_shape_to_path(shape: &Shape) -> PathData {
    if shape.transform == Affine2::IDENTITY {
        (*shape.path).clone()
    } else {
        transform_path(&shape.path, &shape.transform)
    }
}

/// Translate: Affine2::from_translation(offset)
pub fn translate(data: &NodeData, offset: Vec2) -> NodeData {
    let xform = Affine2::from_translation(offset);
    apply_transform(data, &xform)
}

/// Rotate by `angle_degrees` around `center`.
pub fn rotate(data: &NodeData, angle_degrees: f64, center: Vec2) -> NodeData {
    let angle_rad = (angle_degrees as f32).to_radians();
    let xform = Affine2::from_translation(center)
        * Affine2::from_angle(angle_rad)
        * Affine2::from_translation(-center);
    apply_transform(data, &xform)
}

/// Scale by `factor` around `center`.
pub fn scale(data: &NodeData, factor: Vec2, center: Vec2) -> NodeData {
    let xform = Affine2::from_translation(center)
        * Affine2::from_scale(factor)
        * Affine2::from_translation(-center);
    apply_transform(data, &xform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_offsets_correctly() {
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
            ],
            closed: false,
        };
        let data = NodeData::Path(Arc::new(path));
        let result = translate(&data, Vec2::new(5.0, 3.0));
        if let NodeData::Path(p) = result {
            match p.verbs[0] {
                PathVerb::MoveTo(pt) => {
                    assert!((pt.x - 5.0).abs() < 1e-5);
                    assert!((pt.y - 3.0).abs() < 1e-5);
                }
                _ => panic!("expected MoveTo"),
            }
            match p.verbs[1] {
                PathVerb::LineTo(pt) => {
                    assert!((pt.x - 15.0).abs() < 1e-5);
                    assert!((pt.y - 3.0).abs() < 1e-5);
                }
                _ => panic!("expected LineTo"),
            }
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn rotate_90_degrees() {
        let path = PathData {
            verbs: vec![PathVerb::MoveTo(Point { x: 10.0, y: 0.0 })],
            closed: false,
        };
        let data = NodeData::Path(Arc::new(path));
        let result = rotate(&data, 90.0, Vec2::ZERO);
        if let NodeData::Path(p) = result {
            match p.verbs[0] {
                PathVerb::MoveTo(pt) => {
                    assert!(pt.x.abs() < 1e-4, "expected ~0, got {}", pt.x);
                    assert!((pt.y - 10.0).abs() < 1e-4, "expected ~10, got {}", pt.y);
                }
                _ => panic!("expected MoveTo"),
            }
        } else {
            panic!("expected Path");
        }
    }

    #[test]
    fn scale_doubles() {
        let pts = PointBatch {
            xs: vec![1.0, 2.0],
            ys: vec![3.0, 4.0],
        };
        let data = NodeData::Points(Arc::new(pts));
        let result = scale(&data, Vec2::new(2.0, 2.0), Vec2::ZERO);
        if let NodeData::Points(p) = result {
            assert!((p.xs[0] - 2.0).abs() < 1e-5);
            assert!((p.ys[0] - 6.0).abs() < 1e-5);
        } else {
            panic!("expected Points");
        }
    }
}
