# Implementation Plan: Design 17 Nodes

## Status: COMPLETE

## Shared Infrastructure

### ArcLengthTable (path_ops.rs)
Extract from duplicated code in `resample_path`/`resample_with_tangents`:
```rust
pub(crate) struct ArcLengthTable {
    pub segments: Vec<(Point, Point)>,
    pub cumulative_lengths: Vec<f32>,
    pub total_length: f32,
}
impl ArcLengthTable {
    pub fn from_segments(segments: &[(Point, Point)]) -> Self;
    pub fn position_at_t(&self, t: f32) -> Point;       // arc-length normalized
    pub fn tangent_at_t(&self, t: f32) -> (f32, f32);   // unit tangent
    pub fn normal_at_t(&self, t: f32) -> (f32, f32);    // perpendicular to tangent
    pub fn segment_index_at_t(&self, t: f32) -> (usize, f32); // segment + local t
}
```

### Visibility changes (path_ops.rs)
- `flatten_to_segments` → `pub(crate)`
- `flatten_to_contours` → `pub(crate)`

### New helper (cpu/mod.rs)
```rust
fn get_scalars(inputs: &ResolvedInputs, idx: usize) -> Vec<f64>
// Extracts NodeData::Scalars(vec) or wraps NodeData::Scalar(v) into vec![v]
```

---

## Node 1: Path Intersection Points

**NodeOp**: `PathIntersectionPoints` (no fields)
**Category**: PathOps
**Name**: "Path Intersection Points"
**Inputs**: a (Path), b (Path), tolerance (Scalar, default 0.5, hidden)
**Outputs**: points (Points), t_a (Scalars), t_b (Scalars), count (Int)
**Version**: 0

**Compute** (`path_ops.rs`):
```rust
pub fn path_intersection_points(a: &PathData, b: &PathData, tolerance: f32)
    -> (PointBatch, Vec<f64>, Vec<f64>)
```
Algorithm: flatten both → build ArcLengthTable for each → line-line intersection on all segment pairs → record point + arc-length t on each path → sort by t_a.

**Dispatch**: Multi-output (4 ports), early return pattern.

**Tests**: crossing lines (1 intersection), line vs square (2), disjoint (0), circle vs line.

---

## Node 2: Split Path at T

**NodeOp**: `SplitPathAtT` (no fields)
**Category**: PathOps
**Name**: "Split Path at T"
**Inputs**: path (Path), t_values (Scalars), tolerance (Scalar, default 0.5, hidden), close (Bool, default false, hidden)
**Outputs**: parts (Paths), count (Int)
**Version**: 0

**Compute** (`path_ops.rs`):
```rust
pub fn split_path_at_t(path: &PathData, t_values: &[f64], tolerance: f32, close: bool)
    -> Vec<PathData>
```
Algorithm: flatten → ArcLengthTable → sort/dedup t_values → walk segments between consecutive t positions → emit sub-paths. Open path: N cuts → N+1 parts. Closed path: N cuts → N parts.

**Dispatch**: Multi-output (2 ports), needs `get_scalars` helper.

**Tests**: split line at 0.5 (2 parts), split square at 0.25/0.5/0.75 (4 parts), close=true, empty t_values (1 part).

---

## Node 3: Close Path

**NodeOp**: `ClosePath` (no fields)
**Category**: PathOps
**Name**: "Close Path"
**Inputs**: path (Any)
**Outputs**: path (Any)
**Version**: 0

**Compute** (`path_ops.rs`):
```rust
pub fn close_path(path: &PathData) -> PathData
```
Set closed=true, append Close verb if not present.

**Dispatch**: get_any pattern, handle Path/Paths/Shape/Shapes.

**Tests**: close open path, already closed (no change), empty path.

---

## Node 4: Polygon from Points

**NodeOp**: `PolygonFromPoints` (no fields)
**Category**: PathOps
**Name**: "Polygon from Points"
**Inputs**: points (Points), close (Bool, default true)
**Outputs**: path (Path)
**Version**: 0

**Compute** (`path_ops.rs`):
```rust
pub fn polygon_from_points(points: &PointBatch, close: bool) -> PathData
```
MoveTo first, LineTo each subsequent, Close if close=true.

**Dispatch**: Simple single-output.

**Tests**: triangle, open polygon, single point, empty.

---

## Node 5: Spline from Points

**NodeOp**: `SplineFromPoints` (no fields)
**Category**: PathOps
**Name**: "Spline from Points"
**Inputs**: points (Points), close (Bool, default false), tension (Scalar, default 0.0, hidden)
**Outputs**: path (Path)
**Version**: 0

**Compute** (`path_ops.rs`):
```rust
pub fn spline_from_points(points: &PointBatch, close: bool, tension: f64) -> PathData
```
Catmull-Rom → cubic bezier:
- Tangent: T[i] = (1-tension) * (P[i+1] - P[i-1]) / 2
- Endpoints (open): T[0] = P[1]-P[0], T[n-1] = P[n-1]-P[n-2]
- Closed: wrap indices
- Bezier ctrl: ctrl1 = P[i] + T[i]/3, ctrl2 = P[i+1] - T[i+1]/3
- Emit CubicTo for each segment

**Dispatch**: Simple single-output.

**Tests**: 3 points (2 cubics), 2 points, closed 4 points, high tension.

---

## Node 6+7: Warp to Curve

**NodeOp**: `WarpToCurve` (no fields)
**Category**: Transforms
**Name**: "Warp to Curve"
**Inputs**: geometry (Any), curve (Path), mode (Int, default 0, hidden), tolerance (Scalar, default 0.5, hidden)
**Outputs**: geometry (Any)
**Version**: 0

**Compute** (`path_ops.rs`):
```rust
pub fn warp_to_curve(geometry: &NodeData, curve: &PathData, mode: i64, tolerance: f32)
    -> NodeData
```
Core: for each point, u = (x-bbox.min_x)/bbox.width → arc length, v = y-bbox.center_y → perpendicular offset. P(u) + v*N(u).

Mode 0: simple positional. Mode 1: curvature-aware (scale by 1/(1+k*v)).

Batch handling: compute collective bbox across all elements before warping.
Handle Path, Paths, Shape, Shapes. Preserve verb types (warp control points too).

**Dispatch**: get_any pattern, single output.

**Tests**: warp line onto semicircle, warp rectangle, mode 1 tight curve, empty geometry, center point → curve midpoint.

---

## Catalog Ordering (alphabetical within category)

**PathOps**: Close Path, Path Boolean, Path Intersection Points, Path Offset, Path Reverse, Path Subdivide, Polygon from Points, Resample Path, Spline from Points, Split Path at T

**Transforms**: Apply Transform, Rotate, Scale, Translate, Warp to Curve

## Files to Modify
1. `crates/vector-flow-core/src/node.rs` — 6 NodeOp variants + 6 factories
2. `crates/vector-flow-compute/src/cpu/path_ops.rs` — ArcLengthTable + 6 functions + ~25 tests
3. `crates/vector-flow-compute/src/cpu/mod.rs` — 6 dispatch arms + get_scalars helper
4. `crates/vector-flow-app/src/ui_node.rs` — 6 catalog entries + 6 node_op_label entries
5. `docs/node-reference.md` — 6 node entries
6. `docs/app-guide.md` — update category lists
