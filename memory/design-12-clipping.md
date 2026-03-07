# Design 12: Clipping and Masking

## Overview

Three related content visibility and processing features:
- **Clipping** — binary in/out visibility defined by a path shape boundary
- **Masking** — graduated visibility (0-100% opacity) defined by a grayscale image or rendered node output
- **Raster effects** — pixel-space operations (blur, sharpen, edge detect) applied via GPU kernel convolution

## Two Implementation Approaches

### Approach A: Geometric Clipping (path-only)

Boolean intersection of content paths against the clip path, producing new clipped paths at compute time.

- Reuses the existing `BooleanOp::Intersect` stub (currently pass-through)
- Integrate `i_overlay` crate (or similar) to implement real boolean path operations
- **Scope**: Path data only — cannot clip images or text
- **Effort**: Small-medium — mostly wiring up a boolean ops library

This approach also unblocks the existing stubbed boolean ops (Union, Intersect, Difference).

### Approach B: Stencil-Based Clipping (universal)

GPU stencil buffer masks rendering of any content type (shapes, images, text).

- Draw clip shape to stencil buffer, then draw content with stencil test
- Works for all renderable types
- Supports nested clips via stencil increment/decrement
- **Effort**: Medium-large — significant render pipeline changes

#### Render Pipeline Changes Required

1. Add `DepthStencilState` to render pass (currently no depth/stencil attachment)
2. Create stencil-write pipeline variant that writes clip shape to stencil buffer
3. Modify vector, image, and text pipelines to support stencil-test mode
4. Multi-pass rendering per clip group: write stencil -> draw clipped content -> restore stencil
5. For nested clips: increment stencil on enter, decrement on exit, test against current depth

#### Scene Data Changes

`CollectedScene` needs a grouping concept:

```rust
pub struct ClipGroup {
    pub clip_path: PathData,
    pub clip_transform: Affine2,
    pub children: Vec<SceneItem>,
}

pub enum SceneItem {
    Shape(CollectedShape),
    Image(CollectedImage),
    Text(CollectedText),
    Clip(ClipGroup),  // recursive nesting
}
```

## Node Design

### Option 1: Single-Input Clip

```
NodeOp::Clip
  Input 0: clip_path (Path) — defines the clipping region
  Input 1: content (Any) — single item to clip
  Output 0: clipped content (same type as input)
```

Simple, one Clip node per item. Multiple items need multiple Clip nodes (or a Merge first).

### Option 2: Group-Based Clip

```
NodeOp::Clip
  Input 0: clip_path (Path)
  Input 1..N: content (Any) — multiple items
  Output 0: Group (new composite type)
```

Requires a new `NodeData::Group(Vec<NodeData>)` type — more powerful but adds complexity to the type system and rendering pipeline.

### Option 3: Subnet/Nesting Clip

The clip node acts as a container (subnet) — children inside it are clipped. Aligns with the nesting design (design-08) but depends on that being implemented first.

## Masking

Unlike clipping (binary), masking provides graduated opacity control. A mask source defines per-pixel visibility: white = fully visible, black = fully hidden, gray = partially transparent.

### Clip vs Mask Comparison

| | Clip | Mask |
|---|---|---|
| **Source** | Path shape | Raster image or rendered node output |
| **GPU technique** | Stencil buffer (integer test) | Render-to-texture + alpha multiply |
| **Precision** | Binary in/out | 256 levels (grayscale) |
| **Pipeline** | Stencil write + stencil test | Offscreen pass + compositing shader |
| **Geometric shortcut** | Boolean intersection (paths only) | None — always requires render pipeline |

### Mask Node Design

```
NodeOp::Mask
  Input 0: mask (Image or Any) — grayscale mask source
  Input 1: content (Any) — item to mask
  Output 0: masked content (same type as input)
```

The mask input could accept:
- **Image**: a loaded grayscale/alpha image used directly as mask texture
- **Any rendered output**: another node's visual output rendered to texture first (e.g., a blurred circle for a soft vignette, a gradient for a fade)

