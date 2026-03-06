# Design 11: Text Nodes

## Overview

Two new nodes in a `NodeCategory::Text` category:
- **Text** — defines text content, font, and layout; outputs `TextInstance` rendered on canvas
- **TextToPath** — pure converter: accepts `TextInstance`, outputs `Path` with glyph outlines

## Core Types

### TextStyle
```rust
pub struct TextStyle {
    pub font_family: String,       // system font name ("Noto Sans", "Arial"), empty = default
    pub font_path: String,         // explicit .ttf/.otf path (overrides family lookup)
    pub font_size: f64,            // in canvas units (default 24.0)
    pub font_weight: u16,          // 100-900 (400=regular, 700=bold)
    pub font_style: FontStyle,     // Normal / Italic / Oblique
    pub letter_spacing: f64,       // extra spacing between glyphs (default 0.0)
    pub line_height: f64,          // multiplier (default 1.2)
    pub alignment: TextAlignment,  // Left / Center / Right
    pub wrap: bool,                // word wrapping (default true)
    pub box_width: f64,            // text box width (0 = no constraint)
    pub box_height: f64,           // text box height (0 = no constraint)
}

pub enum FontStyle { Normal, Italic, Oblique }
pub enum TextAlignment { Left, Center, Right }
```

### TextLayout (computed during evaluation)
```rust
pub struct TextLayout {
    pub glyphs: Vec<PositionedGlyph>,
    pub bounds: (f32, f32),         // width, height of laid-out text
    pub font_data: Arc<Vec<u8>>,    // raw font bytes (for re-rasterization + outline extraction)
    pub font_index: u32,            // face index in font collection
}

pub struct PositionedGlyph {
    pub glyph_id: u16,
    pub x: f32,
    pub y: f32,
    pub size: f32,                  // font size in canvas units
}
```

### TextInstance
```rust
pub struct TextInstance {
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
    pub transform: Affine2,
    pub opacity: f32,
    pub layout: Arc<TextLayout>,
}
```

### Type System Additions
- `NodeData::Text(Arc<TextInstance>)` — new variant
- `DataType::Text` — new variant
- Promotion: none (Text is a terminal type, only TextToPath converts it)

## NodeOp Variants

### Text
```rust
NodeOp::Text {
    text: String,          // text content (edited in properties panel)
    font_family: String,   // system font name or empty
    font_path: String,     // explicit font file path or empty
}
```

**Input ports:**
| Port        | Type   | Default | Description |
|-------------|--------|---------|-------------|
| position    | Vec2   | (0,0)  | Text anchor position |
| font_size   | Scalar | 24.0   | Size in canvas units |
| font_weight | Int    | 400    | Weight 100-900 |
| font_style  | Int    | 0      | 0=Normal, 1=Italic, 2=Oblique |
| letter_spacing | Scalar | 0.0 | Extra inter-glyph spacing |
| line_height | Scalar | 1.2    | Line height multiplier |
| alignment   | Int    | 0      | 0=Left, 1=Center, 2=Right |
| box_width   | Scalar | 0.0    | Text box width (0=unconstrained) |
| box_height  | Scalar | 0.0    | Text box height (0=unconstrained) |
| wrap        | Bool   | true   | Word wrapping |
| color       | Color  | white  | Text color |
| opacity     | Scalar | 1.0    | Opacity 0-1 |

**Output ports:**
| Port   | Type | Description |
|--------|------|-------------|
| text   | Text | TextInstance for rendering or TextToPath |
| width  | Scalar | Computed layout width |
| height | Scalar | Computed layout height |

### TextToPath
```rust
NodeOp::TextToPath
```

**Input ports:**
| Port | Type | Description |
|------|------|-------------|
| text | Text | TextInstance from Text node |

**Output ports:**
| Port | Type | Description |
|------|------|-------------|
| path | Path | Glyph outlines as PathData |

All text/font/layout decisions live on the Text node. TextToPath is a pure converter
that reads the font_data and glyph positions from TextLayout to extract outlines.

## Font Loading

### Dependencies
- `ab_glyph = "0.2"` — font loading, metrics, rasterization, outline extraction
- `fontdb = "0.23"` — system font discovery with CSS-like family/weight/style queries

### Resolution Order
1. `font_path` set and non-empty → load from file (resolve relative to project_dir)
2. `font_family` set and non-empty → query fontdb (family + weight + style)
3. Fallback → bundled Noto Sans (`include_bytes!`)

### Bundled Font
Noto Sans Regular and Noto Sans Italic (OFL-licensed) embedded via `include_bytes!`.
Stored in `crates/vector-flow-compute/src/fonts/`.

### System Fonts
`fontdb::Database::load_system_fonts()` scans platform directories:
- Linux: `/usr/share/fonts`, `~/.local/share/fonts`
- macOS: `/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts`
- Windows: `C:\Windows\Fonts`

