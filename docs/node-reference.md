# Vector Flow Node Reference

This document describes every node type available in Vector Flow, organized by category. Each entry covers the node's purpose, inputs, outputs, properties, and any special behavior.

## Table of Contents

- [Data Types](#data-types)
- [Generators](#generators)
  - [Arc](#arc)
  - [Circle](#circle)
  - [Line](#line)
  - [Load Image](#load-image)
  - [Point Grid](#point-grid)
  - [Rectangle](#rectangle)
  - [Regular Polygon](#regular-polygon)
  - [Scatter Points](#scatter-points)
  - [SVG Path](#svg-path)
- [Transforms](#transforms)
  - [Apply Transform](#apply-transform)
  - [Rotate](#rotate)
  - [Scale](#scale)
  - [Translate](#translate)
  - [Warp to Curve](#warp-to-curve)
- [Path Operations](#path-operations)
  - [Close Path](#close-path)
  - [Path Boolean](#path-boolean)
  - [Path Intersection Points](#path-intersection-points)
  - [Path Offset](#path-offset)
  - [Path Reverse](#path-reverse)
  - [Path Subdivide](#path-subdivide)
  - [Polygon from Points](#polygon-from-points)
  - [Resample Path](#resample-path)
  - [Spline from Points](#spline-from-points)
  - [Split Path at T](#split-path-at-t)
- [Styling](#styling)
  - [Set Fill](#set-fill)
  - [Set Stroke](#set-stroke)
  - [Set Style](#set-style)
  - [Stroke to Path](#stroke-to-path)
- [Color](#color)
  - [Adjust Alpha](#adjust-alpha)
  - [Adjust Hue](#adjust-hue)
  - [Adjust Lightness](#adjust-lightness)
  - [Adjust Luminance](#adjust-luminance)
  - [Adjust Saturation](#adjust-saturation)
  - [Color Parse](#color-parse)
  - [Grayscale](#grayscale)
  - [Invert Color](#invert-color)
  - [Mix Colors](#mix-colors)
- [Text](#text)
  - [Text](#text-1)
  - [Text to Path](#text-to-path)
- [Constants](#constants)
  - [Constant Color](#constant-color)
  - [Constant Int](#constant-int)
  - [Constant Scalar](#constant-scalar)
  - [Constant Vec2](#constant-vec2)
- [Utility](#utility)
  - [Copy to Points](#copy-to-points)
  - [Duplicate](#duplicate)
  - [Merge](#merge)
  - [Pack Points](#pack-points)
  - [Place at Points](#place-at-points)
  - [Portal Receive](#portal-receive)
  - [Portal Send](#portal-send)
- [Code](#code)
  - [Generate](#generate)
  - [Map](#map)
  - [VFS Code](#vfs-code)
- [Graph I/O](#graph-io)
  - [Graph Input](#graph-input)
  - [Graph Output](#graph-output)

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
| Text      | Styled text with font, layout, and color       |
| Any       | Accepts any of the above types                 |

**Automatic type promotion:** Some nodes accept broader types than their inputs strictly require. For example, a node expecting a Shape will accept a Path (promoted to a Shape with default styling). A node expecting Scalar will accept Int (promoted to float).

---

## Generators

Generators create geometry from parameters. They are the starting points of most graphs.

### Arc

Creates an arc, wedge (pie slice), or donut wedge (annular sector) path using cubic Bézier curves.

**Inputs:**

| Name         | Type   | Default    | Description                                      |
|--------------|--------|------------|--------------------------------------------------|
| outer_radius | Scalar | 100.0      | Outer radius of the arc                          |
| inner_radius | Scalar | 0.0        | Inner radius (0 = wedge/arc, >0 = donut)         |
| start_angle  | Scalar | 0.0        | Start angle in degrees                           |
| sweep_angle  | Scalar | 90.0       | Sweep angle in degrees                           |
| close        | Bool   | true       | Whether to close the shape                        |
| center       | Vec2   | (0.0, 0.0) | Center position                                 |

**Outputs:**

| Name | Type | Description  |
|------|------|--------------|
| path | Path | The arc path |

**Notes:** The shape produced depends on the combination of `close` and `inner_radius`:

- **Open arc** (`close=0`, `inner_radius=0`): a curved stroke with no fill area.
- **Wedge** (`close=1`, `inner_radius=0`): a pie-slice shape — arc plus two radial lines to the center.
- **Donut wedge** (`close=1`, `inner_radius>0`): an annular sector — outer arc, two radial sides, and an inner arc.
- **Full circle/ring**: set `sweep_angle=360` for a complete circle or ring.

If a Points batch is connected to the `center` input, the node creates one arc per point.

```
Example patch: Arc (sweep_angle: 60, close: 1) -> Set Fill (color: orange) -> Graph Output
```

---

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

### Warp to Curve

Deforms geometry to follow a spine curve, mapping the source bounding box onto the curve's arc length.

**Inputs:**

| Name      | Type   | Default | Hidden | Description                                              |
|-----------|--------|---------|--------|----------------------------------------------------------|
| geometry  | Any    | --      | No     | Input geometry to warp                                   |
| spine     | Path   | --      | No     | Spine curve to warp along                                |
| tolerance | Scalar | 0.0     | Yes    | Curve flattening tolerance (0 = zoom-aware)              |

**Parameters (property panel):**

| Name | Values                     | Default | Description                                              |
|------|----------------------------|---------|----------------------------------------------------------|
| Mode | Simple, Curvature-Aware    | Simple  | Warping algorithm                                        |

**Outputs:**

| Name     | Type | Description       |
|----------|------|-------------------|
| geometry | Any  | Warped geometry   |

**Notes:** The source geometry's bounding box is mapped onto the spine: the horizontal axis maps to arc length along the curve, and the vertical axis maps to perpendicular offset from the curve. **Simple** mode performs a direct positional mapping. **Curvature-Aware** mode adjusts for bend distortion, compressing perpendicular offsets on tight curves to produce more uniform results. Handles Path, Paths, Shape, and Shapes — for batches, a collective bounding box is computed so all elements are warped consistently.

```
Example patch: Rectangle (200x50) -> Warp to Curve (spine: Circle) -> Set Stroke -> Graph Output
```

---

## Path Operations

Path operation nodes modify or combine geometric paths.

### Close Path

Closes open paths by appending a Close verb and setting the closed flag.

**Inputs:**

| Name | Type | Default | Description                    |
|------|------|---------|--------------------------------|
| path | Any  | --      | Input geometry to close        |

**Outputs:**

| Name | Type | Description    |
|------|------|----------------|
| path | Any  | Closed geometry |

**Notes:** Sets `closed=true` and appends a Close verb to open paths. Works on Path, Paths, Shape, and Shapes — already-closed paths pass through unchanged.

---

### Path Boolean

Performs boolean geometry operations on two closed paths using the [i_overlay](https://crates.io/crates/i_overlay) library.

**Inputs:**

| Name      | Type   | Default | Hidden | Description                                         |
|-----------|--------|---------|--------|-----------------------------------------------------|
| a         | Path   | --      | No     | First path                                          |
| b         | Path   | --      | No     | Second path                                         |
| tolerance | Scalar | 0.0     | Yes    | Curve flattening tolerance (0 = zoom-aware)         |

**Properties:**

| Name      | Values                              | Default | Description              |
|-----------|-------------------------------------|---------|--------------------------|
| Operation | Union, Intersect, Difference, Xor, Divide | Union   | Boolean operation to perform |

**Outputs:**

| Name   | Type  | Description                                      |
|--------|-------|--------------------------------------------------|
| result | Path  | Combined result of boolean op (all contours)     |
| parts  | Paths | Individual non-overlapping regions as separate paths |

**Operations:**

- **Union** — combines both paths into a single outline covering the area of both.
- **Intersect** — keeps only the area where both paths overlap.
- **Difference** — subtracts path `b` from path `a`, keeping only the area in `a` that does not overlap with `b`.
- **Xor** — keeps the area covered by exactly one of the two paths, excluding the overlap.
- **Divide** — splits both paths into all distinct non-overlapping regions. For two overlapping shapes this produces up to three parts: the area unique to `a`, the intersection, and the area unique to `b`. Empty regions are omitted.

**Notes:** Input paths are flattened to polygon approximations before the boolean operation. When tolerance is 0 (the default), zoom-aware tolerance is used automatically, so results stay precise at any zoom level. Set a positive tolerance value to override with a fixed precision. The output is always a polygon path (no curves). Both paths should be closed for meaningful results. If the paths do not overlap, Union returns both contours, Intersect returns an empty path, and Difference returns path `a` unchanged.

```
Example patch: Circle (50) -> [a] Path Boolean (Difference) [b] <- Rectangle (40x40)
```

---

### Path Intersection Points

Finds all intersection points between two paths.

**Inputs:**

| Name      | Type   | Default | Hidden | Description                                         |
|-----------|--------|---------|--------|-----------------------------------------------------|
| a         | Path   | --      | No     | First path                                          |
| b         | Path   | --      | No     | Second path                                         |
| tolerance | Scalar | 0.0     | Yes    | Curve flattening tolerance (0 = zoom-aware)         |

**Outputs:**

| Name   | Type    | Description                                                |
|--------|---------|------------------------------------------------------------|
| points | Points  | Intersection positions                                     |
| t_a    | Scalars | Arc-length normalized parameters (0..1) on path `a`       |
| t_b    | Scalars | Arc-length normalized parameters (0..1) on path `b`       |
| count  | Int     | Number of intersection points found                        |

**Notes:** Both paths are flattened to polylines before intersection detection. The `t_a` and `t_b` outputs provide arc-length normalized parameters (0 = start, 1 = end) indicating where each intersection falls on its respective path. These parameters can be fed into Split Path at T to cut paths at their intersection points.

```
Example patch: Circle (80) -> [a] Path Intersection Points [b] <- Line (-100,-100 to 100,100)
```

---

### Path Offset

Expands or contracts a path by a given distance. Curves are flattened to line segments, offset with miter joins, and reassembled. Winding detection determines inside vs outside for correct offset direction.

**Inputs:**

| Name      | Type   | Default | Hidden | Description                                              |
|-----------|--------|---------|--------|----------------------------------------------------------|
| path      | Path   | --      | No     | Input path                                               |
| distance  | Scalar | 10.0    | No     | Offset distance (positive = outward, negative = inward)  |
| tolerance | Scalar | 0.0     | Yes    | Curve flattening tolerance (0 = zoom-aware)              |

**Outputs:**

| Name   | Type | Description  |
|--------|------|--------------|
| result | Path | Offset path  |

```
Example patch: Circle (50) -> Path Offset (distance: 10) -> Set Stroke -> Graph Output
```

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

### Polygon from Points

Constructs a path from an ordered list of points using straight line segments.

**Inputs:**

| Name   | Type   | Default | Description                            |
|--------|--------|---------|----------------------------------------|
| points | Points | --      | Ordered points to connect              |
| close  | Bool   | true    | Whether to close the polygon           |

**Outputs:**

| Name | Type | Description             |
|------|------|-------------------------|
| path | Path | Constructed polygon path |

**Notes:** Creates a path by placing a MoveTo at the first point, then a LineTo to each subsequent point. When `close` is true, a Close verb is appended to form a closed polygon. This is the inverse of Resample Path — it turns a point cloud back into geometry.

```
Example patch: Scatter Points (count: 5) -> Polygon from Points -> Set Stroke -> Graph Output
```

---

### Resample Path

Samples evenly-spaced points along a path.

**Inputs:**

| Name      | Type   | Default | Hidden | Description                                         |
|-----------|--------|---------|--------|-----------------------------------------------------|
| path      | Path   | --      | No     | Input path                                          |
| count     | Int    | 32      | No     | Number of points to sample                          |
| tolerance | Scalar | 0.0     | Yes    | Curve flattening tolerance (0 = zoom-aware)         |

**Outputs:**

| Name   | Type   | Description                      |
|--------|--------|----------------------------------|
| points | Points | Evenly-distributed sample points |

**Notes:** Points are distributed by arc length, so they are evenly spaced along the path regardless of how the original vertices are distributed. When tolerance is 0 (the default), zoom-aware tolerance is used automatically, so point placement stays precise at any zoom level. Set a positive tolerance value to override with a fixed precision. This is useful for instancing geometry along a path or extracting a point cloud from a shape.

```
Example patch: Circle (100) -> Resample Path (count: 12) -> Regular Polygon (sides: 3, radius: 10) -> Set Fill -> Graph Output
```

---

### Spline from Points

Fits a smooth cubic bezier spline through an ordered list of points using Catmull-Rom interpolation.

**Inputs:**

| Name    | Type   | Default | Hidden | Description                                        |
|---------|--------|---------|--------|----------------------------------------------------|
| points  | Points | --      | No     | Ordered points to fit the spline through           |
| close   | Bool   | false   | No     | Whether to close the spline into a loop            |
| tension | Scalar | 0.0     | Yes    | Spline tension (0 = natural curve, higher = tighter) |

**Outputs:**

| Name | Type | Description         |
|------|------|---------------------|
| path | Path | Smooth spline path  |

**Notes:** Generates cubic bezier curves that pass exactly through each input point. Tension 0.0 produces a natural-looking curve; increasing tension pulls the curve closer to the straight-line polygon connecting the points. When `close` is true, the spline wraps around so the last point connects smoothly back to the first. Requires at least 2 points.

```
Example patch: Point Grid (3x3) -> Spline from Points (close: true) -> Set Stroke -> Graph Output
```

---

### Split Path at T

Splits a path at arc-length normalized parameter values into multiple sub-paths.

**Inputs:**

| Name      | Type    | Default | Hidden | Description                                         |
|-----------|---------|---------|--------|-----------------------------------------------------|
| path      | Path    | --      | No     | Input path to split                                 |
| t_values  | Scalars | --      | No     | Arc-length normalized split positions (0..1)        |
| tolerance | Scalar  | 0.0     | Yes    | Curve flattening tolerance (0 = zoom-aware)         |
| close     | Bool    | false   | Yes    | Whether to close each resulting sub-path            |

**Outputs:**

| Name  | Type  | Description                    |
|-------|-------|--------------------------------|
| parts | Paths | Resulting sub-paths after splitting |
| count | Int   | Number of resulting parts      |

**Notes:** For an open path, N cuts produce N+1 parts. For a closed path, N cuts produce N parts (the path is "unwrapped" at cut points). The `t_values` input uses arc-length normalized parameters where 0.0 is the start and 1.0 is the end of the path. When `close` is true, each resulting sub-path gets a Close verb appended. Pairs naturally with Path Intersection Points, which outputs compatible t-values.

```
Example patch: Circle (100) -> Split Path at T (t_values: [0.25, 0.75]) -> Set Stroke -> Graph Output
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

| Name         | Type   | Default               | Description                                |
|--------------|--------|-----------------------|--------------------------------------------|
| shape        | Shape  | --                    | Input shape                                |
| color        | Color  | (0.0, 0.0, 0.0, 1.0) | Stroke color (black)                       |
| width        | Scalar | 2.0                   | Stroke width in pixels                     |
| cap          | Int    | 0                     | End cap style: 0=Butt, 1=Round, 2=Square   |
| join         | Int    | 0                     | Line join style: 0=Miter, 1=Round, 2=Bevel |
| miter_limit  | Scalar | 4.0                   | Miter limit (only applies to Miter join)   |
| dash_offset  | Scalar | 0.0                   | Dash pattern offset                        |
| tolerance    | Scalar | 0.0                   | Curve flattening tolerance (0 = zoom-aware) |

**Outputs:**

| Name  | Type  | Description          |
|-------|-------|----------------------|
| shape | Shape | Shape with stroke    |

**Properties:**

| Name         | Description                                                     |
|--------------|-----------------------------------------------------------------|
| Dash Pattern | Comma or space-separated dash/gap lengths (e.g., "10 5" or "10,5,2,5") |

**Notes:** Both open and closed paths can have strokes. Chain Set Fill and Set Stroke to get both a fill and an outline. When tolerance is 0 (the default), dash and stroke rendering uses zoom-aware tolerance automatically, so curves stay smooth at any zoom level. Set a positive tolerance value to override with a fixed precision.

```
Example patch: Circle -> Set Fill (red) -> Set Stroke (black, 3px) -> Graph Output
```

---

### Set Style

Combined fill + stroke styling node. Applies both fill and stroke to a shape in a single node, reducing graph clutter for the common case. Does not replace the separate Set Fill and Set Stroke nodes.

**Inputs:**

| Name           | Type   | Default               | Visible | Description                                |
|----------------|--------|-----------------------|---------|--------------------------------------------|
| path           | Any    | --                    | yes     | Input geometry                             |
| fill_color     | Color  | (1.0, 1.0, 1.0, 1.0) | yes     | Fill color                                 |
| fill_opacity   | Scalar | 1.0                   | no      | Fill opacity (0-1)                         |
| has_fill       | Bool   | true                  | no      | Enable fill                                |
| stroke_color   | Color  | (0.0, 0.0, 0.0, 1.0) | yes     | Stroke color                               |
| stroke_width   | Scalar | 2.0                   | yes     | Stroke width                               |
| stroke_opacity | Scalar | 1.0                   | no      | Stroke opacity (0-1)                       |
| has_stroke     | Bool   | true                  | no      | Enable stroke                              |
| cap            | Int    | 0                     | no      | End cap: 0=Butt, 1=Round, 2=Square        |
| join           | Int    | 0                     | no      | Line join: 0=Miter, 1=Round, 2=Bevel      |
| miter_limit    | Scalar | 4.0                   | no      | Miter limit (only for Miter join)          |
| dash_offset    | Scalar | 0.0                   | no      | Dash pattern offset                        |
| tolerance      | Scalar | 0.0                   | no      | Curve flattening tolerance (0 = zoom-aware) |

**Outputs:**

| Name   | Type | Description             |
|--------|------|-------------------------|
| output | Any  | Geometry with styling   |

**Properties:**

| Name         | Description                                                     |
|--------------|-----------------------------------------------------------------|
| Dash Pattern | Comma or space-separated dash/gap lengths (e.g., "10,5")       |

**Notes:** Hidden ports can be shown via the property sheet's visibility toggle. Set `has_fill` or `has_stroke` to false to skip that styling pass. For permanent single-style use, prefer the dedicated Set Fill or Set Stroke nodes.

```
Example patch: Circle -> Set Style (fill: red, stroke: black 3px) -> Graph Output
```

---

### Stroke to Path

Converts a stroke outline into a filled path. The resulting path traces the outline of what the stroke would look like, allowing it to be used as geometry for further operations.

**Inputs:**

| Name         | Type   | Default | Description                                |
|--------------|--------|---------|--------------------------------------------|
| shape        | Any    | --      | Input shape or path                        |
| width        | Scalar | 2.0     | Stroke width                               |
| cap          | Int    | 0       | End cap style: 0=Butt, 1=Round, 2=Square  |
| join         | Int    | 0       | Line join style: 0=Miter, 1=Round, 2=Bevel|
| miter_limit  | Scalar | 4.0     | Miter limit (only applies to Miter join)   |
| dash_offset  | Scalar | 0.0     | Dash pattern offset                        |
| tolerance    | Scalar | 0.0     | Curve flattening tolerance (0 = zoom-aware) |

**Outputs:**

| Name | Type | Description                         |
|------|------|-------------------------------------|
| path | Path | Filled path tracing the stroke outline |

**Properties:**

| Name         | Description                                                     |
|--------------|-----------------------------------------------------------------|
| Dash Pattern | Comma or space-separated dash/gap lengths (e.g., "10 5" or "10,5,2,5") |

**Notes:** This node tessellates the stroke into a triangle mesh, then extracts the boundary edges to produce a closed path. It supports all stroke parameters including dashes. An empty dash pattern produces a solid stroke outline. When tolerance is 0 (the default), curve flattening adapts to the current zoom level — the path is re-evaluated when you zoom in or out, producing smoother outlines at closer zoom. Set a positive tolerance to use a fixed precision instead. This is useful for creating outlined text effects, converting strokes into cuttable paths, or applying further path operations to a stroke shape.

```
Example patch: Circle -> Stroke to Path (width: 5, cap: Round) -> Set Fill (gold) -> Graph Output
```

---

## Color

Color operation nodes manipulate color values. All color nodes handle both single Color values and batched Colors transparently.

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

## Text

Text nodes create and manipulate text content on the canvas.

### Text

Creates a text element that renders on the canvas.

**Inputs:**

| Name           | Type   | Default    | Description                                     |
|----------------|--------|------------|-------------------------------------------------|
| position       | Vec2   | (0.0, 0.0) | Text anchor position                           |
| font_size      | Scalar | 24.0       | Font size in canvas units                       |
| font_weight    | Int    | 400        | Font weight (100-900; 400=Regular, 700=Bold)    |
| font_style     | Int    | 0          | Font style: 0=Normal, 1=Italic, 2=Oblique      |
| letter_spacing | Scalar | 0.0        | Extra spacing between glyphs                    |
| line_height    | Scalar | 1.2        | Line height multiplier                          |
| alignment      | Int    | 0          | Text alignment: 0=Left, 1=Center, 2=Right      |
| box_width      | Scalar | 0.0        | Text box width (0 = unconstrained)              |
| box_height     | Scalar | 0.0        | Text box height (0 = unconstrained)             |
| wrap           | Bool   | true       | Enable word wrapping                            |
| color          | Color  | White      | Text color                                      |
| opacity        | Scalar | 1.0        | Opacity (0.0 to 1.0)                           |

**Outputs:**

| Name   | Type   | Description                    |
|--------|--------|--------------------------------|
| text   | Text   | The text instance              |
| width  | Scalar | Measured width of the text     |
| height | Scalar | Measured height of the text    |

**Properties:**

| Name        | Description                                              |
|-------------|----------------------------------------------------------|
| Text        | Multiline text content                                   |
| Font Family | System font family name (e.g., "Arial", "Helvetica")    |
| Font Path   | Path to a .ttf or .otf font file (overrides font family) |

**Notes:** Font resolution priority: font path (if set) > font family (system lookup) > bundled Noto Sans. The text is rasterized at a zoom-aware resolution for crisp rendering at any zoom level. When box_width is 0, the text is unconstrained horizontally. Set box_width and enable wrap for paragraph-style text layout. The text instance can be further transformed by connecting it through Translate, Rotate, or Scale nodes.

```
Example patch: Text ("Hello World", font_size: 48) -> Graph Output
```

---

### Text to Path

Converts a text instance into vector path outlines of the glyphs.

**Inputs:**

| Name | Type | Default | Description         |
|------|------|---------|---------------------|
| text | Text | --      | Input text instance |

**Outputs:**

| Name | Type | Description                  |
|------|------|------------------------------|
| path | Path | Glyph outlines as a path     |

**Notes:** Extracts the outline curves of each glyph from the font and converts them to a path. This is useful for applying path operations (offset, boolean, etc.) to text, or for getting resolution-independent text that can be filled and stroked like any other path. The resulting path inherits the text's position and transform.

```
Example patch: Text ("VFS") -> Text to Path -> Set Fill (red) -> Set Stroke (black, 2) -> Graph Output
```

---

## Constants

Constant nodes output fixed values that can be edited in the properties panel. They serve as configurable parameters for your graph.

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

## Utility

### Copy to Points

Places copies of geometry at evenly-spaced points along a target path.

**Inputs:**

| Name        | Type   | Default | Hidden | Description                                      |
|-------------|--------|---------|--------|--------------------------------------------------|
| geometry    | Any    | --      | No     | Shape to copy to each point                      |
| target_path | Path   | --      | No     | Path whose sampled points receive copies         |
| count       | Int    | 10      | No     | Number of copies along the path                  |
| align       | Bool   | true    | No     | Rotate copies to align with the path tangent     |
| tolerance   | Scalar | 0.0     | Yes    | Curve flattening tolerance (0 = zoom-aware)      |

**Outputs:**

| Name           | Type   | Description                                |
|----------------|--------|--------------------------------------------|
| geometry       | Shapes | All copies merged into a batch             |
| tangent_angles | Scalars| Tangent angle in degrees at each point     |
| indices        | Scalars| Index of each copy (0 to count-1)         |
| count          | Scalar | Total number of copies                     |

**Notes:** Points are distributed by arc length, so copies are evenly spaced regardless of the path's vertex distribution. When `align` is true, each copy is rotated to follow the path's direction at that point. When tolerance is 0 (the default), zoom-aware tolerance is used automatically. Set a positive tolerance value to override with a fixed precision. The `tangent_angles` and `indices` outputs are useful for driving per-copy variations via downstream nodes.

```
Example patch: Regular Polygon (sides: 3, radius: 10) -> Set Fill -> Copy to Points (target: Circle, count: 12) -> Graph Output
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

### Merge

Combines multiple geometry inputs into a single output. Inputs are variadic -- additional ports are added automatically as you connect wires.

**Inputs:**

| Name | Type | Default | Description  |
|------|------|---------|--------------|
| a    | Any  | --      | First input  |
| b    | Any  | --      | Second input |

**Outputs:**

| Name   | Type | Description     |
|--------|------|-----------------|
| merged | Any  | Combined result |

**Properties:**

| Name          | Type | Default | Description                                              |
|---------------|------|---------|----------------------------------------------------------|
| Keep Separate | Bool | false   | Promote paths to shapes so each input stays a distinct batch element |

**Notes:** The merge behavior depends on the input types:
- **Path + Path** -- merged into a single multi-contour Path
- **Shape + Shape** -- combined into a Shapes batch
- **Paths + Path(s)** -- concatenated into a larger Paths batch
- **Shapes + Shape(s)** -- concatenated into a larger Shapes batch

Types are automatically promoted to match when possible.

When **Keep Separate** is enabled, path inputs are promoted to unstyled shapes before merging. This means multiple paths become a Shapes batch (one shape per input) rather than a single combined Path. This is useful when you want to style or transform each input independently downstream, or when feeding into Place at Points for 1:1 distribution.

```
Example patch:
  Circle -> Set Fill (red)  -> Merge
  Rectangle -> Set Fill (blue) -> Merge -> Graph Output
```

---

### Pack Points

Zips two scalar arrays into a point batch.

**Inputs:**

| Name | Type    | Default | Description               |
|------|---------|---------|---------------------------|
| xs   | Scalars | --      | X coordinates             |
| ys   | Scalars | --      | Y coordinates             |

**Outputs:**

| Name   | Type   | Description                          |
|--------|--------|--------------------------------------|
| points | Points | Combined (x, y) point batch          |

**Notes:** Pairs elements from `xs` and `ys` by index to produce a Points batch. If the two arrays differ in length, the output length is the minimum of the two. This node is useful for bridging VFS Code or Generate scalar outputs into geometry construction nodes such as Polygon from Points or Spline from Points.

```
Example patch: Generate (out_x, out_y) -> Pack Points -> Polygon from Points -> Set Stroke -> Graph Output
```

---

### Place at Points

Places each shape at the corresponding point. Shape[0] goes to point[0], shape[1] to point[1], and so on.

**Inputs:**

| Name     | Type   | Default | Description                                    |
|----------|--------|---------|------------------------------------------------|
| geometry | Any    | --      | Shapes to place (single or batch)              |
| points   | Points | --      | Target points from Grid, Scatter Points, etc.  |
| cycle    | Bool   | false   | Cycle shorter list to match longer list length  |

**Outputs:**

| Name     | Type   | Description                      |
|----------|--------|----------------------------------|
| geometry | Shapes | Shapes placed at the given points |

**Notes:** The point translation is prepended to each shape's existing transform, so any local transforms (translate, rotate, scale) applied before this node are preserved. When `cycle` is off, output length is `min(shapes, points)`. When on, both lists wrap to produce `max(shapes, points)` outputs — useful for distributing a small set of shapes across a larger grid.

A single shape input is treated as a 1-element list. With `cycle` enabled, this behaves like Copy to Points but using pre-computed points instead of sampling a path.

```
Example patch: Regular Polygon (sides: 3) -> Set Fill -> Place at Points (points: Point Grid, cycle: true) -> Graph Output
```

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

## Code

Code nodes let you write custom logic using Vector Flow Script (VFS). See the [VFS Reference](vfs-reference.md) for the full language documentation.

### Generate

Runs a VFS script for each index in a range (`start..end`), collecting results into output batches. Unlike Map, Generate does not require a batch input — it generates data from scratch based on the range.

**Inputs:**

| Name  | Type | Default | Description                |
|-------|------|---------|----------------------------|
| start | Int  | 0       | Range start (inclusive)     |
| end   | Int  | 10      | Range end (exclusive)      |

Additional inputs can be added in the properties panel to pass extra data into the script.

**Outputs:** Defined by the user in the properties panel. Each script output produces a corresponding graph output collecting all per-index results into a batch.

**Properties:**

| Name          | Description                                    |
|---------------|------------------------------------------------|
| Source        | Multiline code editor for the VFS script       |
| Script Inputs | Configure inputs available inside the script  |
| Script Outputs| Configure outputs collected into batches      |

**Notes:** The Generate node provides two built-in script variables:

| Variable  | Type | Description                              |
|-----------|------|------------------------------------------|
| `index`   | Int  | Current value in `start..end`            |
| `count`   | Int  | Total number of iterations (`end - start`) |

If `start >= end`, the node produces empty batches. User-added script inputs get graph input ports starting at port 2. All script outputs are collected into batches — you can define multiple outputs (e.g., `out_x` and `out_y`) and each produces its own batch on a separate graph output port.

A single `DslContext` is reused across iterations for efficiency. Scripts require semicolons — use explicit assignment rather than tail expressions.

**Caution:** `index` and `count` are Int. Dividing two Ints uses integer division in VFS (truncating toward zero). To get float division, promote one operand: `1.0 * index / (count - 1)`.

**Example — generate indices:**

Script inputs: `index` (Int), `count` (Int)
Script outputs: `result` (Scalar)

```
result = index;
```

**Example — generate rainbow colors:**

Script inputs: `index` (Int), `count` (Int)
Script outputs: `result` (Color)

```
let hue = 1.0 * index * 360 / count;
result = hsl(hue, 100.0, 50.0);
```

**Example — generate X/Y coordinates (multiple outputs):**

Script inputs: `index` (Int), `count` (Int)
Script outputs: `out_x` (Scalar), `out_y` (Scalar)

```
let frac = 1.0 * index / (count - 1);
out_x = sin(frac * 6.28) * 100.0;
out_y = cos(frac * 6.28) * 100.0;
```

---

### Map

Iterates over a batch of elements, running a VFS script on each one to produce transformed output batches.

**Inputs:**

| Name  | Type | Default | Description                              |
|-------|------|---------|------------------------------------------|
| batch | Any  | --      | Batch to iterate over (Scalars, Colors, etc.) |

Additional inputs can be added in the properties panel to pass extra data into the script.

**Outputs:** Defined by the user in the properties panel. Each script output produces a corresponding graph output collecting all per-element results into a batch.

**Properties:**

| Name          | Description                                    |
|---------------|------------------------------------------------|
| Source        | Multiline code editor for the VFS script       |
| Script Inputs | Configure inputs available inside the script  |
| Script Outputs| Configure outputs collected into batches      |

**Notes:** The Map node provides three built-in script variables that are automatically populated for each element:

| Variable  | Type   | Description                              |
|-----------|--------|------------------------------------------|
| `element` | (varies) | The current element from the batch     |
| `index`   | Int    | Zero-based index of the current element  |
| `count`   | Int    | Total number of elements in the batch    |

The `element` variable's type matches the element type of the input batch (e.g., Scalar for a Scalars batch, Color for a Colors batch). You can change its type in the script inputs editor.

User-added script inputs beyond the three built-ins get corresponding graph input ports (starting at port 1), allowing you to pass external values into the per-element script.

A single `DslContext` is reused across iterations for efficiency. Scripts in Map nodes require semicolons -- use explicit assignment rather than tail expressions.

**Example -- scale each value:**

Script inputs: `element` (Scalar), `index` (Int), `count` (Int)
Script outputs: `result` (Scalar)

```
result = element * 2.0;
```

**Example -- color shift per element:**

Script inputs: `element` (Color), `index` (Int), `count` (Int)
Script outputs: `result` (Color)

```
let t = index as Scalar / count as Scalar;
result = set_hue(element, t);
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

### Graph Input

Declares a typed input to the graph for external value injection.

**Inputs:** None

**Outputs:**

| Name   | Type       | Description                  |
|--------|------------|------------------------------|
| {name} | (declared) | The externally-provided value |

**Notes:** Graph Input nodes are created programmatically. They allow external code or a parent graph to feed values into the node network.

---

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

