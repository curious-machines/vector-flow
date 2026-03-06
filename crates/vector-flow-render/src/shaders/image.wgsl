// Camera uniform — group 0, same as vector pipeline
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

// Per-primitive uniform — group 1, same layout as vector pipeline
struct PrimitiveUniform {
    transform: mat4x4<f32>,
    color: vec4<f32>,       // .a used for opacity tint
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> primitive: PrimitiveUniform;

@group(2) @binding(0)
var t_image: texture_2d<f32>;

@group(2) @binding(1)
var s_image: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = primitive.transform * vec4<f32>(in.position, 0.0, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_image, s_image, in.uv);
    return tex_color * primitive.color;
}
