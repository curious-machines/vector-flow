# Rendering Internals: From Paths to Pixels

This document describes the complete rendering pipeline in Vector Flow — how vector paths, images, and text are defined, tessellated, collected into scenes, uploaded to the GPU, and drawn to screen via wgpu. It covers every stage in enough detail to reimplement the pipeline from scratch.

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Path and Shape Definitions](#path-and-shape-definitions)
- [Scene Collection](#scene-collection)
- [Tessellation](#tessellation)
- [GPU Data Structures](#gpu-data-structures)
- [Vector Rendering Pipeline](#vector-rendering-pipeline)
- [Image Rendering Pipeline](#image-rendering-pipeline)
- [Text Rendering Pipeline](#text-rendering-pipeline)
- [Camera and Coordinate Systems](#camera-and-coordinate-systems)
- [egui Integration](#egui-integration)
- [Offscreen Rendering](#offscreen-rendering)

## Architecture Overview

The rendering pipeline has five logical stages:

```
Graph Evaluation
    │
    ▼
┌──────────────────┐    CollectedScene (shapes, images, texts, points)
│  Scene Collection │ ──────────────────────────────────────────────►
└──────────────────┘
    │
    ▼
┌──────────────────┐    PreparedScene (vertices, indices, batches)
│  Tessellation     │ ──────────────────────────────────────────────►
└──────────────────┘
    │
    ▼
┌──────────────────┐    GPU buffers + bind groups
│  GPU Upload       │ ──────────────────────────────────────────────►
└──────────────────┘
    │
    ▼
┌──────────────────┐    Render pass draw commands
│  Rendering        │ ──────────────────────────────────────────────►
└──────────────────┘
```

Three separate rendering paths handle different content types, all sharing the same camera:

| Path | Content | Geometry | Shader |
|---|---|---|---|
| Vector | Filled and stroked paths | Tessellated triangles | `vector.wgsl` |
| Image | Loaded images | Textured quads | `image.wgsl` |
| Text | Rasterized text | Textured quads | `image.wgsl` (reused) |

Drawing order: all vector shapes first, then all images and text (painter's algorithm).

**Key crates and files:**

| Crate | File | Purpose |
|---|---|---|
| `vector-flow-core` | `types.rs` | PathVerb, PathData, Shape, StrokeStyle, Color, ImageData, TextInstance |
| `vector-flow-render` | `batch.rs` | Scene collection, tessellation, dash patterns, batching |
| `vector-flow-render` | `vertex.rs` | CanvasVertex, ImageVertex (GPU vertex formats) |
| `vector-flow-render` | `renderer.rs` | CanvasRenderer (pipeline setup, upload, draw commands) |
| `vector-flow-render` | `camera.rs` | Camera (orthographic projection, pan, zoom) |
| `vector-flow-render` | `text_raster.rs` | Text rasterization to pixel buffers |
| `vector-flow-render` | `overlay.rs` | egui_wgpu callback integration |
| `vector-flow-render` | `offscreen.rs` | Export/offscreen rendering with MSAA |
| `vector-flow-render` | `shaders/vector.wgsl` | Vector shape shader |
| `vector-flow-render` | `shaders/image.wgsl` | Image/text shader |

---

## Path and Shape Definitions

**File:** `crates/vector-flow-core/src/types.rs`

### PathVerb and PathData

Paths are defined as a sequence of drawing commands:

```rust
pub enum PathVerb {
    MoveTo(Point),                                  // begin subpath at point
    LineTo(Point),                                  // straight line to point
    QuadTo { ctrl: Point, to: Point },              // quadratic Bézier curve
    CubicTo { ctrl1: Point, ctrl2: Point, to: Point }, // cubic Bézier curve
    Close,                                          // close current subpath
}

pub struct PathData {
    pub verbs: Vec<PathVerb>,
    pub closed: bool,   // metadata flag
}
```

`Point` is a simple `{ x: f32, y: f32 }` struct. A single `PathData` can contain multiple subpaths (sequences of verbs delimited by `MoveTo`/`Close`).

### Color

```rust
pub struct Color {
    pub r: f32, pub g: f32, pub b: f32, pub a: f32,
}
```

Colors are stored in **linear RGB** color space throughout the compute pipeline. Conversion to sRGB happens during tessellation, just before GPU upload. Alpha is straight (not premultiplied).

### StrokeStyle

```rust
pub struct StrokeStyle {
    pub color: Color,           // linear RGB
    pub width: f32,             // world units
    pub line_cap: LineCap,      // Butt, Round, Square
    pub line_join: LineJoin,    // Miter(limit), Round, Bevel
    pub dash_array: Vec<f32>,   // alternating dash/gap lengths
    pub dash_offset: f32,       // phase offset
    pub tolerance: f32,         // per-stroke flattening tolerance (0 = use global)
}
```

### Shape

A `Shape` combines geometry with appearance:

```rust
pub struct Shape {
    pub path: Arc<PathData>,
    pub fill: Option<Color>,
    pub stroke: Option<StrokeStyle>,
    pub transform: Affine2,     // 2D affine (glam::Affine2)
}
```

A shape may have fill, stroke, both, or neither. The transform is applied in the vertex shader via a uniform, not by transforming the path data CPU-side.

### Image Types

```rust
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,       // RGBA8, row-major, top-to-bottom
    pub source_path: String,   // cache key
}

pub struct ImageInstance {
    pub image: Arc<ImageData>,
    pub transform: Affine2,
    pub opacity: f32,
}
```

### Text Types

```rust
pub struct TextInstance {
    pub text: String,
    pub style: TextStyle,          // font family, size, weight, etc.
    pub color: Color,
    pub transform: Affine2,
    pub opacity: f32,
    pub layout: Arc<TextLayout>,   // pre-computed glyph positions
}

pub struct TextLayout {
    pub bounds: (f32, f32),                // (width, height)
    pub glyphs: Vec<PositionedGlyph>,      // per-glyph position + size
    pub font_data: Arc<Vec<u8>>,           // font file bytes
    pub font_index: u32,
}

pub struct PositionedGlyph {
    pub glyph_id: u16,
    pub x: f32,       // horizontal position in layout
    pub y: f32,       // vertical position in layout
    pub size: f32,    // font size at this glyph
}
```

Text layout (glyph positioning) is computed in the compute crate. The render crate receives pre-laid-out text and only handles rasterization.

### NodeData

The graph evaluation produces `NodeData` values, which flow through edges:

```rust
pub enum NodeData {
    // Renderable types
    Shape(Arc<Shape>),
    Shapes(Arc<Vec<Shape>>),
    Path(Arc<PathData>),
    Paths(Arc<Vec<PathData>>),
    Image(Arc<ImageInstance>),
    Text(Arc<TextInstance>),
    Points(Arc<PointBatch>),

    // Scalar/batch types (not directly rendered)
    Scalar(f64), Vec2(...), Color(...), Bool(...), Int(i64),
    Scalars(...), Colors(...), Ints(...),

    // Heterogeneous collection
    Mixed(Arc<Vec<NodeData>>),
}
```

---

## Scene Collection

**File:** `crates/vector-flow-render/src/batch.rs`

Scene collection extracts renderable content from the graph evaluation result and organizes it by type.

### Collected Types

```rust
pub struct CollectedScene {
    pub shapes: Vec<CollectedShape>,
    pub images: Vec<CollectedImage>,
    pub texts: Vec<CollectedText>,
    pub points: Vec<CollectedPoints>,
}

pub struct CollectedShape {
    pub shape: Shape,
    pub dimmed: bool,       // true if preview-dimmed (unselected)
}

pub struct CollectedImage {
    pub image: Arc<ImageData>,
    pub transform: Affine2,
    pub opacity: f32,
    pub dimmed: bool,
}

pub struct CollectedText {
    pub text: Arc<TextInstance>,
    pub dimmed: bool,
}

pub struct CollectedPoints {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    pub dimmed: bool,
}
```

### Collection Process

`collect_scene(eval_result, visible_nodes)` iterates over the evaluation outputs:

1. **Visibility filtering**: If `visible_nodes` is `Some`, only include output from those nodes. If `None`, include everything.

2. **Node ordering**: Optional `node_order` map controls rendering order (nodes with lower order values are drawn first).

3. **Type dispatch** (`collect_node_data`): For each `NodeData` output:
   - `Shape`/`Shapes` → pushed to `shapes` list
   - `Path`/`Paths` → converted to `Shape` with white fill + white stroke (1.5px round cap/join) for preview visibility, then pushed to `shapes`
   - `Image` → pushed to `images` list
   - `Text` → pushed to `texts` list
   - `Points` → pushed to `points` list (raw coordinates, rendered as overlay markers by the app)
   - `Mixed` → recursively unwrapped
   - Other types (Scalar, Int, Color, etc.) → ignored (not renderable)

### Dimmed Tint

Unselected/non-visible shapes can be drawn dimmed using `DIMMED_TINT = [0.3, 0.3, 0.3, 0.5]`. This is applied as a per-batch color tint in the shader.

---

## Tessellation

**File:** `crates/vector-flow-render/src/batch.rs`

Tessellation converts vector paths into triangle meshes suitable for GPU rendering. The application uses **lyon** for all tessellation.

### Path Conversion to Lyon

`build_lyon_path(path: &PathData) -> lyon::path::Path`

Converts our `PathVerb` sequence to a `lyon::path::Path`:

1. Track `in_subpath` state (whether we're inside a begun subpath).
2. For each verb:
   - `MoveTo(p)`: If already in a subpath, call `builder.end(false)` first. Then `builder.begin(point)`. Set `in_subpath = true`.
   - `LineTo(p)`: If not in a subpath, auto-begin at this point. Otherwise `builder.line_to(point)`.
   - `QuadTo { ctrl, to }`: If not in a subpath, auto-begin at control point. Then `builder.quadratic_bezier_to(ctrl, to)`.
   - `CubicTo { ctrl1, ctrl2, to }`: Same auto-begin logic. Then `builder.cubic_bezier_to(ctrl1, ctrl2, to)`.
   - `Close`: If in subpath, `builder.end(true)`. Set `in_subpath = false`.
3. If still in a subpath at the end, `builder.end(false)`.
4. Call `builder.build()`.

### Scene Preparation

`prepare_scene(shapes: &[CollectedShape], tolerance: f32) -> PreparedScene`

For each shape:

1. Convert the shape's `Affine2` transform to a `Mat4` (for the GPU uniform).
2. Compute the tint color: `[1, 1, 1, 1]` for normal shapes, `DIMMED_TINT` for dimmed.
3. **Fill pass**: If the shape has a fill color, tessellate the fill.
4. **Stroke pass**: If the shape has a stroke, tessellate the stroke.

Both passes convert colors from linear to sRGB before writing to vertices.

### Fill Tessellation

`tessellate_fill(path, color, tolerance, transform, tint, buf)`

1. Build the lyon path via `build_lyon_path()`.
2. Create a `FillTessellator` and a `VertexBuffers<CanvasVertex, u32>`.
3. Call `tessellator.tessellate_path()` with `FillOptions::tolerance(tolerance)`.
4. The vertex callback creates a `CanvasVertex` with position from lyon and the fill color converted to sRGB.
5. If successful and non-empty, push the batch.

### Stroke Tessellation

`tessellate_stroke(path, stroke, tolerance, transform, tint, buf)`

1. **Dash pattern check**: If `stroke.dash_array` is non-empty, apply dashing first (see below), then tessellate each dash segment as a simple stroke.
2. **Simple stroke**: Build lyon path, create `StrokeTessellator`, configure `StrokeOptions`:
   - `tolerance(tolerance)`
   - `with_line_width(stroke.width)`
   - Line cap: map `LineCap::Butt/Round/Square` to lyon equivalents
   - Line join: map `LineJoin::Miter(limit)/Round/Bevel` to lyon equivalents
3. Tessellate and push batch.

**Per-shape tolerance**: Each `StrokeStyle` has a `tolerance` field. If > 0, it overrides the global tolerance. This enables zoom-aware rendering (global tolerance = `0.5 / zoom`).

### Dash Pattern Application

`apply_dash_pattern(path, dash_array, dash_offset, tolerance) -> Vec<PathData>`

Converts a continuous path into a series of dashed sub-paths:

1. **Flatten**: Convert the path to line segments using `lyon_path.iter().flattened(tolerance)`. Group segments by subpath (segments between `Begin` and `End` events).

2. **Per-subpath dashing**: Each subpath is dashed independently (dashes don't bridge across disjoint contours). For each subpath:
   - Initialize dash state: `dash_idx = 0`, `dash_remaining = dash_array[0]`, `drawing = true`.
   - Consume the `dash_offset` by advancing through the dash pattern without drawing.
   - Walk each segment: advance along the segment, toggling between dash (drawing) and gap (not drawing) states. When a dash is complete, emit the accumulated path and start a new one.

3. **Output**: A `Vec<PathData>` where each entry is one visible dash segment. Each segment is then tessellated as a normal stroke.

### Batch Merging

`push_batch(geometry, transform, color, buf)`

After tessellation, vertices and indices are appended to shared buffers. The function attempts to merge with the previous batch if the transform and color are identical and the index ranges are contiguous:

```rust
if last.transform == transform && last.color == color
    && last.index_offset + last.index_count == index_offset {
    last.index_count += index_count;  // merge
    return;
}
```

This reduces the number of draw calls for scenes with many shapes sharing the same transform (e.g., shapes at identity transform).

### Color Space Conversion

All colors in the compute pipeline are linear RGB. During tessellation, `linear_to_srgb()` converts to sRGB:

```rust
fn linear_to_srgb_channel(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}
```

This conversion is applied to vertex colors. Alpha is not converted (passed through unchanged).

---

## GPU Data Structures

**File:** `crates/vector-flow-render/src/vertex.rs`, `batch.rs`, `renderer.rs`

### Vertex Formats

**CanvasVertex** (24 bytes) — used for tessellated vector shapes:

```rust
#[repr(C)]
pub struct CanvasVertex {
    pub position: [f32; 2],  // offset 0, location 0, Float32x2
    pub color: [f32; 4],     // offset 8, location 1, Float32x4
}
```

Color is baked into the vertex as sRGB. This means each vertex knows its own color — there is no per-shape color uniform. The per-batch `color` uniform is a tint multiplier (usually `[1,1,1,1]` or `DIMMED_TINT`).

**ImageVertex** (16 bytes) — used for image and text quads:

```rust
#[repr(C)]
pub struct ImageVertex {
    pub position: [f32; 2],  // offset 0, location 0, Float32x2
    pub uv: [f32; 2],        // offset 8, location 1, Float32x2
}
```

### Draw Batches

**DrawBatch** — one draw call for vector shapes:

```rust
pub struct DrawBatch {
    pub vertex_offset: u32,   // (unused — we bind the whole VBO)
    pub index_offset: u32,    // start index in shared index buffer
    pub index_count: u32,     // number of indices to draw
    pub transform: Mat4,      // world transform (from shape.transform)
    pub color: [f32; 4],      // tint multiplier (sRGB)
}
```

**ImageDrawBatch** — one draw call for an image or text:

```rust
pub struct ImageDrawBatch {
    pub image: Arc<ImageData>,           // pixel data + cache key
    pub vertices: [ImageVertex; 4],      // quad corners
    pub indices: [u32; 6],               // two triangles [0,1,2, 0,2,3]
    pub transform: Mat4,                 // world transform
    pub color: [f32; 4],                 // [r, g, b, opacity] tint
}
```

### PreparedScene

The output of tessellation, ready for GPU upload:

```rust
pub struct PreparedScene {
    pub vertices: Vec<CanvasVertex>,       // shared vertex buffer
    pub indices: Vec<u32>,                 // shared index buffer
    pub batches: Vec<DrawBatch>,           // vector draw calls
    pub image_batches: Vec<ImageDrawBatch>, // image/text draw calls
}
```

### Uniform Buffers

**CameraUniform** (64 bytes):

```rust
#[repr(C)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],  // orthographic view-projection matrix
}
```

**PrimitiveUniform** (80 bytes):

```rust
#[repr(C)]
pub struct PrimitiveUniform {
    pub transform: [[f32; 4]; 4],  // world transform
    pub color: [f32; 4],           // tint color
}
```

---

## Vector Rendering Pipeline

**Files:** `renderer.rs`, `shaders/vector.wgsl`

### Pipeline Setup

The vector pipeline is a standard wgpu render pipeline:

**Vertex buffer layout**: `CanvasVertex::desc()` — stride 24, two attributes at locations 0 and 1.

**Bind group layouts**:
- Group 0 (camera): one uniform buffer, vertex-only visibility
- Group 1 (primitive): one uniform buffer, vertex + fragment visibility

**Pipeline layout**: `[camera_layout, primitive_layout]`

**Pipeline state**:
- Topology: `TriangleList`
- Front face: `Ccw`
- Cull mode: `None` (two-sided — necessary for non-convex paths)
- Depth/stencil: `None` (2D rendering, no depth testing)
- Multisample: configurable (1 for on-screen via egui, 4 for offscreen export)

**Blend state** (standard alpha blending):
- Color: `src * SrcAlpha + dst * OneMinusSrcAlpha`
- Alpha: `src * One + dst * OneMinusSrcAlpha`

### Vector Shader (vector.wgsl)

```wgsl
struct CameraUniform { view_proj: mat4x4<f32> };
struct PrimitiveUniform { transform: mat4x4<f32>, color: vec4<f32> };

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> primitive: PrimitiveUniform;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let world_pos = primitive.transform * vec4(in.position, 0.0, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.color = in.color * primitive.color;  // vertex color × tint
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;  // pass through
}
```

The vertex shader applies two transforms: the per-shape world transform and the camera projection. Vertex colors are multiplied by the per-batch tint (identity `[1,1,1,1]` for normal shapes, `[0.3,0.3,0.3,0.5]` for dimmed). The fragment shader just outputs the interpolated color.

### Scene Upload

`upload_scene(device, scene: &PreparedScene)`:

1. Skip if vertices, indices, or batches are empty.
2. Create a single vertex buffer from `scene.vertices` (bytemuck cast to bytes).
3. Create a single index buffer from `scene.indices`.
4. For each batch:
   - Create a `PrimitiveUniform` buffer with the batch's transform and color.
   - Create a bind group for group 1 referencing this buffer.
   - Store `(bind_group, index_offset, index_count)`.

### Draw Commands

`render(render_pass)`:

```rust
// Vector shapes
render_pass.set_pipeline(&self.pipeline);
render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
render_pass.set_vertex_buffer(0, scene.vertex_buffer.slice(..));
render_pass.set_index_buffer(scene.index_buffer.slice(..), IndexFormat::Uint32);

for (bind_group, index_offset, index_count) in &scene.batches {
    render_pass.set_bind_group(1, bind_group, &[]);
    render_pass.draw_indexed(offset..offset+count, 0, 0..1);
}
```

All vector shapes share a single vertex buffer and index buffer. Only the per-batch uniform (transform + tint) changes between draw calls. This is efficient because most shapes share the identity transform, and batch merging combines consecutive same-transform shapes into a single draw call.

---

## Image Rendering Pipeline

**Files:** `renderer.rs`, `shaders/image.wgsl`

### Pipeline Setup

The image pipeline shares the same camera bind group layout (group 0) and primitive bind group layout (group 1) as the vector pipeline, plus an additional texture bind group (group 2).

**Bind group layouts**:
- Group 0 (camera): same as vector pipeline
- Group 1 (primitive): same as vector pipeline
- Group 2 (texture): one `texture_2d<f32>` binding + one `sampler` binding, fragment-only visibility

**Pipeline state**: Same as vector pipeline except for the vertex buffer layout (`ImageVertex::desc()`) and the 3-group pipeline layout.

**Sampler**: Linear filtering for both magnification and minification, clamp-to-edge addressing.

### Image Shader (image.wgsl)

```wgsl
@group(2) @binding(0) var t_image: texture_2d<f32>;
@group(2) @binding(1) var s_image: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let world_pos = primitive.transform * vec4(in.position, 0.0, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_image, s_image, in.uv);
    return tex_color * primitive.color;  // sample × tint (tint.a = opacity)
}
```

The fragment shader samples the texture and multiplies by the tint color. For images, `primitive.color` is `[1, 1, 1, opacity]`, so the RGB channels pass through and only opacity is modulated.

### Image Quad Construction

Each image is rendered as a textured quad centered at the origin:

```rust
let hw = image.width as f32 / 2.0;
let hh = image.height as f32 / 2.0;

let vertices = [
    ImageVertex { position: [-hw, -hh], uv: [0.0, 1.0] },  // bottom-left
    ImageVertex { position: [ hw, -hh], uv: [1.0, 1.0] },  // bottom-right
    ImageVertex { position: [ hw,  hh], uv: [1.0, 0.0] },  // top-right
    ImageVertex { position: [-hw,  hh], uv: [0.0, 0.0] },  // top-left
];
let indices = [0, 1, 2, 0, 2, 3];  // two triangles
```

The UV Y-axis is flipped (`uv.y = 1.0` at bottom, `0.0` at top) because image pixel data is top-to-bottom but the world coordinate system has Y-up.

### Texture Upload and Caching

`upload_images(device, queue, image_batches)`:

For each image batch:

1. **Texture cache check**: Use `image.source_path` as the cache key. If the texture already exists in `texture_cache`, skip creation.

2. **Texture creation** (on cache miss):
   - Create a 2D texture with format `Rgba8UnormSrgb`, usage `TEXTURE_BINDING | COPY_DST`.
   - Upload pixel data via `queue.write_texture()` with `bytes_per_row = width * 4`.
   - Create a texture view and bind group (texture + sampler).
   - Store in `texture_cache`.

3. **Per-image resources** (always created fresh per frame):
   - Vertex buffer (4 vertices) and index buffer (6 indices) for the quad.
   - Primitive uniform buffer with the image's transform and tint color.
   - Texture bind group referencing the cached texture.

### Draw Commands

```rust
render_pass.set_pipeline(&self.image_pipeline);
render_pass.set_bind_group(0, &self.camera_bind_group, &[]);

for img in &self.uploaded_images {
    render_pass.set_bind_group(1, &img.primitive_bind_group, &[]);
    render_pass.set_bind_group(2, &img.texture_bind_group, &[]);
    render_pass.set_vertex_buffer(0, img.vertex_buffer.slice(..));
    render_pass.set_index_buffer(img.index_buffer.slice(..), IndexFormat::Uint32);
    render_pass.draw_indexed(0..6, 0, 0..1);
}
```

Each image is a separate draw call with its own vertex buffer, index buffer, and bind groups. This is acceptable because image counts are typically low (tens, not thousands).

---

## Text Rendering Pipeline

**File:** `crates/vector-flow-render/src/text_raster.rs`

Text is rendered by rasterizing glyphs to a pixel buffer on the CPU, then drawing the result as a textured quad via the image pipeline. This is a pragmatic approach that reuses the existing image rendering infrastructure.

### Rasterization

`rasterize_text(text: &TextInstance, scale_factor: f32) -> Option<(u32, u32, Vec<u8>)>`

1. Load the font from `TextLayout.font_data` using `ab_glyph::FontArc`.
2. Compute raster dimensions: `layout_bounds * scale_factor`, clamped to 4096 pixels max.
3. Allocate an RGBA8 pixel buffer (initialized to transparent black).
4. For each `PositionedGlyph` in the layout:
   - Create an `ab_glyph::Glyph` with the scaled position and size.
   - Call `Font::outline_glyph()` to get the glyph's curve outline.
   - Rasterize with a coverage callback: for each pixel, compute alpha from coverage and the text color, then alpha-blend into the buffer using source-over compositing.
5. Return `(width, height, pixels)`.

### Scale Factor and Size Bucketing

The scale factor is `zoom * pixels_per_point`, where `zoom` is the camera zoom level and `pixels_per_point` is the display DPI scale. Higher zoom = higher rasterization quality = crisp text at all zoom levels.

To avoid re-rasterizing on every tiny zoom change, the font size is quantized into buckets:

```rust
fn size_bucket(pixel_size: f32) -> u32 {
    if pixel_size <= 1.0 { return 0; }
    (pixel_size.ln() / 1.25_f32.ln()).round() as u32
}
```

Each bucket is approximately 25% larger than the previous. Rasterization only occurs when the zoom changes enough to cross a bucket boundary.

### Text to Image Batch Conversion

`prepare_text_batches(texts, zoom, pixels_per_point) -> Vec<ImageDrawBatch>`

For each text instance:

1. Rasterize at the current scale factor.
2. Build a quad in world space, sized to the text's layout bounds (same UV convention as images).
3. Compute the transform: the text's `Affine2` transform composed with a center offset (since the quad is centered at origin but text layout starts at (0, 0)).
4. Set the tint: `[1, 1, 1, opacity]` for normal text, dimmed tint otherwise.
5. Create a unique `source_path` for texture caching: `"__text_{hash}_{width}_{height}"`. The hash incorporates the text content, font properties, color, and size bucket.
6. Return as an `ImageDrawBatch`.

### Integration with Scene Preparation

`prepare_scene_full_with_text(scene, tolerance, zoom, pixels_per_point) -> PreparedScene`

1. Tessellate vector shapes via `prepare_scene()`.
2. Build image batches via `prepare_image_batches()`.
3. Append text batches via `prepare_text_batches()`.
4. The combined image+text batches are stored in `prepared.image_batches`.

Text batches and image batches are drawn in the same pass using the same image pipeline and shader.

---

## Camera and Coordinate Systems

**File:** `crates/vector-flow-render/src/camera.rs`

### Coordinate Systems

The application uses three coordinate systems:

| System | Origin | Y Direction | Units |
|---|---|---|---|
| Screen | Top-left | Down | Pixels |
| World | Viewport center | Up | World units |
| Clip (NDC) | Center | Up | [-1, 1] |

### Camera State

```rust
pub struct Camera {
    pub center: Vec2,         // world-space point at screen center
    pub zoom: f32,            // pixels per world unit (1.0 = no zoom)
    pub viewport_size: Vec2,  // screen dimensions in pixels
}
```

### Orthographic Projection

The camera produces an orthographic view-projection matrix:

```rust
pub fn uniform(&self) -> CameraUniform {
    let half_w = self.viewport_size.x * 0.5 / self.zoom;
    let half_h = self.viewport_size.y * 0.5 / self.zoom;

    let left   = self.center.x - half_w;
    let right  = self.center.x + half_w;
    let bottom = self.center.y - half_h;
    let top    = self.center.y + half_h;

    Mat4::orthographic_rh(left, right, bottom, top, -1.0, 1.0)
}
```

This maps the visible world-space rectangle to clip space [-1, 1]. The visible extent in world units is `viewport_size / zoom`.

### Screen-to-World Conversion

```rust
pub fn screen_to_world(&self, screen_pos: Vec2) -> Vec2 {
    let half_vp = self.viewport_size * 0.5;
    let ndc_x =  (screen_pos.x - half_vp.x) / half_vp.x;
    let ndc_y = -(screen_pos.y - half_vp.y) / half_vp.y;  // Y-flip

    let half_w = self.viewport_size.x * 0.5 / self.zoom;
    let half_h = self.viewport_size.y * 0.5 / self.zoom;

    Vec2::new(
        self.center.x + ndc_x * half_w,
        self.center.y + ndc_y * half_h,
    )
}
```

### Pan

```rust
pub fn pan(&mut self, delta_screen: Vec2) {
    let dx = -delta_screen.x / self.zoom;  // screen right → world left
    let dy =  delta_screen.y / self.zoom;  // screen down → world up
    self.center += Vec2::new(dx, dy);
}
```

### Zoom at Point

```rust
pub fn zoom_at(&mut self, screen_pos: Vec2, factor: f32) {
    let world_before = self.screen_to_world(screen_pos);
    self.zoom *= factor;
    self.zoom = self.zoom.clamp(0.01, 1000.0);
    let world_after = self.screen_to_world(screen_pos);
    self.center += world_before - world_after;
}
```

This preserves the world point under the cursor: compute the world point before zoom, apply the zoom factor, then adjust the camera center so the same world point maps back to the same screen position.

### Show All (Fit to Content)

```rust
pub fn show_all(&mut self, content_min: Vec2, content_max: Vec2) {
    self.center = (content_min + content_max) * 0.5;
    let margin = 1.2;  // 20% padding
    let zoom_x = self.viewport_size.x / (content_size.x * margin);
    let zoom_y = self.viewport_size.y / (content_size.y * margin);
    self.zoom = zoom_x.min(zoom_y).clamp(0.01, 1000.0);
}
```

Centers on the content midpoint and chooses the tighter of horizontal/vertical fit with 20% margin.

### Affine2 to Mat4 Conversion

Shapes use `glam::Affine2` transforms (2D affine: 2×2 matrix + translation). For the GPU, these are converted to `Mat4`:

```rust
pub fn affine2_to_mat4(affine: &Affine2) -> Mat4 {
    let cols = affine.to_cols_array();  // [a, b, c, d, tx, ty]
    Mat4::from_cols_array_2d(&[
        [cols[0], cols[1], 0.0, 0.0],  // column 0
        [cols[2], cols[3], 0.0, 0.0],  // column 1
        [0.0,     0.0,     1.0, 0.0],  // column 2 (z identity)
        [cols[4], cols[5], 0.0, 1.0],  // column 3 (translation)
    ])
}
```

---

## egui Integration

**File:** `crates/vector-flow-render/src/overlay.rs`

The canvas is rendered inside an egui panel using the `egui_wgpu::CallbackTrait` system.

### CanvasRenderResources

Persistent resources stored in egui's `CallbackResources` type map (a `TypeMap`):

```rust
pub struct CanvasRenderResources {
    pub renderer: CanvasRenderer,
    pub camera: Camera,
}
```

These are inserted once during `VectorFlowApp::new()` and persist for the application lifetime.

### CanvasCallback

```rust
pub struct CanvasCallback {
    pub scene: Option<Arc<PreparedScene>>,
}
```

Implements `egui_wgpu::CallbackTrait` with two methods:

**`prepare(device, queue, ...)`** — called before the render pass:
1. Extract `CanvasRenderResources` from `callback_resources` (mutable access).
2. Update camera uniform on GPU via `renderer.update_camera(queue, &camera)`.
3. If a new scene is present, upload it: `renderer.upload_scene()` and `renderer.upload_images()`.

**`paint(info, render_pass, ...)`** — called during the render pass:
1. Extract `CanvasRenderResources` (immutable access).
2. Set viewport from `info.viewport_in_pixels()`.
3. Set scissor rect from `info.clip_rect_in_pixels()`.
4. Call `renderer.render(render_pass)`.

### Paint Callback Creation

```rust
pub fn canvas_paint_callback(
    rect: egui::Rect,
    scene: Option<Arc<PreparedScene>>,
) -> egui::epaint::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(rect, CanvasCallback { scene })
}
```

The app creates this callback each frame in the canvas panel's `ui()` method and adds it to the egui painter. egui schedules it to be drawn as part of its rendering pass — the canvas draws into the same render pass as egui UI elements, at the correct Z-order.

---

## Offscreen Rendering

**File:** `crates/vector-flow-render/src/offscreen.rs`

The offscreen renderer is used for image and video export. It is completely independent of the on-screen renderer — it has its own `CanvasRenderer`, textures, and staging buffer.

### Architecture

```rust
pub struct OffscreenRenderer {
    renderer: CanvasRenderer,          // independent renderer (MSAA-capable)
    texture: wgpu::Texture,            // resolve target (sample_count=1)
    texture_view: wgpu::TextureView,
    msaa_texture: wgpu::Texture,       // MSAA render target (sample_count=4)
    msaa_texture_view: wgpu::TextureView,
    staging_buffer: wgpu::Buffer,      // for GPU→CPU readback
    width: u32,
    height: u32,
    padded_row_bytes: u32,
}
```

**MSAA**: The offscreen renderer uses 4x MSAA for anti-aliased output. The MSAA texture is the render attachment; it resolves to the 1x texture, which is then copied to the staging buffer for readback.

**Texture format**: `Rgba8Unorm` (not `Rgba8UnormSrgb` — export pixels are in linear space).

### Render Workflow

`render_scene_with_bg(device, queue, scene, camera_mode, clear_color) -> (Vec<u8>, Vec2, f32)`

1. **Configure camera**: Create a `Camera` with the export dimensions as viewport. Apply the camera mode:
   - `ExportCamera::Explicit { center, zoom }`: use the provided values directly.
   - `ExportCamera::FitToContent`: call `camera.show_all()` on the scene bounds.

2. **Upload**: Update camera uniform, upload scene data and images (same as on-screen path).

3. **Render pass**:
   - Create a command encoder.
   - Begin a render pass with the MSAA texture as the color attachment and the 1x texture as the resolve target.
   - Clear with the background color (or transparent black if `None`).
   - Set viewport and scissor to full texture dimensions.
   - Call `renderer.render(&mut render_pass)`.

4. **Copy to staging buffer**: `encoder.copy_texture_to_buffer()` copies the resolved 1x texture to the staging buffer, with row padding for wgpu alignment.

5. **Submit and wait**: `queue.submit()` then `device.poll(Maintain::Wait)` — blocking wait for GPU completion.

6. **Read back pixels**:
   - Map the staging buffer for reading.
   - Strip row padding: wgpu requires `bytes_per_row` to be a multiple of 256, but the actual image row is `width * 4` bytes. Copy each unpadded row to the output.
   - Unmap the buffer.

7. **Return**: `(pixels, used_center, used_zoom)` — the camera settings are returned so that video export can lock the camera across all frames (preventing jitter from `FitToContent` recomputation).

### Row Alignment

```rust
const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = 256;

let unpadded_row_bytes = width * 4;
let padded_row_bytes = unpadded_row_bytes.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT)
    * COPY_BYTES_PER_ROW_ALIGNMENT;
```

For example, a 640px-wide image has `640 * 4 = 2560` bytes per row, which is already aligned to 256. A 100px-wide image has `400` bytes, padded to `512`. The staging buffer is sized for padded rows; the readback loop strips the padding.

### Resize

`resize(device, width, height)` recreates all GPU resources (textures + staging buffer) for new dimensions. This is called when the export dialog settings change.
