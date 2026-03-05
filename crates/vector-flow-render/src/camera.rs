use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
}

pub struct Camera {
    pub center: Vec2,
    pub zoom: f32,
    pub viewport_size: Vec2,
}

impl Camera {
    pub fn new(viewport_size: Vec2) -> Self {
        Self {
            center: Vec2::ZERO,
            zoom: 1.0,
            viewport_size,
        }
    }

    pub fn uniform(&self) -> CameraUniform {
        let half_w = self.viewport_size.x * 0.5 / self.zoom;
        let half_h = self.viewport_size.y * 0.5 / self.zoom;

        let left = self.center.x - half_w;
        let right = self.center.x + half_w;
        // Y-up in world space: bottom < top
        let bottom = self.center.y - half_h;
        let top = self.center.y + half_h;

        let mat = Mat4::orthographic_rh(left, right, bottom, top, -1.0, 1.0);
        CameraUniform {
            view_proj: mat.to_cols_array_2d(),
        }
    }

    /// Convert screen pixel coordinates to world coordinates.
    /// Screen origin is top-left, Y increases downward.
    /// World origin is center of viewport, Y increases upward.
    pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
        let half_vp = self.viewport_size * 0.5;
        // Normalize to [-1, 1] with y-flip
        let ndc_x = (screen_pos.x - half_vp.x) / half_vp.x;
        let ndc_y = -(screen_pos.y - half_vp.y) / half_vp.y;
        // Scale by visible extent
        let half_w = self.viewport_size.x * 0.5 / self.zoom;
        let half_h = self.viewport_size.y * 0.5 / self.zoom;
        Vec2::new(
            self.center.x + ndc_x * half_w,
            self.center.y + ndc_y * half_h,
        )
    }

    /// Pan the camera by a screen-space delta (pixels).
    pub fn pan(&mut self, delta_screen: Vec2) {
        // Convert screen pixels to world units, y-flip
        let dx = -delta_screen.x / self.zoom;
        let dy = delta_screen.y / self.zoom;
        self.center += Vec2::new(dx, dy);
    }

    /// Zoom toward a screen position, preserving the world point under the cursor.
    pub fn zoom_at(&mut self, screen_pos: Vec2, factor: f32) {
        let world_before = self.screen_to_world(screen_pos);
        self.zoom *= factor;
        self.zoom = self.zoom.clamp(0.01, 1000.0);
        let world_after = self.screen_to_world(screen_pos);
        // Adjust center so the world point stays under the cursor
        self.center += world_before - world_after;
    }

    pub fn set_viewport(&mut self, size: Vec2) {
        self.viewport_size = size;
    }

    /// Reset camera to default: zoom 1.0, centered on origin.
    pub fn reset(&mut self) {
        self.center = Vec2::ZERO;
        self.zoom = 1.0;
    }

    /// Fit the camera so that the given content bounds fill the viewport.
    /// Chooses horizontal or vertical fit based on aspect ratio, with a small margin.
    pub fn show_all(&mut self, content_min: Vec2, content_max: Vec2) {
        let content_size = content_max - content_min;
        if content_size.x <= 0.0 && content_size.y <= 0.0 {
            self.reset();
            return;
        }

        self.center = (content_min + content_max) * 0.5;

        // Add 10% margin on each side.
        let margin = 1.2;
        let content_w = content_size.x * margin;
        let content_h = content_size.y * margin;

        // Choose zoom so content fits within viewport.
        let zoom_x = if content_w > 0.0 {
            self.viewport_size.x / content_w
        } else {
            f32::INFINITY
        };
        let zoom_y = if content_h > 0.0 {
            self.viewport_size.y / content_h
        } else {
            f32::INFINITY
        };

        self.zoom = zoom_x.min(zoom_y).clamp(0.01, 1000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_uniform_size() {
        assert_eq!(std::mem::size_of::<CameraUniform>(), 64);
    }

    #[test]
    fn center_maps_to_origin() {
        let cam = Camera::new(Vec2::new(800.0, 600.0));
        // Screen center (400, 300) should map to world (0, 0)
        let world = cam.screen_to_world(Vec2::new(400.0, 300.0));
        assert!((world.x).abs() < 1e-5);
        assert!((world.y).abs() < 1e-5);
    }

    #[test]
    fn top_left_corner_mapping() {
        let cam = Camera::new(Vec2::new(800.0, 600.0));
        // Screen (0, 0) = top-left → world (-400, 300)
        let world = cam.screen_to_world(Vec2::new(0.0, 0.0));
        assert!((world.x - (-400.0)).abs() < 1e-3);
        assert!((world.y - 300.0).abs() < 1e-3);
    }

    #[test]
    fn pan_moves_center() {
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        cam.pan(Vec2::new(100.0, 0.0));
        // Dragging right should move center left in world
        assert!(cam.center.x < 0.0);
    }

    #[test]
    fn zoom_at_preserves_world_point() {
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        let screen_pos = Vec2::new(200.0, 150.0);
        let world_before = cam.screen_to_world(screen_pos);

        cam.zoom_at(screen_pos, 2.0);

        let world_after = cam.screen_to_world(screen_pos);
        assert!((world_before.x - world_after.x).abs() < 1e-3);
        assert!((world_before.y - world_after.y).abs() < 1e-3);
    }

    #[test]
    fn orthographic_matrix_is_valid() {
        let cam = Camera::new(Vec2::new(800.0, 600.0));
        let u = cam.uniform();
        let mat = Mat4::from_cols_array_2d(&u.view_proj);
        // Should not be zero or identity
        assert_ne!(mat, Mat4::ZERO);
        assert_ne!(mat, Mat4::IDENTITY);
        // Determinant should be non-zero (invertible)
        assert!(mat.determinant().abs() > 1e-10);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut cam = Camera::new(Vec2::new(800.0, 600.0));
        cam.zoom_at(Vec2::new(400.0, 300.0), 0.0001);
        assert!(cam.zoom >= 0.01);
        cam.zoom = 1.0;
        cam.zoom_at(Vec2::new(400.0, 300.0), 100000.0);
        assert!(cam.zoom <= 1000.0);
    }
}
