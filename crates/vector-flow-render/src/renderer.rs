use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::batch::PreparedScene;
use crate::camera::{Camera, CameraUniform};
use crate::vertex::CanvasVertex;

// ---------------------------------------------------------------------------
// PrimitiveUniform — per-batch transform + color tint
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PrimitiveUniform {
    pub transform: [[f32; 4]; 4],
    pub color: [f32; 4],
}

impl PrimitiveUniform {
    pub fn new(transform: Mat4, color: [f32; 4]) -> Self {
        Self {
            transform: transform.to_cols_array_2d(),
            color,
        }
    }
}

// ---------------------------------------------------------------------------
// Uploaded scene data
// ---------------------------------------------------------------------------

struct UploadedScene {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    /// Per-batch: (bind_group, index_offset, index_count)
    batches: Vec<(wgpu::BindGroup, u32, u32)>,
}

// ---------------------------------------------------------------------------
// CanvasRenderer
// ---------------------------------------------------------------------------

pub struct CanvasRenderer {
    pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    camera_bind_group_layout: wgpu::BindGroupLayout,
    primitive_bind_group_layout: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    scene: Option<UploadedScene>,
}

impl CanvasRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        // Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vector_canvas_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/vector.wgsl").into(),
            ),
        });

        // Bind group layouts
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let primitive_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("primitive_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vector_canvas_pipeline_layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &primitive_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Camera uniform buffer
        let camera_uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniform_buffer"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vector_canvas_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[CanvasVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            camera_bind_group_layout,
            primitive_bind_group_layout,
            camera_buffer,
            camera_bind_group,
            scene: None,
        }
    }

    /// Write updated camera uniform to the GPU.
    pub fn update_camera(&self, queue: &wgpu::Queue, camera: &Camera) {
        let uniform = camera.uniform();
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Upload tessellated scene to GPU buffers.
    pub fn upload_scene(&mut self, device: &wgpu::Device, scene: &PreparedScene) {
        if scene.vertices.is_empty() || scene.indices.is_empty() || scene.batches.is_empty() {
            self.scene = None;
            return;
        }

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canvas_vertex_buffer"),
            contents: bytemuck::cast_slice(&scene.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canvas_index_buffer"),
            contents: bytemuck::cast_slice(&scene.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let batches: Vec<(wgpu::BindGroup, u32, u32)> = scene
            .batches
            .iter()
            .map(|batch| {
                let uniform = PrimitiveUniform::new(batch.transform, batch.color);
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("primitive_uniform_buffer"),
                    contents: bytemuck::bytes_of(&uniform),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("primitive_bind_group"),
                    layout: &self.primitive_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                (bind_group, batch.index_offset, batch.index_count)
            })
            .collect();

        self.scene = Some(UploadedScene {
            vertex_buffer,
            index_buffer,
            batches,
        });
    }

    /// Record draw commands into a render pass.
    pub fn render(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        let scene = match &self.scene {
            Some(s) => s,
            None => return,
        };

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
        render_pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
        render_pass.set_index_buffer(scene.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        for (bind_group, index_offset, index_count) in &scene.batches {
            render_pass.set_bind_group(1, bind_group, &[]);
            render_pass.draw_indexed(*index_offset..(*index_offset + *index_count), 0, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_uniform_size() {
        assert_eq!(std::mem::size_of::<PrimitiveUniform>(), 80);
    }

    #[test]
    fn primitive_uniform_new() {
        let u = PrimitiveUniform::new(Mat4::IDENTITY, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(u.transform, Mat4::IDENTITY.to_cols_array_2d());
        assert_eq!(u.color, [1.0, 0.0, 0.0, 1.0]);
    }
}