### Mask Render Pipeline

1. **Render content** to an offscreen texture (temporary render target)
2. **Obtain mask texture** — either a loaded image or render another subgraph to a second offscreen texture
3. **Compositing pass** — a fullscreen quad shader that samples both textures and outputs `content.rgba * mask_luminance` (or `content.rgba * mask.a` for alpha-channel masks)

The `OffscreenRenderer` already exists for export — the same concept applies here, but per-frame and potentially multiple times (one per mask node).

### Mask Channel Modes

How to interpret the mask image:
- **Luminance** (default): `0.2126*R + 0.7152*G + 0.0722*B` — standard BT.709, matches `color_math.rs`
- **Alpha**: use the mask's alpha channel directly
- **Red/Green/Blue**: use a single channel

Could be exposed as an Int parameter (0-4) on the Mask node.

## Raster Effects

Pixel-space operations applied via GPU shaders. Content is rendered to a texture, processed by a kernel convolution (or other pixel shader), and composited back.

### Architecture: Where Effects Live

**Option A: Render-time effects (recommended default)**

The compute graph tags content with effects; the render pipeline applies them. Content stays vector until the last moment.

```
NodeOp::GaussianBlur
  Input 0: content (Any) — item to blur
  Input 1: radius (Scalar) — blur radius in canvas units
  Output 0: affected content (same type, tagged with effect)
```

The collected scene carries effect metadata. The renderer sees "this item has a blur" and inserts render-to-texture + post-process passes.

- Pro: resolution-independent until final render
- Con: effects can't feed back into the compute graph as data

**Option B: Explicit Rasterize + process**

A Rasterize node converts visual content to an Image at a fixed resolution. Effect nodes then operate on Images.

```
NodeOp::Rasterize { width, height }
  Input 0: content (Any)
  Output 0: Image

NodeOp::GaussianBlur
  Input 0: image (Image)
  Output 0: Image
```

- Pro: composable — blurred output is a real Image usable as mask input, etc.
- Con: locks in a resolution, requires CPU rasterization or GPU readback

**Option C: Hybrid (recommended)**

Render-time by default (Option A). Add an explicit Rasterize node for when the result is needed as data (e.g., to feed into a Mask node). Similar to After Effects' pre-compose workflow.

### GPU Implementation

#### Simple kernels (single-pass)

Fragment shader samples input texture at neighboring texel offsets, multiplies by kernel weights, sums:

```wgsl
fn apply_kernel(uv: vec2<f32>, tex: texture_2d<f32>, samp: sampler) -> vec4<f32> {
    var result = vec4<f32>(0.0);
    for (var y: i32 = -1; y <= 1; y++) {
        for (var x: i32 = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * uniforms.texel_size;
            result += textureSample(tex, samp, uv + offset) * kernel[y + 1][x + 1];
        }
    }
    return result;
}
```

Works for 3x3 and 5x5 kernels (sharpen, edge detect, emboss).

#### Gaussian blur (separable, two-pass)

Large kernels use separable decomposition — horizontal pass then vertical pass — reducing O(n^2) to O(2n):

1. Source texture -> horizontal blur -> temp texture
2. Temp texture -> vertical blur -> output texture

Two render-to-texture passes per blur, but large radii stay performant.

#### Custom kernels

A general Convolve node uploads an NxN kernel as a uniform buffer:

```
NodeOp::Convolve
  Input 0: content (Any)
  Input 1: kernel_size (Int) — 3, 5, or 7
  Input 2: weights (String) — flattened NxN values, space/comma separated
  Output 0: affected content
```

Built-in presets (blur, sharpen, etc.) are catalog entries with pre-filled kernel values.

### Suggested Node Catalog

| Node | Type | Notes |
|---|---|---|
| GaussianBlur | Separable, 2-pass | `radius` parameter, most common effect |
| BoxBlur | Separable, 2-pass | uniform weights, cheaper than gaussian |
| Sharpen | 3x3 fixed | `[0,-1,0 / -1,5,-1 / 0,-1,0]` |
| EdgeDetect | 3x3 fixed | Sobel or Laplacian operator |
| Emboss | 3x3 fixed | directional highlight effect |
| Convolve | NxN custom | user-defined kernel matrix |
| Rasterize | N/A | converts any content to Image at fixed resolution |

