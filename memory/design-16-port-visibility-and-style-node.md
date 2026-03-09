# Design 16: Port Visibility & SetStyle Node

## Port Visibility System

### Overview

Ports can be shown or hidden on nodes in the graph. Visibility is purely a UI concern — hidden ports still participate in evaluation and retain their values. The property sheet always shows all ports regardless of visibility.

### Port Definition

Each port has a formal definition including name, data type, and `visible_by_default` flag. Both input and output ports use the same visibility system. Outputs will almost always default to visible, but the framework treats them uniformly.

Port indices are stable — every port has a fixed index in the definition list regardless of visibility. Connections and evaluation reference ports by index.

### Per-Instance State

Visibility is a property on each port instance (not a parallel data structure). When a node is created, visibility is initialized from the port definitions' `visible_by_default` flags. After creation, the user can show or hide any port freely. This per-instance state is serialized with the project file.

### Default Visibility Layers

Defaults are resolved in priority order (highest wins):

1. **Built-in defaults** — hardcoded in the node type's port definitions
2. **Application defaults** (future) — user preferences across all projects
3. **Project defaults** (future) — per-project overrides, saved in the project file
4. **Instance state** — per-node visibility, starts from the applicable default

When creating a new node, walk the chain: project defaults > application defaults > built-in defaults. Once the instance exists, its state is independent.

This layered settings pattern applies beyond port visibility to any configurable behavior (default values, color schemes, grid snapping, etc.).

### Rules

- **Must be visible to connect**: a port must be visible on the node to accept or initiate a connection. To connect to a hidden port, first show it via the property sheet.
- **Can't hide connected ports**: the visibility toggle is disabled for ports with active connections. Disconnect first, then hide.
- **No auto-hide on disconnect**: visibility stays as the user set it. Users manage visibility explicitly.
- **Hidden ports retain values**: hiding a port does not reset or disconnect anything. The port's literal/expression value continues to be used in evaluation.

### Interaction

- **Property sheet** is the primary (and initially only) interface for toggling visibility. It always shows all ports for the selected node. Each port row has a visibility toggle (e.g., pin/eye icon) indicating whether that port appears on the node in the graph.
- **All ports are editable** in the property sheet regardless of visibility. The property sheet is the complete interface for a node; visible ports on the node are the subset exposed for graph wiring.
- **Future shortcuts** (context menus, on-node toggles, etc.) would be convenience wrappers around the property sheet toggle.

### Serialization

Port visibility is a property on the port instance, serialized with the node in the project file. On load, ports not present in the saved data (e.g., from a newer node version with added ports) fall back to `visible_by_default` from the port definition.

## SetStyle Node

### Overview

A combined fill + stroke styling node that reduces graph clutter for the common case of applying both fill and stroke to a shape. Does not replace the separate SetFill and SetStroke nodes.

### Ports

**Input ports:**

| Index | Name | Type | Visible by Default |
|-------|------|------|--------------------|
| 0 | path | Any | yes |
| 1 | fill_color | Color | yes |
| 2 | fill_opacity | Scalar | no |
| 3 | has_fill | Bool | no |
| 4 | stroke_color | Color | yes |
| 5 | stroke_width | Scalar | yes |
| 6 | stroke_opacity | Scalar | no |
| 7 | has_stroke | Bool | no |
| 8 | cap | Int | no |
| 9 | join | Int | no |
| 10 | miter_limit | Scalar | no |
| 11 | dash_pattern | String | no |
| 12 | dash_offset | Scalar | no |

- `has_fill` and `has_stroke` default to true. Setting to false skips that styling pass.
- For experimentation, toggle fill/stroke off in the property sheet. For permanent single-style use, prefer the dedicated SetFill or SetStroke nodes.

**Output ports:**

| Index | Name | Type | Visible by Default |
|-------|------|------|--------------------|
| 0 | output | Any | yes |

### Consistency

SetFill, SetStroke, and StrokeToPath should adopt the same port visibility treatment — advanced ports (cap, join, dash, miter) hidden by default, common ones visible.

### Promotion and Demotion

Nodes can be promoted/demoted between individual and combined styling:

**Promotion:**
- SetFill → SetStyle: fill values carry over, stroke gets defaults
- SetStroke → SetStyle: stroke values carry over, fill gets defaults
- Chained SetFill + SetStroke → SetStyle: both value sets merge, intermediate connection removed, upstream/downstream rewired

**Demotion:**
- SetStyle → SetFill: only fill values preserved
- SetStyle → SetStroke: only stroke values preserved
- SetStyle → SetFill + SetStroke chain: both preserved, auto-wired in sequence

Demotion that discards non-default values should show a transient status bar message (e.g., "Demoted to SetStroke — fill settings discarded") that clears on the next user interaction. No dialog — undo is always available.

Promotion/demotion is deferred to a follow-up after the initial implementation.

### Status Bar (future)

A transient message bar for non-critical notifications (demotion info, export progress, errors). Clears on next user interaction. Not yet implemented — noted here as a dependency for promotion/demotion messaging.

## Implementation Order

1. Formalize port definitions (PortDef struct with name, type, visible_by_default)
2. Add per-instance port visibility state
3. Update property sheet to show all ports with visibility toggle
4. Update node rendering to respect visibility
5. Implement SetStyle node using the new system
6. Apply visibility defaults to existing styling nodes (SetFill, SetStroke, StrokeToPath)
7. (Future) Application-level default overrides
8. (Future) Project-level default overrides
9. (Future) Promotion/demotion between styling nodes
10. (Future) Status bar for transient messages
