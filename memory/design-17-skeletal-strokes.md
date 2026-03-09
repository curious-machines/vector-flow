# Design 17: Path Splitting & Skeletal Stroke Primitives

## Motivation

The user wants to slice shapes along cutting paths and deform the resulting strips
along skeleton curves (skeletal strokes). Rather than a monolithic skeletal stroke
node, we build composable primitives that can later be packaged into a compound
node via the planned nesting/subnet feature.

This also addresses the broader need for path-path intersection and path splitting,
which are general-purpose operations useful beyond skeletal strokes.

## Use Cases

1. **Slice a shape into strips** — cut a circle/leaf/glyph with vertical lines,
   get the individual pieces for independent manipulation.
2. **Skeletal strokes** — sweep/deform a 2D shape along a skeleton curve.
   The shape is sliced into strips, each strip mapped onto the corresponding
   segment of the skeleton.
3. **Path intersection queries** — find where two paths cross, for alignment,
   snapping, or procedural placement.

## Primitive Nodes

### 1. Path Intersection Points

Find all intersection points between two paths.

**Inputs:**
- `a` — Path (first path)
- `b` — Path (second path)
- `tolerance` — Scalar (curve flattening tolerance)

**Outputs:**
- `points` — Points (intersection positions)
- `t_a` — Scalars (parameter values along path A, normalized 0..1 by arc length)
- `t_b` — Scalars (parameter values along path B, normalized 0..1 by arc length)
- `count` — Int (number of intersections)

**Notes:**
- Both paths are flattened to polylines first; intersections are found between
  line segments, then reported as positions and arc-length parameters.
- Parameters are global (0..1 over the full path arc length).

### 2. Split Path at T

Split a path into sub-paths at given parameter values.

**Inputs:**
- `path` — Path (the path to split)
- `t_values` — Scalars (parameter values where to split, normalized 0..1 by arc length)
- `tolerance` — Scalar (curve flattening tolerance)
- `close` — Bool (whether to close each resulting sub-path, default false)

**Outputs:**
- `parts` — Paths (the sub-paths, one per segment between consecutive t values)
- `count` — Int (number of parts)

**Notes:**
- **All parameters are arc-length normalized**, not bezier t-parameters.
  A value of 0.5 means "halfway along the physical length of the path,"
  not "bezier parameter 0.5" (which may be a very different position on
  a cubic). Implementation: flatten the path, compute cumulative segment
  lengths, then binary-search for the position at each target arc length.
  Same approach used by Resample Path.
- t-values are sorted internally; duplicates are ignored.
- For a closed path split at N points, produces N parts.
  For an open path split at N points, produces N+1 parts.
- Each output sub-path carries both local arc length (0..1 within the sub-path)
  and preserves knowledge of its global arc-length range on the original path
  (via metadata or additional output ports).
- When `close` is true, each sub-path gets a Close verb appended (connecting
  its last point back to its first). Useful for making sliced pieces into
  fillable shapes.

### 3. Close Path

Simple utility to close an open path (or batch of paths).

**Inputs:**
- `path` — Any (Path or Paths)

**Outputs:**
- `path` — Any (closed version)

**Notes:**
- Sets `closed: true` and appends `PathVerb::Close` if not already present.
- Useful after Split Path to make fillable regions.

### 4. Warp to Curve

Deform geometry so it follows a target curve. Points are mapped from a
rectangular source region onto the curve.

**Inputs:**
- `geometry` — Any (geometry to warp — Path, Paths, Shape, Shapes)
- `curve` — Path (the target/skeleton curve)
- `mode` — Int (0 = simple positional, 1 = curvature-aware)
- `tolerance` — Scalar (curve flattening tolerance)

**Outputs:**
- `geometry` — Any (warped geometry)

**Mapping:**
- The source geometry's bounding box defines the rectangular source region.
- **u** = horizontal position in source bbox, mapped to arc length along the curve.
  u=0 maps to curve start, u=1 maps to curve end.
- **v** = vertical position in source bbox (centered), mapped to perpendicular
  offset from the curve at the corresponding arc length position.

**Warp modes:**

