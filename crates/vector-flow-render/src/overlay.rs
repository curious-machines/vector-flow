use egui_wgpu::wgpu;

use crate::batch::PreparedScene;
use crate::camera::Camera;
use crate::renderer::CanvasRenderer;

// ---------------------------------------------------------------------------
// Resources stored in egui's CallbackResources type map
// ---------------------------------------------------------------------------

pub struct CanvasRenderResources {
    pub renderer: CanvasRenderer,
    pub camera: Camera,
}

// ---------------------------------------------------------------------------
// CanvasCallback — bridges egui_wgpu::CallbackTrait
// ---------------------------------------------------------------------------

pub struct CanvasCallback {
    pub scene: Option<PreparedScene>,
}

impl egui_wgpu::CallbackTrait for CanvasCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut CanvasRenderResources = callback_resources.get_mut().unwrap();

        // Update camera uniform
        resources.renderer.update_camera(queue, &resources.camera);

        // Upload scene if we have new geometry
        if let Some(ref scene) = self.scene {
            resources.renderer.upload_scene(device, scene);
            resources.renderer.upload_images(device, queue, &scene.image_batches);
        }

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &CanvasRenderResources = callback_resources.get().unwrap();

        // Set viewport and scissor from the egui paint callback info
        let viewport = info.viewport_in_pixels();
        render_pass.set_viewport(
            viewport.left_px as f32,
            viewport.top_px as f32,
            viewport.width_px as f32,
            viewport.height_px as f32,
            0.0,
            1.0,
        );

        let clip = info.clip_rect_in_pixels();
        render_pass.set_scissor_rect(
            clip.left_px as u32,
            clip.top_px as u32,
            clip.width_px as u32,
            clip.height_px as u32,
        );

        resources.renderer.render(render_pass);
    }
}

/// Create an egui `PaintCallback` that renders the canvas into the given rect.
pub fn canvas_paint_callback(
    rect: egui::Rect,
    scene: Option<PreparedScene>,
) -> egui::epaint::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(rect, CanvasCallback { scene })
}
