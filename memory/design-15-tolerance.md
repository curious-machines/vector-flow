# Design 15 — Tolerance as a Design Parameter

## Philosophy

Tolerance (flatness) controls how finely curves are approximated by line segments. While it's tempting to treat this as a behind-the-scenes rendering quality knob, **artists consider render quality a design parameter**. Visible faceting, dash placement, and stroke outline fidelity all affect the final artwork. Tolerance must be user-controllable wherever it influences the output.

## Implementation (completed)

Tolerance input ports (Scalar, default `0.5`, clamped > 0) on:

- **Path Boolean** — controls flattening for boolean operations
- **Resample Path** — controls resampling precision
- **Copy to Points** — controls flattening during copy
- **Stroke to Path** — controls flattening for outline extraction and dash pattern application
- **Set Stroke** — controls flattening for render-time dash pattern application

All five nodes are at version 1. Tolerance values <= 0 are clamped to `DEFAULT_FLATTEN_TOLERANCE` (0.5).

### StrokeStyle

`tolerance: f32` field added to `StrokeStyle` in core types. Set by both SetStroke and StrokeToPath when constructing the style.

### Render-Time Dash Consistency

When SetStroke has a dash pattern, the render crate's `tessellate_stroke()` in `batch.rs` uses `stroke.tolerance` for dash flattening instead of the pipeline tolerance. This ensures that when a user wires the same Constant Scalar into both SetStroke and StrokeToPath tolerance ports, the visual dashes and the geometric dashes match.

Non-dashed stroke tessellation still uses the pipeline tolerance since it's purely about screen fidelity.

### Consistency Between SetStroke and StrokeToPath

SetStroke and StrokeToPath are independent nodes with their own tolerance ports. When used side-by-side on the same input (as in `stroke-path.vflow`), the user is responsible for matching tolerance values. A Constant Scalar wired to both nodes is the clean pattern for this.
