# Design 19 — Control Points & Handles Overlay

## Summary

Display path control points, curve handles, and point markers as screen-space overlays on the canvas. All annotation markers render at constant pixel size regardless of zoom, are excluded from exports, and follow selection to determine visibility.

## Motivation

When working with bezier paths, it's useful to see the underlying control points and handle lines for debugging and fine-tuning. Point Grid and Scatter Points nodes also produce point data that benefits from zoom-independent rendering. Currently, point markers are rendered as world-space geometry (circles via `points_to_marker_shapes`), which means they scale with zoom — this is inconsistent with their role as UI annotations.

## What To Draw

### Point markers (from PointBatch data)
- Small filled circles at each point position
- Constant screen-space size (e.g., 3–4px radius)
- Semi-transparent light gray (matching current marker color intent)

### On-curve points (from PathData verbs)
- The endpoint of each verb: `MoveTo(p)`, `LineTo(p)`, `QuadTo { to }`, `CubicTo { to }`
- Small filled circles, distinct color from control points (e.g., white or light blue)

### Off-curve control points
- `QuadTo { ctrl }`, `CubicTo { ctrl1, ctrl2 }`
- Small hollow squares or diamonds, distinct color (e.g., orange or yellow)

### Handle lines
- Thin lines connecting each control point to its adjacent on-curve point
- Semi-transparent, same color family as control points

## Rendering Approach: egui Painter Overlay

Draw all annotations via `ui.painter()` in screen space, after the wgpu paint callback. This was chosen over two alternatives:

- **World-space wgpu geometry** (current point marker approach): scales with zoom, appears in exports, requires tessellation — wrong for UI chrome
- **Separate wgpu overlay pass**: more complex, unnecessary for simple dots and lines

The egui approach:
- Uses the existing world-to-screen transform in `canvas_panel.rs` (same math as canvas background rect)
- Naturally excluded from exports (egui painter is not part of the wgpu scene)
- Constant screen-space size regardless of zoom
- Cheap — just iterating verbs and drawing circles/lines

### Point markers change

`collect_node_data` currently converts `NodeData::Points` → circle `Shape`s via `points_to_marker_shapes`. This will change: `CollectedScene` gains a `points: Vec<CollectedPoints>` field to carry raw point coordinates through to the overlay drawing step, instead of converting them to world-space shapes.

## Visibility Rules

The overlay follows a **selection-driven** model, consistent with how canvas preview visibility already works:

1. **Global toggle off** → no overlays drawn (regardless of selection)
2. **Global toggle on + nodes selected** → draw overlays for selected nodes' shapes only
3. **Global toggle on + nothing selected, nothing pinned** → draw for all visible shapes
4. **Pinned but not selected** → no overlays for pinned-only nodes

The key principle: **pinning = "show this output", selection = "I'm working on this."** Control points are a working/debugging tool, so they follow selection. A pinned Graph Output shows its shapes on canvas but doesn't trigger the overlay unless also selected.

### Rationale

- Pinning is typically used for the final output view — you don't want control point clutter there
- Selection already means "I'm focused on these nodes" — showing their control points is a natural extension
- This requires no new per-node state; it leverages the existing selection + pinning model

## Toggle UI

Canvas toolbar button (alongside existing Reset / Show All buttons). Possibly also a View menu entry and/or keyboard shortcut.

## Data Flow

1. `collect_scene_ordered()` collects `CollectedScene` with shapes, images, texts, and now raw points
2. App resolves which nodes are selected vs. pinned-only
3. `show_canvas_panel()` (or a new `draw_overlays()` helper) receives:
   - The collected scene's shapes and points
   - The set of selected node IDs (to filter which shapes get overlays)
   - Camera state (center, zoom, viewport) for world-to-screen transform
4. For each shape belonging to a selected node, iterate `path.verbs`, apply `shape.transform`, convert to screen coords, draw via `ui.painter()`
5. For each point batch belonging to a selected node, same transform + draw

## Future: Per-Node Override

The global toggle + selection model covers most workflows. A future enhancement could add per-node tri-state control:

- **Inherit** (default): follows the global toggle + selection rules above
- **Show**: always draw control points for this node's output when visible on canvas
- **Hide**: never draw control points for this node, even when selected with toggle on

This would live on `UiNode` (similar to `pinned`) and could be exposed in the properties panel or node context menu. Deferred until the need is confirmed through usage.

## Implementation Scope

### Render crate (`batch.rs`)
- Add `CollectedPoints` struct and `points` field to `CollectedScene`
- Change `collect_node_data` `Points` arm to push raw coordinates instead of marker shapes
- Remove (or keep as dead code) `points_to_marker_shapes` / `circle_marker_path`

### App crate (`canvas_panel.rs`)
- New `draw_overlays()` function: world-to-screen transform, egui painter calls
- Accept overlay data + camera state + which-nodes-are-selected filter

### App crate (`app.rs`)
- Add `show_control_points: bool` to app state
- Pass overlay data to canvas panel
- Canvas toolbar button for toggle
- Track which shapes belong to which nodes (so selection filtering works)

### Tests
- Unit tests for overlay point extraction from PathData verbs
- Test that points are collected as raw coordinates (not shapes)
- Test visibility rules (selected vs. pinned vs. nothing)
