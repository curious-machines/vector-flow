use std::sync::Arc;

use glam::Affine2;

use vector_flow_core::types::{Color, LineCap, LineJoin, NodeData, PathData, Shape, StrokeStyle};

/// Set fill color on geometry. Handles single and batch types.
pub fn set_fill(data: &NodeData, color: Color) -> NodeData {
    match data {
        NodeData::Shape(s) => {
            let mut shape = (**s).clone();
            shape.fill = Some(color);
            NodeData::Shape(Arc::new(shape))
        }
        NodeData::Shapes(shapes) => {
            let updated: Vec<Shape> = shapes
                .iter()
                .map(|s| {
                    let mut shape = s.clone();
                    shape.fill = Some(color);
                    shape
                })
                .collect();
            NodeData::Shapes(Arc::new(updated))
        }
        NodeData::Path(p) => NodeData::Shape(Arc::new(Shape {
            path: (**p).clone(),
            fill: Some(color),
            stroke: None,
            transform: Affine2::IDENTITY,
        })),
        NodeData::Paths(paths) => {
            let shapes: Vec<Shape> = paths
                .iter()
                .map(|p| Shape {
                    path: p.clone(),
                    fill: Some(color),
                    stroke: None,
                    transform: Affine2::IDENTITY,
                })
                .collect();
            NodeData::Shapes(Arc::new(shapes))
        }
        _ => NodeData::Shape(Arc::new(Shape {
            path: PathData::new(),
            fill: Some(color),
            stroke: None,
            transform: Affine2::IDENTITY,
        })),
    }
}

/// Set stroke style on geometry. Handles single and batch types.
pub fn set_stroke(data: &NodeData, color: Color, width: f64) -> NodeData {
    let stroke = StrokeStyle {
        color,
        width: width as f32,
        line_cap: LineCap::Butt,
        line_join: LineJoin::Miter(4.0),
    };

    match data {
        NodeData::Shape(s) => {
            let mut shape = (**s).clone();
            shape.stroke = Some(stroke);
            NodeData::Shape(Arc::new(shape))
        }
        NodeData::Shapes(shapes) => {
            let updated: Vec<Shape> = shapes
                .iter()
                .map(|s| {
                    let mut shape = s.clone();
                    shape.stroke = Some(stroke);
                    shape
                })
                .collect();
            NodeData::Shapes(Arc::new(updated))
        }
        NodeData::Path(p) => NodeData::Shape(Arc::new(Shape {
            path: (**p).clone(),
            fill: None,
            stroke: Some(stroke),
            transform: Affine2::IDENTITY,
        })),
        NodeData::Paths(paths) => {
            let shapes: Vec<Shape> = paths
                .iter()
                .map(|p| Shape {
                    path: p.clone(),
                    fill: None,
                    stroke: Some(stroke),
                    transform: Affine2::IDENTITY,
                })
                .collect();
            NodeData::Shapes(Arc::new(shapes))
        }
        _ => NodeData::Shape(Arc::new(Shape {
            path: PathData::new(),
            fill: None,
            stroke: Some(stroke),
            transform: Affine2::IDENTITY,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::types::{PathVerb, Point};

    #[test]
    fn set_fill_on_path() {
        let path = PathData {
            verbs: vec![PathVerb::MoveTo(Point { x: 0.0, y: 0.0 })],
            closed: false,
        };
        let data = NodeData::Path(Arc::new(path));
        let result = set_fill(&data, Color::WHITE);
        if let NodeData::Shape(s) = result {
            assert_eq!(s.fill, Some(Color::WHITE));
            assert!(s.stroke.is_none());
        } else {
            panic!("expected Shape");
        }
    }

    #[test]
    fn set_stroke_on_shape() {
        let shape = Shape {
            path: PathData::new(),
            fill: Some(Color::BLACK),
            stroke: None,
            transform: Affine2::IDENTITY,
        };
        let data = NodeData::Shape(Arc::new(shape));
        let result = set_stroke(&data, Color::WHITE, 3.0);
        if let NodeData::Shape(s) = result {
            assert_eq!(s.fill, Some(Color::BLACK)); // preserved
            let st = s.stroke.unwrap();
            assert_eq!(st.color, Color::WHITE);
            assert!((st.width - 3.0).abs() < 1e-5);
        } else {
            panic!("expected Shape");
        }
    }

    #[test]
    fn set_fill_on_shapes_batch() {
        let shapes = vec![
            Shape {
                path: PathData::new(),
                fill: None,
                stroke: None,
                transform: Affine2::IDENTITY,
            },
            Shape {
                path: PathData::new(),
                fill: None,
                stroke: None,
                transform: Affine2::IDENTITY,
            },
        ];
        let data = NodeData::Shapes(Arc::new(shapes));
        let result = set_fill(&data, Color::WHITE);
        if let NodeData::Shapes(s) = result {
            assert_eq!(s.len(), 2);
            assert!(s.iter().all(|s| s.fill == Some(Color::WHITE)));
        } else {
            panic!("expected Shapes batch");
        }
    }

    #[test]
    fn set_stroke_on_shapes_batch() {
        let shapes = vec![
            Shape {
                path: PathData::new(),
                fill: Some(Color::BLACK),
                stroke: None,
                transform: Affine2::IDENTITY,
            },
        ];
        let data = NodeData::Shapes(Arc::new(shapes));
        let result = set_stroke(&data, Color::WHITE, 2.0);
        if let NodeData::Shapes(s) = result {
            assert_eq!(s.len(), 1);
            assert_eq!(s[0].fill, Some(Color::BLACK)); // preserved
            assert!(s[0].stroke.is_some());
        } else {
            panic!("expected Shapes batch");
        }
    }
}
