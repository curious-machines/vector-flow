use egui_wgpu::wgpu;
use glam::Vec2;

use crate::batch::PreparedScene;
use crate::camera::Camera;
use crate::renderer::CanvasRenderer;

/// Row alignment required by wgpu for buffer-to-texture copies.
const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = 256;

/// Texture format used for offscreen rendering.
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// MSAA sample count for anti-aliased offscreen rendering.
const MSAA_SAMPLE_COUNT: u32 = 4;

/// Camera configuration for offscreen export.
pub enum ExportCamera {
    /// Use explicit center and zoom from the current viewport.
    Explicit { center: Vec2, zoom: f32 },
    /// Automatically fit all content in the rendered frame.
    FitToContent,
}

/// Offscreen renderer for exporting canvas content to pixel buffers.
///
/// Creates its own `CanvasRenderer`, texture, and staging buffer — completely
/// independent of the on-screen egui callback renderer.
pub struct OffscreenRenderer {
    renderer: CanvasRenderer,
    /// Resolve target (sample_count=1) — final image is read from here.
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    /// Multisample render target (sample_count=MSAA_SAMPLE_COUNT).
    msaa_texture: wgpu::Texture,
    msaa_texture_view: wgpu::TextureView,
    staging_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row_bytes: u32,
}

impl OffscreenRenderer {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let renderer =
            CanvasRenderer::with_sample_count(device, OFFSCREEN_FORMAT, MSAA_SAMPLE_COUNT);
        let (texture, texture_view, msaa_texture, msaa_texture_view, staging_buffer, padded_row_bytes) =
            Self::create_resources(device, width, height);

        Self {
            renderer,
            texture,
            texture_view,
            msaa_texture,
            msaa_texture_view,
            staging_buffer,
            width,
            height,
            padded_row_bytes,
        }
    }

    /// Recreate texture and staging buffer for new dimensions.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == self.width && height == self.height {
            return;
        }
        let (texture, texture_view, msaa_texture, msaa_texture_view, staging_buffer, padded_row_bytes) =
            Self::create_resources(device, width, height);
        self.texture = texture;
        self.texture_view = texture_view;
        self.msaa_texture = msaa_texture;
        self.msaa_texture_view = msaa_texture_view;
        self.staging_buffer = staging_buffer;
        self.width = width;
        self.height = height;
        self.padded_row_bytes = padded_row_bytes;
    }

    /// Render a scene and return RGBA pixel bytes (row-major, top-to-bottom, no padding).
    ///
    /// This is a blocking operation — it submits GPU work and waits for completion.
    pub fn render_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &PreparedScene,
        camera_mode: &ExportCamera,
    ) -> Vec<u8> {
        // Configure camera.
        let mut camera = Camera::new(Vec2::new(self.width as f32, self.height as f32));
        match camera_mode {
            ExportCamera::Explicit { center, zoom } => {
                camera.center = *center;
                camera.zoom = *zoom;
            }
            ExportCamera::FitToContent => {
                if let Some((min, max)) = scene.bounds() {
                    camera.show_all(min, max);
                }
            }
        }

        // Upload scene data.
        self.renderer.update_camera(queue, &camera);
        self.renderer.upload_scene(device, scene);
        self.renderer
            .upload_images(device, queue, &scene.image_batches);

        // Create command encoder, render pass, and draw.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("offscreen_export_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("offscreen_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_texture_view,
                    resolve_target: Some(&self.texture_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_viewport(
                0.0,
                0.0,
                self.width as f32,
                self.height as f32,
                0.0,
                1.0,
            );
            render_pass.set_scissor_rect(0, 0, self.width, self.height);

            self.renderer.render(&mut render_pass);
        }

        // Copy texture to staging buffer.
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row_bytes),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(std::iter::once(encoder.finish()));

        // Map and read back pixels (blocking).
        let buffer_slice = self.staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::Maintain::Wait);
        receiver
            .recv()
            .expect("GPU channel closed")
            .expect("Failed to map staging buffer");

        let data = buffer_slice.get_mapped_range();
        let unpadded_row_bytes = self.width * 4;
        let mut pixels = Vec::with_capacity((unpadded_row_bytes * self.height) as usize);
        for row in 0..self.height {
            let start = (row * self.padded_row_bytes) as usize;
            let end = start + unpadded_row_bytes as usize;
            pixels.extend_from_slice(&data[start..end]);
        }
        drop(data);
        self.staging_buffer.unmap();

        pixels
    }

    fn create_resources(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::Texture, wgpu::TextureView, wgpu::Buffer, u32) {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        // Resolve target (sample_count=1) — pixels are read from here.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // MSAA render target.
        let msaa_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen_msaa_texture"),
            size,
            mip_level_count: 1,
            sample_count: MSAA_SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let msaa_texture_view = msaa_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let unpadded_row_bytes = width * 4;
        let padded_row_bytes =
            unpadded_row_bytes.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT)
                * COPY_BYTES_PER_ROW_ALIGNMENT;

        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen_staging_buffer"),
            size: (padded_row_bytes * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        (texture, texture_view, msaa_texture, msaa_texture_view, staging_buffer, padded_row_bytes)
    }
}
