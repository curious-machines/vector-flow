# Design 15 — Tolerance as a Design Parameter

## Philosophy

Tolerance (flatness) controls how finely curves are approximated by line segments. There are two distinct tolerance contexts:

1. **Render tolerance** — controls visual smoothness of curves on screen. This is now **zoom-aware** (`0.5 / zoom`), so curves stay smooth at any zoom level without user intervention.

2. **Geometric tolerance** — controls precision of operations that produce new path data (boolean ops, resampling, stroke-to-path conversion). This remains user-controllable via per-node tolerance ports because it affects the actual output geometry.

## Render Tolerance (automatic)

The render pipeline computes tolerance as `0.5 / zoom`, passed to `prepare_scene_full()` and related functions. This means:
- At zoom 1.0: tolerance = 0.5 (standard)
- At zoom 2.0: tolerance = 0.25 (finer, for zoomed-in detail)
- At zoom 0.5: tolerance = 1.0 (coarser, acceptable since zoomed out)

Dash pattern flattening at render time also uses this zoom-aware tolerance (via the render pipeline tolerance parameter).

Export uses a fixed tolerance of 0.1 for high quality output.

## Geometric Tolerance (per-node ports)

Tolerance input ports (Scalar, default `0.5`, clamped > 0) remain on:

- **Path Boolean** (v2) — controls flattening for boolean operations
- **Resample Path** (v1) — controls resampling precision
- **Copy to Points** (v1) — controls flattening during copy
- **Stroke to Path** (v1) — controls flattening for outline extraction and dash pattern application

Values <= 0 are clamped to `DEFAULT_FLATTEN_TOLERANCE` (0.5).

## Removed

- **Set Stroke** tolerance port — removed in v2. Dash flattening now uses the zoom-aware render tolerance automatically.
- **StrokeStyle.tolerance** field — removed from core types. The render crate uses the pipeline tolerance for all stroke operations.