- **Simple positional (mode 0):** For each source point (x,y):
  1. Compute u = (x - bbox.min_x) / bbox.width → arc length parameter
  2. Compute v = y - bbox.center_y → perpendicular offset distance
  3. Evaluate curve position P(u) and tangent T(u) at that arc length
  4. Normal N(u) = perpendicular to T(u)
  5. Output point = P(u) + v * N(u)
  - Fast. Works well for gentle curves. Distorts at sharp bends because
    the "outside" of a bend stretches more than the "inside."

- **Curvature-aware (mode 1):** Same as above but adjusts for local curvature:
  1. At each point, compute curvature k(u) = 1/radius
  2. Scale factor = 1 / (1 + k(u) * v) — points on the outside of a bend
     are spaced farther apart, inside are compressed
  3. Adjust the arc-length mapping locally to account for this
  - More accurate for tight curves. Handles bends without overlapping.
  - More expensive: requires curvature estimation at each sample point.

### 5. Polygon from Points (separate utility, not strictly needed for skeletal strokes)

Construct a path from an ordered list of points.

**Inputs:**
- `points` — Points (vertices)
- `close` — Bool (default true)

**Outputs:**
- `path` — Path

**Notes:**
- MoveTo first point, LineTo each subsequent point, optionally Close.
- Useful for constructing arbitrary shapes from computed point positions.

## Arc Length Representation

**All path parameters throughout this pipeline are arc-length normalized, not
bezier t-parameters.** Bezier parameterization is non-uniform — equal increments
in t do not correspond to equal distances along the curve. Arc-length
normalization ensures that 0.5 always means "halfway along the physical length."

Two scopes available:

- **Global arc length** — normalized 0..1 over the full original path's arc length.
  Useful for knowing position in the overall shape. This is what Path Intersection
  Points outputs and what Split Path at T accepts.
- **Local arc length** — normalized 0..1 within each sub-path/strip after splitting.
  Useful for mapping within a single strip (e.g., for UV-style warp coordinates).

Implementation: flatten path to polylines, compute cumulative segment lengths,
binary-search to find the polyline position at a given arc-length fraction.
This is the same approach already used by Resample Path and Resample with Tangents.

## Skeletal Stroke Pipeline Example

```
Source Shape (e.g., leaf)
    │
    ├──→ Path Intersection Points ←── Vertical cutting lines
    │         │
    │         ├── points (intersection positions)
    │         └── t_a (parameters along source shape)
    │
    └──→ Split Path at T (t_values from t_a, close=true)
              │
              └── parts (individual strips)
                    │
                    └──→ Warp to Curve ←── Skeleton Path
                              │
                              └── warped strips (final skeletal stroke)
```

Alternative: instead of intersecting with explicit cutting lines, use evenly
spaced t-values (e.g., from a Generate or linspace-style node) to split directly:

```
Source Shape ──→ Split Path at T ←── evenly spaced t values (0.0, 0.1, 0.2, ...)
                      │
                      └── strips ──→ Warp to Curve ←── Skeleton
```

## Divide Operation: Open Path Cutting

The existing PathBoolean Divide operation (op=4) uses area-based boolean
composition (Difference + Intersect + InverseDifference), which requires both
inputs to be closed shapes with area.

When input `b` is an open path (line, curve), this doesn't work because lines
have no area. Options considered:

1. **Document the limitation** — Divide requires closed paths. Users use
   StrokeToPath first. Problem: stroke width creates extra regions and gaps.

2. **Half-plane trick** — Convert the open cutting path into a large closed
   polygon (extend endpoints, offset perpendicular to create a huge half-plane
   shape), then use standard boolean Intersect/Difference. Reuses i_overlay.
   Practical but somewhat hacky.

3. **Use the new primitives** — Path Intersection Points + Split Path at T
   compose into the slicing behavior naturally, and are more general than
   a special case inside Divide.

**Decision:** Option 3 preferred. The Path Intersection + Split primitives
cleanly handle the "cut shape with line" use case and compose into the larger
skeletal stroke workflow. The Divide operation on PathBoolean stays as-is for
area-based division of two closed shapes.

### 6. Spline from Points

Fit a smooth cubic bezier spline through an ordered list of points.

**Inputs:**
- `points` — Points (the points to interpolate)
- `close` — Bool (whether the spline forms a closed loop, default false)
- `tension` — Scalar (0.0 = Catmull-Rom, higher = tighter curves, default 0.0)

