# Design 15 — Tolerance as a Design Parameter

## Philosophy

Tolerance (flatness) controls how finely curves are approximated by line segments. All tolerance is now **zoom-aware by default** — both rendering and geometric operations adapt to the current zoom level automatically.

## Zoom-Aware Tolerance (default for all nodes)

The app computes tolerance as `0.5 / zoom`, stored in `EvalContext.tolerance`. This means:
- At zoom 1.0: tolerance = 0.5 (standard)
- At zoom 2.0: tolerance = 0.25 (finer, for zoomed-in detail)
- At zoom 0.5: tolerance = 1.0 (coarser, acceptable since zoomed out)

When a node's tolerance port is **0** (the default), it uses `EvalContext.tolerance` — the zoom-aware value. Users can set a **positive** tolerance value to override with fixed precision.

### Render tolerance

The render pipeline computes tolerance as `0.5 / zoom`, passed to `prepare_scene_full()` and related functions. Dash pattern flattening at render time also uses this zoom-aware tolerance. Export uses a fixed tolerance of 0.1 for high quality output.

### Geometric tolerance (per-node ports)

All nodes with tolerance ports default to 0 (zoom-aware). Hidden by default, but can be shown and overridden:

- **Path Boolean** (v3) — flattening for boolean operations
- **Path Offset** (v2) — flattening for offset computation
- **Path Intersection Points** (v1) — flattening for intersection detection
- **Split Path at T** (v1) — flattening for arc-length splitting
- **Resample Path** (v2) — resampling precision
- **Copy to Points** (v2) — flattening during copy
- **Warp to Curve** (v1) — flattening for warp mapping
- **Stroke to Path** (v2) — outline extraction and dash pattern application
- **Set Stroke** (v3) — dash/stroke rendering
- **Set Style** (v2) — combined fill/stroke styling

Values <= 0 fall through to `EvalContext.tolerance`. Values > 0 are used directly (clamped to `MIN_TOLERANCE` = 0.001 by the underlying functions).
