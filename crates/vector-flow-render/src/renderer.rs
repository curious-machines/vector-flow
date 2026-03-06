use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::batch::{ImageDrawBatch, PreparedScene};
use crate::camera::{Camera, CameraUniform};
use crate::vertex::{CanvasVertex, ImageVertex};

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

struct UploadedImage {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    primitive_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,
}

#[allow(dead_code)]
struct CachedTexture {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

// ---------------------------------------------------------------------------
// CanvasRenderer
// ---------------------------------------------------------------------------

pub struct CanvasRenderer {
    // Vector pipeline
    pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    camera_bind_group_layout: wgpu::BindGroupLayout,
    primitive_bind_group_layout: wgpu::BindGroupLayout,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    scene: Option<UploadedScene>,
    // Image pipeline
    image_pipeline: wgpu::RenderPipeline,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uploaded_images: Vec<UploadedImage>,
    texture_cache: HashMap<String, CachedTexture>,
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

        // ── Image pipeline ──────────────────────────────────────
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image_canvas_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/image.wgsl").into(),
            ),
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("texture_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let image_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("image_canvas_pipeline_layout"),
                bind_group_layouts: &[
                    &camera_bind_group_layout,
                    &primitive_bind_group_layout,
                    &texture_bind_group_layout,
                ],
                push_constant_ranges: &[],
            });

        let image_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("image_canvas_pipeline"),
                layout: Some(&image_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &image_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[ImageVertex::desc()],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &image_shader,
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            pipeline,
            camera_bind_group_layout,
            primitive_bind_group_layout,
            camera_buffer,
            camera_bind_group,
            scene: None,
            image_pipeline,
            texture_bind_group_layout,
            sampler,
            uploaded_images: Vec::new(),
            texture_cache: HashMap::new(),
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

    /// Upload image batches: create textures (cached) and per-image quad buffers.
    pub fn upload_images(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image_batches: &[ImageDrawBatch],
    ) {
        self.uploaded_images.clear();

        for batch in image_batches {
            let key = &batch.image.source_path;

            // Get or create cached texture bind group
            if !self.texture_cache.contains_key(key) {
                let size = wgpu::Extent3d {
                    width: batch.image.width,
                    height: batch.image.height,
                    depth_or_array_layers: 1,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("image_texture"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &batch.image.pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * batch.image.width),
                        rows_per_image: Some(batch.image.height),
                    },
                    size,
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("image_texture_bind_group"),
                    layout: &self.texture_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                self.texture_cache.insert(
                    key.clone(),
                    CachedTexture {
                        _texture: texture,
                        bind_group,
                    },
                );
            }

            // Vertex + index buffers for this quad
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image_vertex_buffer"),
                contents: bytemuck::cast_slice(&batch.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image_index_buffer"),
                contents: bytemuck::cast_slice(&batch.indices),
                usage: wgpu::BufferUsages::INDEX,
            });

            // Primitive uniform
            let uniform = PrimitiveUniform::new(batch.transform, batch.color);
            let prim_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image_primitive_uniform"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let primitive_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("image_primitive_bind_group"),
                layout: &self.primitive_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: prim_buffer.as_entire_binding(),
                }],
            });

            // Recreate texture bind group referencing the cached texture
            // (bind groups are cheap; the expensive texture upload is cached)
            let cached = self.texture_cache.get(key).unwrap();
            let view = cached._texture.create_view(&wgpu::TextureViewDescriptor::default());
            let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("image_texture_ref"),
                layout: &self.texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            self.uploaded_images.push(UploadedImage {
                vertex_buffer,
                index_buffer,
                primitive_bind_group,
                texture_bind_group,
            });
        }
    }

    /// Record draw commands into a render pass.
    pub fn render(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        // Draw vector shapes
        if let Some(scene) = &self.scene {
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                scene.index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );

            for (bind_group, index_offset, index_count) in &scene.batches {
                render_pass.set_bind_group(1, bind_group, &[]);
                render_pass.draw_indexed(
                    *index_offset..(*index_offset + *index_count),
                    0,
                    0..1,
                );
            }
        }

        // Draw images
        if !self.uploaded_images.is_empty() {
            render_pass.set_pipeline(&self.image_pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);

            for img in &self.uploaded_images {
                render_pass.set_bind_group(1, &img.primitive_bind_group, &[]);
                render_pass.set_bind_group(2, &img.texture_bind_group, &[]);
                render_pass.set_vertex_buffer(0, img.vertex_buffer.slice(..));
                render_pass.set_index_buffer(
                    img.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                render_pass.draw_indexed(0..6, 0, 0..1);
            }
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
