use std::collections::HashSet;

use glam::Affine2;

use vector_flow_core::scheduler::EvalResult;
use vector_flow_core::types::{NodeData, NodeId, PathVerb, Shape};

// ---------------------------------------------------------------------------
// Overlay data structures
// ---------------------------------------------------------------------------

/// A shape's path verbs + transform, for drawing control point overlays.
pub struct OverlayPath {
    pub verbs: Vec<PathVerb>,
    pub transform: Affine2,
}

/// All overlay data collected for the visible/selected nodes.
pub struct OverlayData {
    pub paths: Vec<OverlayPath>,
    pub points: Vec<(f32, f32)>,
}

// ---------------------------------------------------------------------------
// Screen-space constants
// ---------------------------------------------------------------------------

/// Radius in screen pixels for on-curve point markers.
const ON_CURVE_RADIUS: f32 = 3.5;
/// Half-size in screen pixels for the control point square.
const CTRL_SQUARE_HALF: f32 = 3.0;
/// Radius in screen pixels for raw point markers (PointBatch).
const POINT_MARKER_RADIUS: f32 = 3.5;
/// Width in screen pixels for handle lines.
const HANDLE_LINE_WIDTH: f32 = 1.0;

const ON_CURVE_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(200, 220, 255, 220);
const CTRL_POINT_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(255, 180, 60, 220);
const HANDLE_LINE_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(255, 180, 60, 100);
const POINT_MARKER_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(180, 180, 180, 200);

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

/// Collect overlay data (path verbs + raw points) for the given set of nodes.
pub fn collect_overlay_data(
    eval: &EvalResult,
    nodes: &HashSet<NodeId>,
) -> OverlayData {
    let mut paths = Vec::new();
    let mut points = Vec::new();

    for node_id in nodes {
        if let Some(outputs) = eval.outputs.get(node_id) {
            for data in outputs {
                collect_node_data_overlay(data, &mut paths, &mut points);
            }
        }
    }

    OverlayData { paths, points }
}

fn collect_node_data_overlay(
    data: &NodeData,
    paths: &mut Vec<OverlayPath>,
    points: &mut Vec<(f32, f32)>,
) {
    match data {
        NodeData::Shape(s) => {
            push_shape_overlay(s, paths);
        }
        NodeData::Shapes(ss) => {
            for s in ss.iter() {
                push_shape_overlay(s, paths);
            }
        }
        NodeData::Path(p) => {
            paths.push(OverlayPath {
                verbs: p.verbs.clone(),
                transform: Affine2::IDENTITY,
            });
        }
        NodeData::Paths(pp) => {
            for p in pp.iter() {
                paths.push(OverlayPath {
                    verbs: p.verbs.clone(),
                    transform: Affine2::IDENTITY,
                });
            }
        }
        NodeData::Points(pts) => {
            for (&x, &y) in pts.xs.iter().zip(pts.ys.iter()) {
                points.push((x, y));
            }
        }
        NodeData::Mixed(items) => {
            for item in items.iter() {
                collect_node_data_overlay(item, paths, points);
            }
        }
        _ => {}
    }
}

