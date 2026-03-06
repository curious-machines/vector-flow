use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CanvasVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl CanvasVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        static ATTRIBUTES: &[wgpu::VertexAttribute] = &[
            // position
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            // color
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x4,
            },
        ];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CanvasVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ImageVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
}

impl ImageVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        static ATTRIBUTES: &[wgpu::VertexAttribute] = &[
            // position
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            // uv
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
        ];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ImageVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: ATTRIBUTES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_size_is_24_bytes() {
        assert_eq!(std::mem::size_of::<CanvasVertex>(), 24);
    }

    #[test]
    fn vertex_attribute_count() {
        let layout = CanvasVertex::desc();
        assert_eq!(layout.attributes.len(), 2);
    }

    #[test]
    fn vertex_pod_roundtrip() {
        let v = CanvasVertex {
            position: [1.0, 2.0],
            color: [0.5, 0.6, 0.7, 1.0],
        };
        let bytes = bytemuck::bytes_of(&v);
        let v2: &CanvasVertex = bytemuck::from_bytes(bytes);
        assert_eq!(v.position, v2.position);
        assert_eq!(v.color, v2.color);
    }
}
