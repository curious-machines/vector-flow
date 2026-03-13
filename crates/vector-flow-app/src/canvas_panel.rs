use std::sync::Arc;

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
    /// Canvas size from project settings, used by show_all to fit canvas to view.
    pub canvas_size: Option<(f32, f32)>,
    /// When set, override the camera zoom to this absolute value (centered on viewport).
    pub set_zoom: Option<f32>,
    /// Content bounds from the last prepared scene, used by show_all.
    pub content_bounds: Option<(Vec2, Vec2)>,
    /// Current camera center in world coordinates (read back after apply).
    pub current_center: Vec2,
    /// Current camera zoom level (read back after apply).
    pub current_zoom: f32,
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
            canvas_size: None,
            set_zoom: None,
            content_bounds: None,
            current_center: Vec2::ZERO,
            current_zoom: 1.0,
        }
    }
}

/// Inset from the content area edge for overlay toolbar buttons.
pub const TOOLBAR_INSET: f32 = 8.0;

/// Canvas dimensions and background color for drawing the project canvas rect.
pub struct CanvasBackground {
    pub width: f32,
    pub height: f32,
    /// Background color. `None` = transparent (no rect drawn).
    pub color: Option<egui::Color32>,
}

/// Show the canvas preview panel. Returns the screen-space rect of the canvas
/// so the caller can overlay toolbar buttons.
pub fn show_canvas_panel(
    ui: &mut Ui,
    scene: Option<Arc<PreparedScene>>,
    cam_state: &mut CameraState,
    background: Option<&CanvasBackground>,
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

    // Draw project canvas background rect (world-space → screen-space).
    if let Some(bg) = background {
        if let Some(color) = bg.color {
            let half_w = bg.width / 2.0;
            let half_h = bg.height / 2.0;
            let zoom = cam_state.current_zoom;
            let center = cam_state.current_center;
            let vp_center = rect.center();

            // World to screen:
            //   screen_x = (world_x - cam_center_x) * zoom + viewport_center_x
            //   screen_y = -(world_y - cam_center_y) * zoom + viewport_center_y  (Y-flip)
            // Canvas top-left in world: (-half_w, +half_h)
            // Canvas bottom-right in world: (+half_w, -half_h)
            let screen_min = egui::pos2(
                (-half_w - center.x) * zoom + vp_center.x,
                -(half_h - center.y) * zoom + vp_center.y,
            );
            let screen_max = egui::pos2(
                (half_w - center.x) * zoom + vp_center.x,
                -(-half_h - center.y) * zoom + vp_center.y,
            );

            let canvas_screen_rect = egui::Rect::from_min_max(screen_min, screen_max)
                .intersect(rect);

            if !canvas_screen_rect.is_negative() {
                ui.painter().rect_filled(canvas_screen_rect, 0.0, color);
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
        if let Some((w, h)) = cam_state.canvas_size {
            let half = Vec2::new(w * 0.5, h * 0.5);
            res.camera.show_all(-half, half);
        } else if let Some((min, max)) = cam_state.content_bounds {
            res.camera.show_all(min, max);
        } else {
            res.camera.reset();
        }
        cam_state.do_show_all = false;
    }

    if cam_state.pan_delta != Vec2::ZERO {
        res.camera.pan(cam_state.pan_delta);
        cam_state.pan_delta = Vec2::ZERO;
    }

    if let Some(z) = cam_state.set_zoom.take() {
        res.camera.zoom = z.clamp(0.01, 1000.0);
    }

    if cam_state.zoom_delta.abs() > 0.001 {
        let factor = (1.0 + cam_state.zoom_delta).clamp(0.5, 2.0);
        res.camera.zoom_at(cam_state.zoom_pos, factor);
        cam_state.zoom_delta = 0.0;
    }

    // Read back current camera state for save/restore.
    cam_state.current_center = res.camera.center;
    cam_state.current_zoom = res.camera.zoom;
}

/// Restore canvas camera center and zoom from saved state.
pub fn restore_camera(
    render_state: &egui_wgpu::RenderState,
    cam_state: &mut CameraState,
    center: Vec2,
    zoom: f32,
) {
    cam_state.current_center = center;
    cam_state.current_zoom = zoom;

    let mut resources = render_state.renderer.write();
    if let Some(res) = resources.callback_resources.get_mut::<CanvasRenderResources>() {
        res.camera.center = center;
        res.camera.zoom = zoom;
    }
}