**Outputs:**
- `path` — Path (smooth cubic bezier path passing through all input points)

**Algorithm — Catmull-Rom to Cubic Bezier conversion:**
- For each segment between points P[i] and P[i+1], compute tangents using
  the neighboring points:
  - T[i] = (1 - tension) * (P[i+1] - P[i-1]) / 2
  - T[i+1] = (1 - tension) * (P[i+2] - P[i]) / 2
- Convert the Hermite segment (points + tangents) to cubic bezier control points:
  - ctrl1 = P[i] + T[i] / 3
  - ctrl2 = P[i+1] - T[i+1] / 3
- For open splines: endpoint tangents use forward/backward differences
  (T[0] = P[1] - P[0], T[n-1] = P[n-1] - P[n-2]).
- For closed splines: wrap indices so the tangent at the first point uses
  the last point as its predecessor.

**Notes:**
- The resulting path passes exactly through every input point (interpolating,
  not approximating).
- Catmull-Rom (tension=0) gives a natural-looking curve. Increasing tension
  pulls the curve closer to the straight-line polygon.
- This is the smooth alternative to Polygon from Points — same input, but
  the output has continuous tangents and smooth normals at every point.
- Useful for: generating smooth skeleton curves from VFS-computed waypoints,
  smooth normal fields for Warp, drawing smooth curves through scattered points.

## Implementation Order

Suggested phasing:

1. **Path Intersection Points** — core algorithm, find segment-segment intersections
   on flattened paths, compute arc-length parameters
2. **Split Path at T** — split a flattened path at arc-length parameters
3. **Close Path** — trivial utility
4. **Polygon from Points** — construct path from point list
5. **Spline from Points** — smooth cubic bezier fit through points
6. **Warp to Curve (simple mode)** — positional mapping
7. **Warp to Curve (curvature-aware mode)** — refinement

## Open Questions

- Should Split Path at T preserve original curve segments (quads/cubics) by
  splitting them at the correct parameter, or always output flattened polylines?
  Preserving curves is more precise but significantly more complex. Starting
  with flattened output is simpler and consistent with how boolean ops work.

- For the Warp node, should the source region be the bounding box (automatic)
  or user-specified (manual rect)? Bounding box is simpler but a manual rect
  gives more control over mapping. Could start with bbox and add manual option later.

- Should Warp handle batch inputs (Paths/Shapes from Split) automatically,
  warping each element using its local bounding box? This seems necessary for
  the strip pipeline to work without an explicit Map node.

## VFS → Points Bridge

To animate skeletal strokes, the backbone curve must change shape each frame.
The backbone is built from control points via Spline from Points. The problem:
Generate/VFS can output Scalar batches, but Spline from Points needs Points.

### Options considered

**A. Pack Points utility node (chosen, implemented):**
New node takes `xs: Scalars` + `ys: Scalars` → `Points`. Zero DSL changes.
Generate outputs two Scalars batches (one for x, one for y), Pack Points
zips them into a PointBatch. Simplest, immediately useful.

Pipeline: `Generate (time-varying xs, ys) → Pack Points → Spline from Points → Warp to Curve`

**B. Vec2 support in Generate (deferred):**
Wire up `DslType::Vec2` as a 2-slot type in `slots_for_dsl_type()`, add
`DataType::Vec2 → DslType::Vec2` mapping, and collect Vec2 outputs into
`NodeData::Points` via `collect_into_batch`. Then Generate with a Vec2
output directly produces Points. Moderate effort — touches `data_type_to_dsl`,
`slots_for_dsl_type`, `collect_into_batch`, and the slot read/write logic.

**C. Full Points/Path types in VFS (deferred):**
Make VFS able to manipulate variable-length point lists and paths. `DslType::Points`
and `DslType::Path` already exist in the parser/AST but are not wired up in
the runtime. Would need heap-allocated array types in the DslContext (currently
fixed-size slot model). Powerful but heavy — significant DSL runtime changes.

## Resolved Questions

- **Spline fitting** — included in scope as "Spline from Points" node using
  Catmull-Rom to cubic bezier conversion. Provides smooth normals for Warp
  and smooth skeleton curves from VFS-generated waypoints.