fn push_shape_overlay(shape: &Shape, paths: &mut Vec<OverlayPath>) {
    if shape.path.verbs.is_empty() {
        return;
    }
    paths.push(OverlayPath {
        verbs: shape.path.verbs.clone(),
        transform: shape.transform,
    });
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Convert a world-space point to screen-space, applying the shape transform
/// and camera projection.
fn world_to_screen(
    wx: f32,
    wy: f32,
    transform: &Affine2,
    cam_center: glam::Vec2,
    zoom: f32,
    vp_center: egui::Pos2,
) -> egui::Pos2 {
    let world = transform.transform_point2(glam::Vec2::new(wx, wy));
    egui::pos2(
        (world.x - cam_center.x) * zoom + vp_center.x,
        -(world.y - cam_center.y) * zoom + vp_center.y,
    )
}

/// Draw the overlay (control points, handles, point markers) onto the canvas.
pub fn draw_overlay(
    painter: &egui::Painter,
    data: &OverlayData,
    cam_center: glam::Vec2,
    zoom: f32,
    canvas_rect: egui::Rect,
) {
    let vp_center = canvas_rect.center();

    // Draw path control points and handles.
    for path in &data.paths {
        draw_path_overlay(painter, path, cam_center, zoom, vp_center);
    }

    // Draw raw point markers.
    for &(x, y) in &data.points {
        let screen = world_to_screen(x, y, &Affine2::IDENTITY, cam_center, zoom, vp_center);
        if canvas_rect.contains(screen) {
            painter.circle_filled(screen, POINT_MARKER_RADIUS, POINT_MARKER_COLOR);
        }
    }
}

fn draw_path_overlay(
    painter: &egui::Painter,
    path: &OverlayPath,
    cam_center: glam::Vec2,
    zoom: f32,
    vp_center: egui::Pos2,
) {
    let tf = &path.transform;
    let mut current = egui::Pos2::ZERO;

    for verb in &path.verbs {
        match *verb {
            PathVerb::MoveTo(p) => {
                let screen = world_to_screen(p.x, p.y, tf, cam_center, zoom, vp_center);
                draw_on_curve(painter, screen);
                current = screen;
            }
            PathVerb::LineTo(p) => {
                let screen = world_to_screen(p.x, p.y, tf, cam_center, zoom, vp_center);
                draw_on_curve(painter, screen);
                current = screen;
            }
            PathVerb::QuadTo { ctrl, to } => {
                let ctrl_s = world_to_screen(ctrl.x, ctrl.y, tf, cam_center, zoom, vp_center);
                let to_s = world_to_screen(to.x, to.y, tf, cam_center, zoom, vp_center);

                // Handle lines: prev → ctrl, ctrl → to
                draw_handle_line(painter, current, ctrl_s);
                draw_handle_line(painter, ctrl_s, to_s);

                draw_ctrl_point(painter, ctrl_s);
                draw_on_curve(painter, to_s);
                current = to_s;
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                let c1 = world_to_screen(ctrl1.x, ctrl1.y, tf, cam_center, zoom, vp_center);
                let c2 = world_to_screen(ctrl2.x, ctrl2.y, tf, cam_center, zoom, vp_center);
                let to_s = world_to_screen(to.x, to.y, tf, cam_center, zoom, vp_center);

                // Handle lines: prev → ctrl1, ctrl2 → to
                draw_handle_line(painter, current, c1);
                draw_handle_line(painter, c2, to_s);

                draw_ctrl_point(painter, c1);
                draw_ctrl_point(painter, c2);
                draw_on_curve(painter, to_s);
                current = to_s;
            }
            PathVerb::Close => {}
        }
    }
}

/// Draw an on-curve point marker (filled circle).
fn draw_on_curve(painter: &egui::Painter, pos: egui::Pos2) {
    painter.circle_filled(pos, ON_CURVE_RADIUS, ON_CURVE_COLOR);
}

/// Draw an off-curve control point marker (filled square).
fn draw_ctrl_point(painter: &egui::Painter, pos: egui::Pos2) {
    let half = CTRL_SQUARE_HALF;
    let rect = egui::Rect::from_center_size(pos, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_filled(rect, 0.0, CTRL_POINT_COLOR);
}

/// Draw a handle line between two screen-space points.
fn draw_handle_line(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2) {
    painter.line_segment(
        [a, b],
        egui::Stroke::new(HANDLE_LINE_WIDTH, HANDLE_LINE_COLOR),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use glam::Affine2;
    use vector_flow_core::types::{
        Color, PathData, PathVerb, Point, PointBatch, Shape,
    };

    #[test]
    fn collect_overlay_extracts_shape_paths() {
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
                PathVerb::CubicTo {
                    ctrl1: Point { x: 10.0, y: 5.0 },
                    ctrl2: Point { x: 5.0, y: 10.0 },
                    to: Point { x: 0.0, y: 10.0 },
                },
                PathVerb::Close,
            ],
            closed: true,
        };
        let shape = Shape {
            path: Arc::new(path),
            fill: Some(Color::WHITE),
            stroke: None,
            transform: Affine2::IDENTITY,
        };
        let mut outputs = HashMap::new();
        outputs.insert(NodeId(1), vec![NodeData::Shape(Arc::new(shape))]);
        let eval = EvalResult { outputs, errors: HashMap::new() };

        let selected: HashSet<NodeId> = [NodeId(1)].into();
        let data = collect_overlay_data(&eval, &selected);

        assert_eq!(data.paths.len(), 1);
        assert_eq!(data.paths[0].verbs.len(), 4);
        assert!(data.points.is_empty());
    }

    #[test]
    fn collect_overlay_extracts_raw_points() {
        let pts = PointBatch {
            xs: vec![1.0, 2.0, 3.0],
            ys: vec![4.0, 5.0, 6.0],
        };
        let mut outputs = HashMap::new();
        outputs.insert(NodeId(2), vec![NodeData::Points(Arc::new(pts))]);
        let eval = EvalResult { outputs, errors: HashMap::new() };

        let selected: HashSet<NodeId> = [NodeId(2)].into();
        let data = collect_overlay_data(&eval, &selected);

        assert!(data.paths.is_empty());
        assert_eq!(data.points.len(), 3);
        assert_eq!(data.points[0], (1.0, 4.0));
        assert_eq!(data.points[2], (3.0, 6.0));
    }

    #[test]
    fn collect_overlay_filters_by_selected_nodes() {
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }),
                PathVerb::LineTo(Point { x: 10.0, y: 0.0 }),
            ],
            closed: false,
        };
        let shape = Shape {
            path: Arc::new(path),
            fill: None,
            stroke: None,
            transform: Affine2::IDENTITY,
        };
        let mut outputs = HashMap::new();
        outputs.insert(NodeId(1), vec![NodeData::Shape(Arc::new(shape))]);
        outputs.insert(
            NodeId(2),
            vec![NodeData::Points(Arc::new(PointBatch {
                xs: vec![0.0],
                ys: vec![0.0],
            }))],
        );
        let eval = EvalResult { outputs, errors: HashMap::new() };

        // Only select node 1 — node 2's points should be excluded.
        let selected: HashSet<NodeId> = [NodeId(1)].into();
        let data = collect_overlay_data(&eval, &selected);

        assert_eq!(data.paths.len(), 1);
        assert!(data.points.is_empty());
    }

    #[test]
    fn world_to_screen_matches_canvas_panel_formula() {
        let cam_center = glam::Vec2::new(10.0, 20.0);
        let zoom = 2.0;
        let vp_center = egui::pos2(400.0, 300.0);
        let tf = Affine2::IDENTITY;

        // World point (15, 25):
        //   screen_x = (15 - 10) * 2 + 400 = 410
        //   screen_y = -(25 - 20) * 2 + 300 = 290
        let screen = world_to_screen(15.0, 25.0, &tf, cam_center, zoom, vp_center);
        assert!((screen.x - 410.0).abs() < 1e-4);
        assert!((screen.y - 290.0).abs() < 1e-4);
    }

    #[test]
    fn world_to_screen_applies_shape_transform() {
        let cam_center = glam::Vec2::ZERO;
        let zoom = 1.0;
        let vp_center = egui::pos2(100.0, 100.0);
        // Translate by (5, 10)
        let tf = Affine2::from_translation(glam::Vec2::new(5.0, 10.0));

        // World point (0, 0) after transform = (5, 10)
        //   screen_x = (5 - 0) * 1 + 100 = 105
        //   screen_y = -(10 - 0) * 1 + 100 = 90
        let screen = world_to_screen(0.0, 0.0, &tf, cam_center, zoom, vp_center);
        assert!((screen.x - 105.0).abs() < 1e-4);
        assert!((screen.y - 90.0).abs() < 1e-4);
    }
}
