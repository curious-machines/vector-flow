# Vector Flow Application Guide

This document describes the Vector Flow desktop application from a user's perspective — window layout, menus, keyboard shortcuts, mouse interactions, export workflows, and project management.

## Table of Contents

- [Window Layout](#window-layout)
- [Menu Bar](#menu-bar)
  - [File Menu](#file-menu)
  - [Arrange Menu](#arrange-menu)
- [Transport Bar](#transport-bar)
- [Node Editor Panel](#node-editor-panel)
  - [Adding Nodes](#adding-nodes)
  - [Selecting Nodes](#selecting-nodes)
  - [Connecting Ports](#connecting-ports)
  - [Node Context Menu](#node-context-menu)
  - [Pinning Nodes](#pinning-nodes)
  - [Graph Toolbar](#graph-toolbar)
- [Canvas Preview Panel](#canvas-preview-panel)
  - [Mouse Controls](#mouse-controls)
  - [Canvas Toolbar](#canvas-toolbar)
  - [What the Canvas Shows](#what-the-canvas-shows)
- [Properties Panel](#properties-panel)
  - [Standard Node Properties](#standard-node-properties)
  - [DSL Code Node Properties](#dsl-code-node-properties)
  - [Load Image Node Properties](#load-image-node-properties)
  - [Color Parse Node Properties](#color-parse-node-properties)
  - [Portal Node Properties](#portal-node-properties)
  - [Network Box Properties](#network-box-properties)
- [Network Boxes](#network-boxes)
  - [Creating a Network Box](#creating-a-network-box)
  - [Managing Members](#managing-members)
  - [Selecting and Editing](#selecting-and-editing)
  - [Deleting a Network Box](#deleting-a-network-box)
- [Exporting](#exporting)
  - [Export Canvas Image](#export-canvas-image)
  - [Export Canvas Video](#export-canvas-video)
  - [Save Graph Screenshot](#save-graph-screenshot)
- [Project Management](#project-management)
  - [File Format](#file-format)
  - [Saving and Loading](#saving-and-loading)
  - [Unsaved Changes](#unsaved-changes)
  - [Window Title](#window-title)
- [Keyboard Shortcuts](#keyboard-shortcuts)
- [Port Type Colors](#port-type-colors)

---

## Window Layout

The application window is divided into four areas:

```
┌─────────────────────────────────────────────┐
│  Menu Bar   │   Transport Bar               │
├─────────────┼──────────────────┬────────────┤
│             │                  │            │
│   Node      │     Canvas       │ Properties │
│   Editor    │     Preview      │   Panel    │
│   (left)    │     (center)     │  (right)   │
│             │                  │            │
└─────────────┴──────────────────┴────────────┘
```

- **Top panel**: Menu bar on the left, transport controls on the right.
- **Left panel**: The node graph editor. Resizable — drag the right edge to adjust width. Defaults to roughly 55% of the window.
- **Center panel**: The canvas preview, showing rendered output of your graph.
- **Right panel**: The properties inspector. Resizable — drag the left edge. Defaults to 250px wide.

---

## Menu Bar

### File Menu

| Menu Item | Shortcut | Description |
|-----------|----------|-------------|
| New | Ctrl+N | Create a new empty project. Prompts to save if there are unsaved changes. |
| Open... | Ctrl+O | Open an existing `.vflow` project file. Prompts to save if there are unsaved changes. |
| Close | Ctrl+W | Close the current project and return to an empty state. Prompts to save if there are unsaved changes. |
| Save | Ctrl+S | Save the current project. If the project has not been saved before, opens a Save As dialog. |
| Save As... | Ctrl+Shift+S | Save the current project to a new file location. |
| Export Canvas Image... | Ctrl+Shift+E | Open the [image export dialog](#export-canvas-image) to render the canvas to a PNG at a chosen resolution. |
| Export Canvas Video... | — | Open the [video export dialog](#export-canvas-video) to render a frame range to a PNG sequence or MP4. |
| Save Graph Screenshot... | — | Capture the node editor panel as a PNG image. Opens a file save dialog, then captures the screen on the next frame. |
| Quit | Ctrl+Q | Close the application. Prompts to save if there are unsaved changes. |

### Arrange Menu

These operations apply to the currently selected nodes in the graph editor.

| Menu Item | Description |
|-----------|-------------|
| Align Left | Align selected nodes to the leftmost node's left edge. |
| Align Right | Align selected nodes to the rightmost node's right edge. |
| Align Top | Align selected nodes to the topmost node's top edge. |
| Align Bottom | Align selected nodes to the bottommost node's bottom edge. |
| Align Centers Horizontally | Align selected nodes to share the same horizontal center. |
| Align Centers Vertically | Align selected nodes to share the same vertical center. |
| Distribute Horizontally | Space selected nodes evenly in the horizontal direction. |
| Distribute Vertically | Space selected nodes evenly in the vertical direction. |

---

## Transport Bar

The transport bar sits in the top panel to the right of the menu bar. It controls animation playback.

| Control | Description |
|---------|-------------|
| **\|<** (Rewind) | Stop playback and reset to frame 0. |
| **>** (Play) | Start playing forward. The frame counter advances each tick based on FPS. Shown when paused. |
| **\|\|** (Pause) | Pause playback at the current frame. Shown when playing. |
| **>\|** (Step) | Advance by one frame and pause. |
| **Frame / Time / FPS display** | Shows the current frame number, elapsed time in seconds, and the configured frames per second. |

When playback is active, the graph re-evaluates each frame and the canvas updates in real time. This is how you preview animations driven by the built-in `frame` and `time` global variables.

---

## Node Editor Panel

The node editor is the main workspace for building your processing graph. Nodes are visual blocks with input and output ports that you connect with wires.

### Adding Nodes

Right-click on the graph background to open the **Add Node** context menu. Nodes are organized into categories:

- **Generators** — Circle, Rectangle, Regular Polygon, Line, Point Grid, Scatter Points, Load Image
- **Transforms** — Translate, Rotate, Scale, Apply Transform
- **Path Ops** — Path Union, Path Intersect, Path Difference, Path Offset, Path Subdivide, Path Reverse, Resample Path
- **Styling** — Set Fill, Set Stroke
- **Color** — Adjust Hue, Adjust Saturation, Adjust Lightness, Adjust Luminance, Invert Color, Grayscale, Mix Colors, Set Alpha, Color Parse
- **Utility** — Constant Scalar, Constant Int, Constant Vec2, Constant Color, Portal Send, Portal Receive, Merge, Duplicate, DSL Code
- **Graph I/O** — Graph Output

A new node is placed at your cursor position.

### Selecting Nodes

- **Click** a node to select it.
- **Click and drag** on the background to box-select multiple nodes.
- The properties panel on the right updates to show the selected node's parameters.

### Connecting Ports

- **Drag** from an output pin to an input pin (or vice versa) to create a connection.
- Each input can only have one incoming connection. Connecting to an already-connected input replaces the old connection.
- **Variadic inputs** (on nodes like Merge): automatically expand when all inputs are connected, and shrink when edges are removed.

### Node Context Menu

Right-click on a node to see:

| Item | Description |
|------|-------------|
| Pin / Unpin | Toggle whether this node's output is always shown in the canvas preview, regardless of selection. Pinned nodes show a colored diamond indicator in their header. |
| Duplicate  Ctrl+D | Create a copy of the node at a slight offset. Works on multiple selected nodes too. |
| Create Network Box | Create a new [network box](#network-boxes) containing this node (or all selected nodes if multiple are selected). |
| Add to Network Box | Add this node to an existing network box. Shows a submenu listing all boxes. |
| Remove from Network Box | Remove this node from whatever network box it belongs to. Only shown if the node is currently in a box. |
| Delete | Remove this node and all its connections from the graph. |

### Pinning Nodes

By default, the canvas shows output from all nodes when nothing is selected, or only the selected nodes when you have a selection. **Pinning** a node makes its output always visible in the canvas, even when other nodes are selected or nothing is selected.

- Right-click a node and choose **Pin** to pin it.
- Pinned nodes display a colored diamond icon in their title bar.
- Right-click and choose **Unpin** to remove the pin.

### Graph Toolbar

A small **Show All** button is overlaid in the top-left corner of the node editor. Clicking it (or pressing **F**) fits all nodes into view by adjusting the editor's pan and zoom.

---

## Canvas Preview Panel

The canvas shows a live preview of your graph's visual output — rendered shapes, paths, and images.

### Mouse Controls

| Input | Action |
|-------|--------|
| Middle mouse button drag | Pan the camera |
| Scroll wheel (while hovering) | Zoom in/out, centered on the cursor |

### Canvas Toolbar

Two buttons are overlaid in the top-left corner of the canvas:

| Button | Description |
|--------|-------------|
| Reset | Reset the camera to zoom level 1.0 and center on the origin (0, 0). |
| Show All | Fit the camera so all rendered content is visible, with a small margin. |

### What the Canvas Shows

- **Nothing selected, no pinned nodes**: All node outputs are shown.
- **Nodes selected**: Only the selected nodes' outputs are shown.
- **Pinned nodes exist**: Pinned nodes' outputs are always shown, combined with any selected nodes' outputs.

This lets you isolate individual nodes to see their contribution, or pin important nodes so they always remain visible.

---

## Properties Panel

The right-side panel shows editable parameters for the current selection.

### Standard Node Properties

When a single node is selected, the panel shows:

- **Node type** as a heading (e.g., "Circle", "Translate").
- **Parameter editors** for each input port that has a default value:
  - **Scalar**: A drag-value field (drag left/right to adjust).
  - **Int**: A drag-value field with integer steps.
  - **Bool**: A checkbox.
  - **Vec2**: Two drag-value fields labeled X and Y.
  - **Color**: A color picker button showing the current color.
- **Variadic input controls** (for Merge and similar nodes):
  - Shows the current input count.
  - **+** button to add another input.
  - **−** button to remove the last input (minimum of 2).

Parameters that have an incoming connection are driven by that connection and cannot be edited manually — the connected value takes priority.

When multiple nodes are selected, the panel shows "{N} nodes selected" with no editable fields.

When nothing is selected, the panel shows "No selection".

### DSL Code Node Properties

The DSL Code node has a special editor:

- **Expression** heading with a multiline text area for your script source code.
- A hint: "e.g. sin(time * 3.14)".
- If the script fails to compile, the error is shown in red below the editor.
- **Inputs** section: Add, remove, rename, and change the type (Scalar or Int) of input ports.
- **Outputs** section: Same controls for output ports.

See the [DSL Reference](dsl-reference.md) for the scripting language.

### Load Image Node Properties

In addition to standard parameters (position, width, height, opacity), shows:

- **Path**: A text field for the image file path (relative to the project file, or absolute).
- A folder icon button that opens a file picker supporting PNG, JPG, GIF, WebP, and BMP formats.

Width and height of 0 mean "use the native image dimensions". They auto-populate after the image first loads.

### Color Parse Node Properties

Shows a text field labeled **Color** where you can type:

- A hex color value (e.g., `#ff0000`, `#f00`).
- A CSS named color (e.g., `red`, `cornflowerblue`). About 148 named colors are supported.

### Portal Node Properties

Portal Send and Portal Receive nodes show a **Label** text field. Portals with matching labels are connected invisibly — data sent through a Portal Send appears at every Portal Receive with the same label, without a visible wire.

### Network Box Properties

When a network box is selected (by clicking its title bar), the panel shows:

- **Title**: Editable text field for the box name.
- **Fill**: RGBA color picker for the box background.
- **Stroke**: RGBA color picker for the box border.
- **Stroke Width**: Drag-value field (0.0 to 10.0) for border thickness.

---

## Network Boxes

Network boxes are visual grouping annotations — colored rectangles that surround a set of nodes to organize your graph. They are purely visual and do not affect evaluation.

### Creating a Network Box

1. Select one or more nodes.
2. Right-click on one of the selected nodes.
3. Choose **Create Network Box**.

A new box is created containing all selected nodes. The box auto-resizes to fit its members with padding.

### Managing Members

- **Add a node**: Right-click a node → **Add to Network Box** → choose the box.
- **Remove a node**: Right-click a node that is in a box → **Remove from Network Box**.
- Each node can belong to at most one network box.
- Member nodes have their border tinted with the box's fill color.

### Selecting and Editing

- **Click** a box's title bar to select it. This clears any node selection.
- **Drag** a box's title bar to move all member nodes together.
- Edit the box's title, colors, and stroke width in the [properties panel](#network-box-properties).

### Deleting a Network Box

- Right-click the graph background → **Delete Network Box** → choose the box.
- Or right-click the box's title bar → **Delete Network Box**.

Deleting a box removes the grouping only — the member nodes are not deleted.

---

## Exporting

Vector Flow offers three ways to export your work.

### Export Canvas Image

**Menu**: File → Export Canvas Image...
**Shortcut**: Ctrl+Shift+E

Opens a dialog to render the canvas to a PNG file at any resolution.

| Field | Description |
|-------|-------------|
| Width | Output width in pixels (1–8192). Default: 1920. |
| Height | Output height in pixels (1–8192). Default: 1080. |
| Camera | **Current View** uses the same center and zoom as the canvas preview. **Fit to Content** automatically frames all content. |
| Output | The destination file path. Click **Browse...** to choose a location. Default: `export.png`. |

Click **Export** to render. A success or error message appears in the dialog. The export uses an independent offscreen renderer, so it does not interfere with the live canvas preview.

### Export Canvas Video

**Menu**: File → Export Canvas Video...

Opens a dialog to render a range of frames to either a numbered PNG sequence or an MP4 video file.

| Field | Description |
|-------|-------------|
| Width | Output width in pixels (1–8192). Default: 1920. |
| Height | Output height in pixels (1–8192). Default: 1080. |
| Camera | **Current View** or **Fit to Content**, same as image export. |
| Format | **PNG Sequence** outputs numbered files (`frame_000000.png`, `frame_000001.png`, ...) to a folder. **MP4 (ffmpeg)** pipes frames to ffmpeg to produce an MP4 video file. |
| Start Frame | First frame to render. Default: 0. |
| End Frame | Last frame to render (inclusive). Default: 100. |
| FPS | Frames per second, read from the transport bar (display only). |
| Duration | Computed total frames and duration in seconds (display only). |
| Output | Destination folder (PNG sequence) or file path (MP4). Click **Browse...** to choose. |

Click **Export** to begin rendering. A progress bar shows the current frame out of the total. During export, the dialog controls are disabled. Click **Cancel** to stop early.

**MP4 export** requires [ffmpeg](https://ffmpeg.org/) to be installed and available on your system PATH. If ffmpeg is not found, an error message is shown in the dialog.

The export works by stepping through each frame in the range, evaluating the graph at that frame's time, and rendering the result. This means animation-driven content (using `frame`, `time`, etc.) will produce proper animated output.

### Save Graph Screenshot

**Menu**: File → Save Graph Screenshot...

Captures the node editor panel as it currently appears on screen and saves it as a PNG. This is useful for documentation or sharing your graph layout.

1. Choose a save location in the file dialog.
2. The screenshot is captured on the next frame.
3. The image is automatically cropped to the node editor panel boundaries.

The screenshot captures everything visible in the graph editor — nodes, connections, network boxes, and the background grid — at screen resolution.

---

## Project Management

### File Format

Vector Flow projects are saved as `.vflow` files. The file contains:

- The complete node graph (all nodes, connections, and parameters).
- Node positions and the graph editor's UI layout.
- View state: graph editor pan/zoom and canvas camera center/zoom.

A companion `.vflow.meta` sidecar file is saved alongside the project to store window geometry (position, size, panel widths). This lets the window restore to the same layout when you reopen the project.

### Saving and Loading

- **Save** (Ctrl+S): Writes to the current file. If no file has been chosen, acts like Save As.
- **Save As** (Ctrl+Shift+S): Always opens a file dialog.
- **Open** (Ctrl+O): Opens a file dialog filtered to `.vflow` files.
- **New** (Ctrl+N): Resets to a blank project.
- **Close** (Ctrl+W): Returns to an empty state without quitting the application.

Image paths in LoadImage nodes are resolved relative to the project file's directory. Save your project before loading images with relative paths.

### Unsaved Changes

When you have unsaved changes and attempt to create a new project, open a different file, close the current file, or quit the application, a dialog appears:

> "You have unsaved changes. What would you like to do?"
>
> **Save** | **Discard** | **Cancel**

- **Save**: Saves the project, then proceeds with the action.
- **Discard**: Proceeds without saving.
- **Cancel**: Cancels the action and returns to editing.

### Window Title

The window title shows the current file name and dirty state:

- `Untitled — Vector Flow` for a new unsaved project.
- `myproject.vflow — Vector Flow` for a saved project.
- `myproject.vflow* — Vector Flow` when there are unsaved changes (note the asterisk).

---

## Keyboard Shortcuts

### File Operations

| Shortcut | Action |
|----------|--------|
| Ctrl+N | New project |
| Ctrl+O | Open project |
| Ctrl+W | Close current file |
| Ctrl+S | Save |
| Ctrl+Shift+S | Save As |
| Ctrl+Shift+E | Export Canvas Image |
| Ctrl+Q | Quit |

### Node Editing

| Shortcut | Action |
|----------|--------|
| Ctrl+D | Duplicate selected nodes |
| Delete | Delete selected nodes (via snarl built-in) |
| Arrow keys | Nudge selected nodes by 1 pixel |
| Shift+Arrow keys | Nudge selected nodes by 10 pixels |

### View

| Shortcut | Action |
|----------|--------|
| F | Fit all nodes in view (graph editor) |

Arrow key and F shortcuts are disabled when a text field has focus, so they do not interfere with typing.

---

## Port Type Colors

Each data type has a distinct color on its port pins and connection wires:

| Type | Color |
|------|-------|
| Scalar | Green |
| Int | Teal |
| Bool | Light gray |
| Vec2 | Blue |
| Color | Magenta |
| Path | Orange-yellow |
| Shape | Red |
| Transform | Light blue |
| Image | Purple |
| Points | Cyan |
| Paths | Orange |
| Shapes | Dark red |
| Scalars | Light green |
| Colors | Bright magenta |
| Ints | Medium teal |
| Any | Gray |

These colors help you quickly identify compatible connections when wiring nodes together.
