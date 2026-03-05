use egui::Ui;
use glam::Vec2;

use vector_flow_render::overlay::{canvas_paint_callback, CanvasRenderResources};
use vector_flow_render::PreparedScene;

/// Pan/zoom commands accumulated during UI phase,
/// applied to the camera during the paint callback's prepare().
pub struct CameraState {
    pub pan_delta: Vec2,
    pub zoom_delta: f32,
    pub zoom_pos: Vec2,
    pub viewport_size: Vec2,
    pub do_reset: bool,
    pub do_show_all: bool,
    /// Content bounds from the last prepared scene, used by show_all.
    pub content_bounds: Option<(Vec2, Vec2)>,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            pan_delta: Vec2::ZERO,
            zoom_delta: 0.0,
            zoom_pos: Vec2::ZERO,
            viewport_size: Vec2::new(640.0, 480.0),
            do_reset: false,
            do_show_all: false,
            content_bounds: None,
        }
    }
}

/// Inset from the content area edge for overlay toolbar buttons.
pub const TOOLBAR_INSET: f32 = 8.0;

/// Show the canvas preview panel. Returns the screen-space rect of the canvas
/// so the caller can overlay toolbar buttons.
pub fn show_canvas_panel(
    ui: &mut Ui,
    scene: Option<PreparedScene>,
    cam_state: &mut CameraState,
) -> egui::Rect {
    // Update content bounds from the scene we're about to render.
    if let Some(ref s) = scene {
        cam_state.content_bounds = s.bounds();
    }

    let (rect, response) = ui.allocate_exact_size(
        ui.available_size(),
        egui::Sense::click_and_drag(),
    );

    // Update viewport size.
    cam_state.viewport_size = Vec2::new(rect.width(), rect.height());

    // Middle-mouse drag → pan.
    if response.dragged_by(egui::PointerButton::Middle) {
        let d = response.drag_delta();
        cam_state.pan_delta += Vec2::new(d.x, d.y);
    }

    // Scroll → zoom (only when hovering over the canvas).
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.01 {
            cam_state.zoom_delta += scroll * 0.01;
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                let local = pos - rect.left_top();
                cam_state.zoom_pos = Vec2::new(local.x, local.y);
            }
        }
    }

    // Paint callback — the render crate's CanvasCallback handles prepare/paint.
    let callback = canvas_paint_callback(rect, scene);
    ui.painter().add(callback);

    rect
}

/// Apply accumulated camera commands to the render resources.
/// Call this before rendering (e.g. in update(), not inside the callback).
pub fn apply_camera_commands(
    render_state: &egui_wgpu::RenderState,
    cam_state: &mut CameraState,
) {
    let mut resources = render_state.renderer.write();
    let Some(res) = resources.callback_resources.get_mut::<CanvasRenderResources>() else {
        return;
    };

    res.camera.set_viewport(cam_state.viewport_size);

    if cam_state.do_reset {
        res.camera.reset();
        cam_state.do_reset = false;
    }

    if cam_state.do_show_all {
        if let Some((min, max)) = cam_state.content_bounds {
            res.camera.show_all(min, max);
        } else {
            // No content — just reset.
            res.camera.reset();
        }
        cam_state.do_show_all = false;
    }

    if cam_state.pan_delta != Vec2::ZERO {
        res.camera.pan(cam_state.pan_delta);
        cam_state.pan_delta = Vec2::ZERO;
    }

    if cam_state.zoom_delta.abs() > 0.001 {
        let factor = (1.0 + cam_state.zoom_delta).clamp(0.5, 2.0);
        res.camera.zoom_at(cam_state.zoom_pos, factor);
        cam_state.zoom_delta = 0.0;
    }
}
