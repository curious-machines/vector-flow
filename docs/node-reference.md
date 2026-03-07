# Vector Flow Node Reference

This document describes every node type available in Vector Flow, organized by category. Each entry covers the node's purpose, inputs, outputs, properties, and any special behavior.

## Table of Contents

- [Data Types](#data-types)
- [Generators](#generators)
  - [Circle](#circle)
  - [Rectangle](#rectangle)
  - [Regular Polygon](#regular-polygon)
  - [Line](#line)
  - [Point Grid](#point-grid)
  - [Scatter Points](#scatter-points)
  - [Load Image](#load-image)
  - [SVG Path](#svg-path)
- [Transforms](#transforms)
  - [Translate](#translate)
  - [Rotate](#rotate)
  - [Scale](#scale)
  - [Apply Transform](#apply-transform)
- [Path Operations](#path-operations)
  - [Path Union](#path-union)
  - [Path Intersect](#path-intersect)
  - [Path Difference](#path-difference)
  - [Path Offset](#path-offset)
  - [Path Subdivide](#path-subdivide)
  - [Path Reverse](#path-reverse)
  - [Resample Path](#resample-path)
- [Styling](#styling)
  - [Set Fill](#set-fill)
  - [Set Stroke](#set-stroke)
- [Color](#color)
  - [Adjust Hue](#adjust-hue)
  - [Adjust Saturation](#adjust-saturation)
  - [Adjust Lightness](#adjust-lightness)
  - [Adjust Luminance](#adjust-luminance)
  - [Invert Color](#invert-color)
  - [Grayscale](#grayscale)
  - [Mix Colors](#mix-colors)
  - [Adjust Alpha](#adjust-alpha)
  - [Color Parse](#color-parse)
- [Constants](#constants)
  - [Constant Scalar](#constant-scalar)
  - [Constant Int](#constant-int)
  - [Constant Vec2](#constant-vec2)
  - [Constant Color](#constant-color)
- [Utility](#utility)
  - [Merge](#merge)
  - [Duplicate](#duplicate)
  - [Portal Send](#portal-send)
  - [Portal Receive](#portal-receive)
  - [VFS Code](#vfs-code)
- [Graph I/O](#graph-io)
  - [Graph Output](#graph-output)
  - [Graph Input](#graph-input)

---

## Data Types

Before diving into nodes, here is a summary of the data types that flow between them:

| Type      | Description                                    |
|-----------|------------------------------------------------|
| Scalar    | 64-bit floating-point number                   |
| Int       | 64-bit signed integer                          |
| Bool      | Boolean (true/false)                           |
| Vec2      | 2D point or vector (x, y)                      |
| Points    | Batch of 2D points                             |
| Path      | Geometric path (open or closed series of vertices) |
| Paths     | Multiple paths                                 |
| Shape     | Path with fill color, stroke color, and stroke width |
| Shapes    | Multiple shapes                                |
| Transform | 2D affine transformation matrix                |
| Color     | RGBA color (each channel 0.0 to 1.0)          |
| Image     | Loaded image with position, size, and opacity  |
| Any       | Accepts any of the above types                 |

**Automatic type promotion:** Some nodes accept broader types than their inputs strictly require. For example, a node expecting a Shape will accept a Path (promoted to a Shape with default styling). A node expecting Scalar will accept Int (promoted to float).

---

## Generators

Generators create geometry from parameters. They are the starting points of most graphs.

### Circle

Creates a circular path approximated by a regular polygon.

**Inputs:**

| Name     | Type   | Default    | Description                            |
|----------|--------|------------|----------------------------------------|
| radius   | Scalar | 100.0      | Radius of the circle                   |
| center   | Vec2   | (0.0, 0.0) | Center position                       |
| segments | Int    | 64         | Number of line segments in the approximation |

**Outputs:**

| Name | Type | Description       |
|------|------|-------------------|
| path | Path | The circular path |

**Notes:** If a Points batch is connected to the `center` input, the node automatically creates one circle per point, merging all results into a single path. Higher segment counts produce smoother circles at the cost of more vertices.

```
Example patch: Circle (radius: 50) -> Set Fill (color: red) -> Graph Output
```

---

### Rectangle

Creates an axis-aligned rectangular path.

**Inputs:**

| Name   | Type   | Default    | Description         |
|--------|--------|------------|---------------------|
| width  | Scalar | 200.0      | Width of rectangle  |
| height | Scalar | 100.0      | Height of rectangle |
| center | Vec2   | (0.0, 0.0) | Center position    |

**Outputs:**

| Name | Type | Description            |
|------|------|------------------------|
| path | Path | The rectangular path   |

**Notes:** Like Circle, connecting a Points batch to `center` produces one rectangle per point. The rectangle is centered on the given position.

```
Example patch: Rectangle (200x100) -> Set Stroke (black, 2px) -> Graph Output
```

---

### Regular Polygon

Creates a regular polygon with a specified number of sides.

**Inputs:**

| Name   | Type   | Default    | Description                    |
|--------|--------|------------|--------------------------------|
| sides  | Int    | 6          | Number of sides (minimum 3)    |
| radius | Scalar | 100.0      | Outer radius (center to vertex)|
| center | Vec2   | (0.0, 0.0) | Center position               |

**Outputs:**

| Name | Type | Description     |
|------|------|-----------------|
| path | Path | The polygon path |

**Notes:** With 3 sides you get a triangle, 5 a pentagon, 6 a hexagon, etc. Auto-iterates when a Points batch is connected to `center`.

```
Example patch: Regular Polygon (sides: 5) -> Set Fill (gold) -> Graph Output
```

---

### Line

Creates a straight line segment between two points.

**Inputs:**

| Name | Type | Default         | Description |
|------|------|-----------------|-------------|
| from | Vec2 | (-100.0, 0.0)  | Start point |
| to   | Vec2 | (100.0, 0.0)   | End point   |

**Outputs:**

| Name | Type | Description                  |
|------|------|------------------------------|
| path | Path | Open path with two vertices  |

**Notes:** The output is an open path (not closed). To make it visible, connect it through a Set Stroke node. Unlike other generators, Line does not auto-iterate on Points batches.

```
Example patch: Line -> Set Stroke (white, 3px) -> Graph Output
```

---

### Point Grid

Generates a rectangular grid of evenly-spaced points.

**Inputs:**

| Name    | Type   | Default | Description                     |
|---------|--------|---------|---------------------------------|
| columns | Int    | 10      | Number of columns               |
| rows    | Int    | 10      | Number of rows                  |
| spacing | Scalar | 20.0    | Distance between adjacent points |

**Outputs:**

| Name   | Type   | Description              |
|--------|--------|--------------------------|
| points | Points | Grid of points           |

**Notes:** The grid is centered at the origin. A 10x10 grid with spacing 20 extends from -90 to +90 on each axis. Feed the output into a generator's `center` input to instance geometry at each grid point.

```
Example patch: Point Grid (5x5, spacing: 40) -> Circle (radius: 15) -> Set Fill -> Graph Output
```

---

### Scatter Points

Generates randomly distributed points within a rectangular region.

**Inputs:**

| Name   | Type   | Default | Description                         |
|--------|--------|---------|-------------------------------------|
| count  | Int    | 100     | Number of points to generate        |
| width  | Scalar | 500.0   | Width of the scatter region         |
| height | Scalar | 500.0   | Height of the scatter region        |
| seed   | Int    | 0       | Random seed for reproducibility     |

**Outputs:**

| Name   | Type   | Description      |
|--------|--------|------------------|
| points | Points | Scattered points |

**Notes:** Uses a deterministic hash-based PRNG. The same seed always produces the same point positions, making animations reproducible. Change the seed to get a different distribution. Points are distributed in the range [-width/2, width/2] x [-height/2, height/2].

```
Example patch: Scatter Points (50, seed: 42) -> Circle (radius: 5) -> Set Fill -> Graph Output
```

---

### Load Image

Loads an image file from disk and outputs it as an Image.

**Inputs:**

| Name     | Type   | Default    | Description                                     |
|----------|--------|------------|-------------------------------------------------|
| position | Vec2   | (0.0, 0.0) | Center position of the image                   |
| width    | Scalar | 0.0        | Display width (0 = use native pixel width)      |
| height   | Scalar | 0.0        | Display height (0 = use native pixel height)    |
| opacity  | Scalar | 1.0        | Opacity from 0.0 (transparent) to 1.0 (opaque) |

**Outputs:**

| Name          | Type   | Description                      |
|---------------|--------|----------------------------------|
| image         | Image  | The loaded image with transforms |
| native_width  | Scalar | Native pixel width of the image  |
| native_height | Scalar | Native pixel height of the image |

**Properties:**

| Name | Description                                              |
|------|----------------------------------------------------------|
| Path | File path to the image (text field + file browser button) |

**Notes:** Supported formats: PNG, JPEG, GIF, WebP, and BMP. Relative paths are resolved relative to the project file's directory. The file browser button opens a native file dialog for selecting images. Images are cached after first load. When both width and height are 0, the image displays at its native pixel size. The image can be further transformed by connecting it through Translate, Rotate, or Scale nodes.

```
Example patch: Load Image ("logo.png", width: 200) -> Graph Output
```

---

### SVG Path

Creates a path from an SVG path data string (the `d` attribute of an SVG `<path>` element).

**Inputs:** None

**Outputs:**

| Name | Type | Description      |
|------|------|------------------|
| path | Path | The parsed path  |

**Properties:**

| Name      | Description                                          |
|-----------|------------------------------------------------------|
| Path Data | Multiline text field for SVG path `d` attribute data |

**Notes:** Supports the full set of SVG path commands: `M`/`m` (move to), `L`/`l` (line to), `H`/`h` (horizontal line), `V`/`v` (vertical line), `C`/`c` (cubic Bezier), `S`/`s` (smooth cubic), `Q`/`q` (quadratic Bezier), `T`/`t` (smooth quadratic), `A`/`a` (arc), and `Z`/`z` (close path). Both absolute (uppercase) and relative (lowercase) commands are supported. Implicit repeated coordinates are handled (e.g., multiple coordinate pairs after `L` create successive line segments). The path data must start with an `M` or `m` command. Validation errors are shown in red below the editor. Parsed paths are cached for performance.

```
Example: SVG Path ("M0,0 L100,0 L100,100 Z") -> Set Fill (red) -> Graph Output
```

---

## Transforms

Transform nodes apply 2D affine transformations to any geometry type, including paths, shapes, images, and point batches.

### Translate

Moves geometry by an offset.

**Inputs:**

| Name     | Type | Default    | Description                 |
|----------|------|------------|-----------------------------|
| geometry | Any  | --         | Input geometry to translate |
| offset   | Vec2 | (0.0, 0.0) | Translation offset (x, y) |

**Outputs:**

| Name     | Type | Description          |
|----------|------|----------------------|
| geometry | Any  | Translated geometry  |

**Notes:** Applies to all geometry types (Path, Shape, Image, Points, and their batch variants). Non-geometry types pass through unchanged.

```
Example patch: Circle -> Translate (offset: 100, 50) -> Graph Output
```

---

### Rotate

Rotates geometry around a center point.

**Inputs:**

| Name     | Type   | Default    | Description                      |
|----------|--------|------------|----------------------------------|
| geometry | Any    | --         | Input geometry to rotate         |
| angle    | Scalar | 0.0        | Rotation angle in **degrees**    |
| center   | Vec2   | (0.0, 0.0) | Center of rotation              |

**Outputs:**

| Name     | Type | Description       |
|----------|------|-------------------|
| geometry | Any  | Rotated geometry  |

**Notes:** Angle is specified in degrees (not radians). Positive angles rotate counter-clockwise. Rotation is applied around the specified center point.

```
Example patch: Rectangle -> Rotate (angle: 45) -> Set Fill -> Graph Output
```

---

### Scale

Scales geometry around a center point with independent X and Y factors.

**Inputs:**

| Name     | Type   | Default    | Description                    |
|----------|--------|------------|--------------------------------|
| geometry | Any    | --         | Input geometry to scale        |
| factor   | Vec2   | (1.0, 1.0) | Scale factor (x, y)          |
| center   | Vec2   | (0.0, 0.0) | Center of scaling             |

**Outputs:**

| Name     | Type | Description      |
|----------|------|------------------|
| geometry | Any  | Scaled geometry  |

**Notes:** Non-uniform scaling is supported -- use different X and Y factors to stretch geometry. A factor of (2.0, 0.5) doubles the width and halves the height.

```
Example patch: Circle -> Scale (factor: 2.0, 0.5) -> Set Stroke -> Graph Output
```

---

### Apply Transform

Applies a pre-computed affine transform matrix to geometry.

**Inputs:**

| Name      | Type      | Default  | Description                    |
|-----------|-----------|----------|--------------------------------|
| geometry  | Any       | --       | Input geometry                 |
| transform | Transform | Identity | Affine2 transform matrix       |

**Outputs:**

| Name     | Type | Description          |
|----------|------|----------------------|
| geometry | Any  | Transformed geometry |

**Notes:** This node applies a Transform value directly. It is useful when a transform is computed by another node or VFS expression and you want to apply it to geometry.

---

## Path Operations

Path operation nodes modify or combine geometric paths.

### Path Union

Merges multiple paths or shapes into a single Shapes batch.

**Inputs:**

| Name | Type | Default | Description    |
|------|------|---------|----------------|
| a    | Any  | --      | First input    |
| b    | Any  | --      | Second input   |

Additional inputs can be added dynamically.

**Outputs:**

| Name   | Type   | Description                         |
|--------|--------|-------------------------------------|
| result | Shapes | All inputs merged into one batch    |

**Special behavior:** This node has **variadic inputs**. It starts with two inputs (`a` and `b`), but you can add more using the **+** button in the properties panel. Remove extra inputs with the **-** button. Inputs are named alphabetically: `a`, `b`, `c`, `d`, etc. Each input accepts any geometry type -- paths are automatically promoted to shapes. Empty inputs are skipped.

```
Example patch:
  Circle -> Set Fill (red)  -> Path Union
  Rectangle -> Set Fill (blue) -> Path Union -> Graph Output
```

---

### Path Intersect

Computes the intersection of two paths.

**Inputs:**

| Name | Type | Default | Description  |
|------|------|---------|--------------|
| a    | Path | --      | First path   |
| b    | Path | --      | Second path  |

**Outputs:**

| Name   | Type | Description         |
|--------|------|---------------------|
| result | Path | Intersection result |

**Notes:** Boolean path operations are currently stubbed -- the node passes through the first input path. Full boolean geometry is planned for a future release.

---

### Path Difference

Subtracts one path from another.

**Inputs:**

| Name | Type | Default | Description       |
|------|------|---------|-------------------|
| a    | Path | --      | Base path         |
| b    | Path | --      | Path to subtract  |

**Outputs:**

| Name   | Type | Description        |
|--------|------|--------------------|
| result | Path | Difference (a - b) |

**Notes:** Currently stubbed -- passes through the first input path.

---

### Path Offset

Expands or contracts a path by a given distance.

**Inputs:**

| Name     | Type   | Default | Description                                 |
|----------|--------|---------|---------------------------------------------|
| path     | Path   | --      | Input path                                  |
| distance | Scalar | 10.0    | Offset distance (positive = outward, negative = inward) |

**Outputs:**

| Name   | Type | Description  |
|--------|------|--------------|
| result | Path | Offset path  |

```
Example patch: Circle (50) -> Path Offset (distance: 10) -> Set Stroke -> Graph Output
```

---

### Path Subdivide

Adds midpoints to path segments, increasing vertex density.

**Inputs:**

| Name   | Type | Default | Description                               |
|--------|------|---------|-------------------------------------------|
| path   | Path | --      | Input path                                |
| levels | Int  | 1       | Number of subdivision levels              |

**Outputs:**

| Name   | Type | Description     |
|--------|------|-----------------|
| result | Path | Subdivided path |

**Notes:** Each level doubles the number of segments by inserting a midpoint between each pair of consecutive vertices. For curves, De Casteljau splitting at t=0.5 is used. Multiple levels compound: level 2 produces 4x the original segments.

---

### Path Reverse

Reverses the winding direction of a path.

**Inputs:**

| Name | Type | Default | Description |
|------|------|---------|-------------|
| path | Path | --      | Input path  |

**Outputs:**

| Name   | Type | Description   |
|--------|------|---------------|
| result | Path | Reversed path |

**Notes:** The path's closed/open status is preserved. This is useful for controlling fill rules or combining paths where winding direction matters.

---

### Resample Path

Samples evenly-spaced points along a path.

**Inputs:**

| Name  | Type | Default | Description                        |
|-------|------|---------|------------------------------------|
| path  | Path | --      | Input path                         |
| count | Int  | 32      | Number of points to sample         |

**Outputs:**

| Name   | Type   | Description                      |
|--------|--------|----------------------------------|
| points | Points | Evenly-distributed sample points |

**Notes:** Points are distributed by arc length, so they are evenly spaced along the path regardless of how the original vertices are distributed. This is useful for instancing geometry along a path or extracting a point cloud from a shape.

```
Example patch: Circle (100) -> Resample Path (count: 12) -> Regular Polygon (sides: 3, radius: 10) -> Set Fill -> Graph Output
```

---

## Styling

Styling nodes apply visual appearance to geometry, converting raw paths into renderable shapes.

### Set Fill

Applies a fill color to a shape.

**Inputs:**

| Name  | Type  | Default           | Description |
|-------|-------|-------------------|-------------|
| shape | Shape | --                | Input shape |
| color | Color | (1.0, 1.0, 1.0, 1.0) | Fill color (white) |

**Outputs:**

| Name  | Type  | Description        |
|-------|-------|--------------------|
| shape | Shape | Shape with fill    |

**Notes:** If a raw Path is connected to the `shape` input, it is automatically promoted to a Shape. Only closed paths produce a visible fill.

```
Example patch: Circle -> Set Fill (color: cornflowerblue) -> Graph Output
```

---

### Set Stroke

Applies a stroke (outline) to a shape.

**Inputs:**

| Name  | Type   | Default           | Description          |
|-------|--------|-------------------|----------------------|
| shape | Shape  | --                | Input shape          |
| color | Color  | (0.0, 0.0, 0.0, 1.0) | Stroke color (black) |
| width | Scalar | 2.0               | Stroke width in pixels |

**Outputs:**

| Name  | Type  | Description          |
|-------|-------|----------------------|
| shape | Shape | Shape with stroke    |

**Notes:** Both open and closed paths can have strokes. Stroke uses round line caps and miter joins. Chain Set Fill and Set Stroke to get both a fill and an outline.

```
Example patch: Circle -> Set Fill (red) -> Set Stroke (black, 3px) -> Graph Output
```

---

## Color

Color operation nodes manipulate color values. All color nodes handle both single Color values and batched Colors transparently.

### Adjust Hue

Shifts or sets the hue of a color.

**Inputs:**

| Name     | Type   | Default | Description                                        |
|----------|--------|---------|----------------------------------------------------|
| color    | Color  | White   | Input color                                        |
| amount   | Scalar | 0.0     | Hue value in degrees (0-360)                       |
| absolute | Bool   | false   | If false, shift hue by amount. If true, set hue to amount. |

**Outputs:**

| Name  | Type  | Description    |
|-------|-------|----------------|
| color | Color | Adjusted color |

**Notes:** Operates in HSL color space. A shift of 180 gives the complementary color. Hue wraps around at 360 degrees.

```
Example patch: Color Parse ("#FF6600") -> Adjust Hue (amount: 120) -> Set Fill
```

---

### Adjust Saturation

Adjusts the saturation of a color.

**Inputs:**

| Name     | Type   | Default | Description                                    |
|----------|--------|---------|------------------------------------------------|
| color    | Color  | White   | Input color                                    |
| amount   | Scalar | 0.0     | Saturation adjustment (-1.0 to 1.0)           |
| absolute | Bool   | false   | If true, set saturation directly (0.0 to 1.0) |

**Outputs:**

| Name  | Type  | Description    |
|-------|-------|----------------|
| color | Color | Adjusted color |

**Notes:** Operates in HSL space. An amount of -1.0 in shift mode fully desaturates (grayscale). The result is clamped to [0, 1].

---

### Adjust Lightness

Adjusts the lightness of a color.

**Inputs:**

| Name     | Type   | Default | Description                                   |
|----------|--------|---------|-----------------------------------------------|
| color    | Color  | White   | Input color                                   |
| amount   | Scalar | 0.0     | Lightness adjustment (-1.0 to 1.0)           |
| absolute | Bool   | false   | If true, set lightness directly (0.0 to 1.0) |

**Outputs:**

| Name  | Type  | Description    |
|-------|-------|----------------|
| color | Color | Adjusted color |

**Notes:** Operates in HSL space. A lightness of 0.0 is black, 0.5 is the pure color, and 1.0 is white.

---

### Adjust Luminance

Adjusts the perceptual luminance of a color using CIE Lab color space.

**Inputs:**

| Name     | Type   | Default | Description                                       |
|----------|--------|---------|---------------------------------------------------|
| color    | Color  | White   | Input color                                       |
| amount   | Scalar | 0.0     | L* adjustment (0 to 100 scale)                    |
| absolute | Bool   | false   | If true, set L* directly; if false, shift L*      |

**Outputs:**

| Name  | Type  | Description    |
|-------|-------|----------------|
| color | Color | Adjusted color |

**Notes:** CIE Lab L* is perceptually uniform -- equal changes in L* look like equal changes in brightness to the human eye. This produces more natural results than HSL lightness. Uses D65 illuminant for the conversion.

---

### Invert Color

Inverts the RGB channels of a color.

**Inputs:**

| Name  | Type  | Default | Description |
|-------|-------|---------|-------------|
| color | Color | White   | Input color |

**Outputs:**

| Name  | Type  | Description    |
|-------|-------|----------------|
| color | Color | Inverted color |

**Notes:** Each RGB channel is inverted: (r, g, b, a) becomes (1-r, 1-g, 1-b, a). The alpha channel is preserved.

---

### Grayscale

Converts a color to grayscale using perceptual luminance weighting.

**Inputs:**

| Name  | Type  | Default | Description |
|-------|-------|---------|-------------|
| color | Color | White   | Input color |

**Outputs:**

| Name  | Type  | Description      |
|-------|-------|------------------|
| color | Color | Grayscale color  |

**Notes:** Uses the BT.709 luminance formula: L = 0.2126R + 0.7152G + 0.0722B. The result is (L, L, L, alpha). This weights green most heavily, matching human brightness perception.

---

### Mix Colors

Blends two colors together.

**Inputs:**

| Name     | Type   | Default | Description                                      |
|----------|--------|---------|--------------------------------------------------|
| color_a  | Color  | Black   | First color                                      |
| color_b  | Color  | White   | Second color                                     |
| factor   | Scalar | 0.5     | Mix factor (0.0 = all A, 1.0 = all B)           |
| lab_mode | Bool   | false   | If true, blend in CIE Lab space instead of RGB   |

**Outputs:**

| Name  | Type  | Description   |
|-------|-------|---------------|
| color | Color | Blended color |

**Notes:** RGB blending is fast but can produce muddy intermediate colors. Lab blending produces perceptually smoother gradients -- for example, blending red and green in Lab avoids the brown you get in RGB.

```
Example patch: Color Parse ("red") -> Mix Colors (factor: 0.5, lab_mode: true)
              Color Parse ("blue") -> Mix Colors -> Set Fill
```

---

### Adjust Alpha

Adjusts the alpha (transparency) channel of a color.

**Inputs:**

| Name     | Type   | Default | Description                                            |
|----------|--------|---------|--------------------------------------------------------|
| color    | Color  | White   | Input color                                            |
| amount   | Scalar | 0.0     | Alpha adjustment (-1.0 to 1.0)                        |
| absolute | Bool   | false   | If true, set alpha directly; if false, shift alpha     |

**Outputs:**

| Name  | Type  | Description              |
|-------|-------|--------------------------|
| color | Color | Color with adjusted alpha |

**Notes:** In the default relative mode, the amount is added to the existing alpha (e.g., -0.3 on a color with alpha 0.8 gives 0.5). In absolute mode, the amount replaces the alpha directly. The result is clamped to [0, 1]. RGB channels are preserved.

---

### Color Parse

Parses a color from a hex string or CSS color name.

**Inputs:** None

**Outputs:**

| Name  | Type  | Description   |
|-------|-------|---------------|
| color | Color | Parsed color  |

**Properties:**

| Name  | Description                                            |
|-------|--------------------------------------------------------|
| Color | Text field for hex code or CSS color name               |

**Notes:** Accepts hex codes (`#RRGGBB` or `#RRGGBBAA`) and approximately 148 CSS named colors (e.g., `red`, `cornflowerblue`, `tomato`, `darkslategray`). Parsing is case-insensitive. If the text cannot be parsed, the output defaults to black.

```
Example patch: Color Parse ("tomato") -> Set Fill
              Color Parse ("#3366CCAA") -> Set Fill  // with alpha
```

---

## Constants

Constant nodes output fixed values that can be edited in the properties panel. They serve as configurable parameters for your graph.

### Constant Scalar

Outputs a floating-point number.

**Inputs:**

| Name  | Type   | Default | Description     |
|-------|--------|---------|-----------------|
| value | Scalar | 0.0     | The scalar value |

**Outputs:**

| Name  | Type   | Description      |
|-------|--------|------------------|
| value | Scalar | The scalar value |

**Notes:** Edited via a drag-value slider in the properties panel.

---

### Constant Int

Outputs an integer.

**Inputs:**

| Name  | Type | Default | Description      |
|-------|------|---------|------------------|
| value | Int  | 0       | The integer value |

**Outputs:**

| Name  | Type | Description       |
|-------|------|-------------------|
| value | Int  | The integer value |

---

### Constant Vec2

Outputs a 2D vector from separate X and Y components.

**Inputs:**

| Name | Type   | Default | Description |
|------|--------|---------|-------------|
| x    | Scalar | 0.0     | X component |
| y    | Scalar | 0.0     | Y component |

**Outputs:**

| Name  | Type | Description       |
|-------|------|-------------------|
| value | Vec2 | The 2D vector     |

---

### Constant Color

Outputs a color value.

**Inputs:**

| Name  | Type  | Default | Description |
|-------|-------|---------|-------------|
| color | Color | White   | The color   |

**Outputs:**

| Name  | Type  | Description |
|-------|-------|-------------|
| value | Color | The color   |

**Notes:** Edited via an interactive color picker in the properties panel.

---

## Utility

### Merge

Combines two geometry inputs into a single output.

**Inputs:**

| Name | Type | Default | Description  |
|------|------|---------|--------------|
| a    | Any  | --      | First input  |
| b    | Any  | --      | Second input |

**Outputs:**

| Name   | Type | Description     |
|--------|------|-----------------|
| merged | Any  | Combined result |

**Notes:** The merge behavior depends on the input types:
- **Path + Path** -- merged into a single multi-contour Path
- **Shape + Shape** -- combined into a Shapes batch
- **Paths + Path(s)** -- concatenated into a larger Paths batch
- **Shapes + Shape(s)** -- concatenated into a larger Shapes batch

Types are automatically promoted to match when possible.

```
Example patch:
  Circle -> Set Fill (red)  -> Merge
  Rectangle -> Set Fill (blue) -> Merge -> Graph Output
```

---

### Duplicate

Creates multiple copies of geometry with a cumulative transform applied to each copy.

**Inputs:**

| Name      | Type      | Default  | Description                              |
|-----------|-----------|----------|------------------------------------------|
| geometry  | Any       | --       | Geometry to duplicate                    |
| count     | Int       | 5        | Number of copies                         |
| transform | Transform | Identity | Transform applied cumulatively per copy  |

**Outputs:**

| Name     | Type | Description               |
|----------|------|---------------------------|
| geometry | Any  | All copies merged         |

**Notes:** The transform is applied cumulatively -- copy 1 gets the transform once, copy 2 gets it applied twice, copy 3 three times, and so on. This makes it easy to create radial arrays, linear sequences, or spirals. If count is 0, the input passes through unchanged.

```
Example patch: Rectangle (40x40) -> Duplicate (count: 10, transform: Translate 45,0 + Rotate 15) -> Set Fill -> Graph Output
```

---

### Portal Send

Sends a value to matching Portal Receive nodes anywhere in the graph.

**Inputs:**

| Name  | Type | Default | Description    |
|-------|------|---------|----------------|
| value | Any  | --      | Value to send  |

**Outputs:**

| Name    | Type | Description                         |
|---------|------|-------------------------------------|
| through | Any  | Pass-through of the input value     |

**Properties:**

| Name  | Description                                |
|-------|--------------------------------------------|
| Label | Name that Portal Receive nodes match against |

**Notes:** Portals allow data to flow between distant parts of the graph without drawing long wires. A Portal Send makes its input value available to any Portal Receive with the same label. The `through` output passes the input value unchanged, allowing the send node to be inserted mid-chain. The node title displays as "Send: {label}".

---

### Portal Receive

Receives a value from a matching Portal Send node.

**Inputs:** None

**Outputs:**

| Name  | Type | Description    |
|-------|------|----------------|
| value | Any  | Received value |

**Properties:**

| Name  | Description                            |
|-------|----------------------------------------|
| Label | Name to match against Portal Send nodes |

**Notes:** Outputs the value from the Portal Send node with the same label. If no matching send exists, outputs a default value (0.0 Scalar). The node title displays as "Receive: {label}".

```
Example:
  Circle -> Set Fill -> Portal Send (label: "my_shape")
  ... elsewhere in graph ...
  Portal Receive (label: "my_shape") -> Translate -> Graph Output
```

---

### VFS Code

A programmable node with user-defined inputs, outputs, and script logic.

**Inputs:** Defined by the user in the properties panel (Scalar, Int, and Color types supported).

**Outputs:** Defined by the user in the properties panel (Scalar, Int, and Color types supported).

**Properties:**

| Name         | Description                                    |
|--------------|------------------------------------------------|
| Source       | Multiline code editor for the VFS script       |
| Input Ports  | Add/remove/rename input ports with type selection |
| Output Ports | Add/remove/rename output ports with type selection |

**Notes:** The VFS Code node lets you write custom computation logic using Vector Flow Script. The script runs in **script mode** -- bare statements without a function wrapper. Input port values are available as pre-declared variables matching the port names. Assign to output port names, or use a tail expression (no semicolon on the last line) to set the first output.

The code is compiled to native machine code via Cranelift JIT and cached by source+port signature. Compilation errors are displayed in red below the code editor. The global variables `time`, `frame`, and `fps` are available for animation.

See the [VFS Reference](vfs-reference.md) for the full language documentation.

**Example -- oscillator:**

Inputs: `freq` (Scalar), `amp` (Scalar)
Outputs: `value` (Scalar)

```
sin(time * freq * TAU) * amp
```

**Example -- step counter:**

Inputs: `steps` (Int), `spread` (Scalar)
Outputs: `x` (Scalar), `y` (Scalar)

```
let angle = frame as Scalar * TAU / steps as Scalar;
x = cos(angle) * spread;
y = sin(angle) * spread;
```

---

## Graph I/O

Graph I/O nodes define the interface between the node graph and the canvas output.

### Graph Output

Marks a node's input as a final output of the graph, making it visible on the canvas.

**Inputs:**

| Name   | Type | Default | Description                    |
|--------|------|---------|--------------------------------|
| {name} | Any  | --      | Accepts any data type          |

**Outputs:** None

**Notes:** When no nodes are selected in the graph editor, the canvas displays the results of all Graph Output nodes. If specific nodes are selected or pinned, those are shown instead. You can have multiple Graph Output nodes to compose a final scene from several branches.

```
Example patch:
  Circle -> Set Fill (red) -> Graph Output ("circles")
  Line -> Set Stroke (white) -> Graph Output ("lines")
```

---

### Graph Input

Declares a typed input to the graph for external value injection.

**Inputs:** None

**Outputs:**

| Name   | Type       | Description                  |
|--------|------------|------------------------------|
| {name} | (declared) | The externally-provided value |

**Notes:** Graph Input nodes are created programmatically. They allow external code or a parent graph to feed values into the node network.