### Feature Comparison: What Needs Render-to-Texture

| Feature | Render-to-texture | Compositing | Post-process shader |
|---|---|---|---|
| Stencil clip | No (stencil buffer) | No | No |
| Mask | Yes (content + mask) | Yes (alpha multiply) | No |
| Raster effects | Yes (content) | Yes (replace or blend) | Yes (kernel shader) |

All three features build incrementally on the same render-to-texture infrastructure.

### Shared Prerequisite: Render-to-Texture for Subgraphs

Both stencil clipping (Phase 2) and masking require the ability to render arbitrary node subgraph output to an offscreen texture. This is the key shared infrastructure:

- Allocate temporary render targets (texture pool with reuse)
- Route a subgraph's collected scene to an offscreen renderer instead of the main canvas
- Composite the result back into the main render pass

Once render-to-texture exists, clipping and masking become sibling features with different compositing modes.

## Recommended Phasing

### Phase 1: Geometric Clipping
- Integrate `i_overlay` for boolean path operations
- Implement `NodeOp::Clip` as path intersection (single Path input + clip Path)
- Also fixes the stubbed `BooleanOp` variants (Union, Intersect, Difference)
- No render changes needed

### Phase 2: Render-to-Texture Infrastructure
- Temporary render target allocation (texture pool)
- Ability to render a subgraph's collected scene to an offscreen texture
- Compositing shader to blend offscreen results back into the main pass
- This is the shared foundation for both stencil clipping and masking

### Phase 3: Stencil Clipping
- Add stencil buffer to render pipeline
- Implement `ClipGroup` in scene collection
- Single-input clip for images and text
- Nested clip support via stencil stack

### Phase 4: Masking
- `NodeOp::Mask` with image or rendered-node-output as mask source
- Luminance-based compositing shader (with channel mode parameter)
- Image masks (loaded texture) first, then rendered subgraph masks

### Phase 5: Raster Effects
- Kernel convolution shader (generic NxN with uniform buffer)
- GaussianBlur node (separable two-pass)
- Built-in presets: BoxBlur, Sharpen, EdgeDetect, Emboss
- Custom Convolve node with user-defined kernel
- Rasterize node for explicit content-to-Image conversion

### Phase 6: Group Clipping/Masking (optional)
- Introduce `NodeData::Group` composite type
- Multi-input clip/mask nodes
- Or implement via nesting/subnets (design-08)

## Dependencies

- Phase 1: `i_overlay` crate (or equivalent boolean polygon ops library)
- Phase 2: wgpu offscreen render targets, compositing shader
- Phase 3: Phase 2 + stencil buffer support (well-supported), scene graph refactor
- Phase 4: Phase 2 (render-to-texture)
- Phase 5: Phase 2 (render-to-texture) + kernel convolution shaders
- Phase 6: nesting/subnet implementation (design-08)

## Open Questions

- Should geometric clip (Phase 1) be a separate `Clip` node or just the existing `BooleanOp::Intersect`? A dedicated Clip node is more discoverable, but functionally identical for paths.
- For stencil clipping, should the clip path support fill rules (even-odd vs non-zero winding)? Useful for clip paths with holes.
- Should clips be invertible (show everything *outside* the clip path)?
- Should masks be invertible (invert the mask luminance)?
- Mask channel mode: expose as Int enum or separate node variants?
- Performance: how many offscreen render targets can we allocate per frame before it becomes a bottleneck? May need a texture pool with LRU reuse.
- Raster effects: should render-time effects carry resolution metadata, or always render at canvas resolution * zoom?
- Should the Rasterize node respect the current camera/zoom, or take an explicit resolution?
- Chaining multiple effects (blur then sharpen): ping-pong between two temp textures, or allocate per-effect?