Query: `db.query(&Query { families: &[Family::Name(&name)], weight, style, .. })`

### Italic Handling
- fontdb queries with `Style::Italic` — finds italic variant if available
- If no italic variant found, apply synthetic shear (skewX ~12 degrees) to glyph positions
- Oblique uses the same fallback

## Compute Crate

### Text Node Execution
1. Load font bytes (cached): try font_path → fontdb query → bundled default
2. Create `ab_glyph::FontRef` from bytes
3. Layout glyphs:
   - Walk characters, get glyph IDs, compute advance widths + kerning
   - Apply letter_spacing
   - Handle word wrapping when box_width > 0 (break on whitespace)
   - Apply alignment per line (left/center/right offset)
   - Stack lines with line_height * font metrics height
   - Clip to box_height if set
4. Build TextLayout with glyph positions
5. Build TextInstance with style, color, transform, opacity
6. Output TextInstance + computed bounds

### Font Cache
`CpuBackend` gets a font cache: `HashMap<String, Arc<Vec<u8>>>` keyed by resolved path.
fontdb Database created once and stored on CpuBackend.

### TextToPath Execution
1. Read TextInstance from input
2. Load font from TextLayout.font_data
3. For each PositionedGlyph:
   - Use ab_glyph outline API to get OutlineCurve variants (Line/Quad/Cubic)
   - Scale and position curves according to glyph position + font size
   - Convert to PathData verbs (MoveTo/LineTo/QuadTo/CubicTo/Close)
   - Font Y-axis is typically inverted (Y-down) — flip during conversion
4. Merge all glyph outlines into one PathData
5. Apply the TextInstance's transform to the path
6. Output as Path

## Render Pipeline

### Approach: Zoom-Aware Rasterization (Phase 1)

Text is rasterized into RGBA textures at a resolution matched to the current view:
`pixel_size = font_size * zoom * device_pixel_ratio`

Rendered as textured quads via the existing image pipeline (shared GPU pipeline).

### Rasterization
- For each TextInstance, rasterize all glyphs into a single RGBA buffer
- Use ab_glyph's `OutlinedGlyph::draw()` for per-pixel coverage
- Apply text color and opacity during rasterization
- Produce an ImageData-like buffer with the text block

### Caching
- Cache keyed by: hash(text + style + color + size_bucket)
- Size bucket: quantize `font_size * zoom` to discrete steps (e.g., round to nearest power of 1.5)
- Invalidate when zoom crosses bucket boundary or text/style changes

### Integration with Image Pipeline
- `CollectedText` in batch.rs (parallel to `CollectedImage`)
- During `prepare_scene_full()`, rasterize text → create texture → produce ImageDrawBatch
- Image pipeline renders the textured quad as usual
- Text draws after vector shapes, interleaved with images by draw order

### Scene Collection
`collect_scene()` extracts `NodeData::Text` from eval results into `CollectedText` structs,
similar to how `NodeData::Image` becomes `CollectedImage`.

## Future: SDF Text Rendering (Phase 2)

The current zoom-aware rasterization approach works well but requires re-rasterization
when zoom changes significantly. A superior approach for the future:

### Signed Distance Field (SDF) Rendering
- Rasterize each glyph as a distance field (not coverage) at a moderate resolution (e.g., 64x64)
- Store in a shared glyph atlas texture
- Fragment shader applies threshold: `step(0.5, distance)` for crisp edges at ANY zoom
- No re-rasterization needed — one SDF works at all scales

### Multi-channel SDF (MSDF)
- Standard SDF loses sharp corners at low resolution
- MSDF uses 3 channels (RGB) to encode corner information
- Library: `msdfgen` (or Rust port) generates MSDF textures
- Requires a dedicated shader but produces near-perfect results

### Benefits
- Single rasterization per glyph (not per zoom level)
- Smooth edges at extreme zoom levels
- Small atlas texture (64x64 per glyph is sufficient)
- GPU-efficient: simple texture lookup + threshold

### Migration Path
1. Replace zoom-aware rasterization with SDF generation
2. Add SDF-specific fragment shader (simple: `smoothstep` on distance)
3. Replace per-text textures with shared glyph atlas
4. Keep the same TextInstance/TextLayout data model — only rendering changes

## App Crate

### Catalog Entry
- Category: `NodeCategory::Text` (new, distinct color)
- Text node and TextToPath node entries

### Properties Panel
- **text**: Multi-line text editor (like DSL source editor)
- **font_family**: Text field (with future autocomplete from fontdb)
- **font_path**: Text field (with future file picker)
- **Font style controls**: shown as standard port editors for weight, style, size
- **Layout controls**: alignment, wrapping, box dimensions as port editors

### Transforms
TextInstance passes through Translate/Rotate/Scale via transform composition
(same pattern as ImageInstance).
