use std::path::{Path, PathBuf};

use egui_snarl::Snarl;
use serde::{Deserialize, Serialize};

use vector_flow_core::graph::Graph;

use crate::id_map::IdMap;
use crate::ui_node::UiNode;

// ---------------------------------------------------------------------------
// Window geometry (stored inline in .vflow project file)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub node_editor_width: Option<f32>,
    #[serde(default)]
    pub properties_width: Option<f32>,
}

const MIN_WINDOW_SIZE: f32 = 200.0;
const MARGIN: f32 = 50.0;

impl WindowGeometry {
    /// Clamp the geometry so the window is visible on a screen of the given size.
    pub fn clamp_to_screen(&mut self, screen_w: f32, screen_h: f32) {
        // Clamp size to fit within screen bounds.
        self.width = self.width.clamp(MIN_WINDOW_SIZE, screen_w);
        self.height = self.height.clamp(MIN_WINDOW_SIZE, screen_h);

        // Ensure at least MARGIN pixels of the window are on-screen.
        self.x = self.x.clamp(MARGIN - self.width, screen_w - MARGIN);
        self.y = self.y.clamp(0.0, screen_h - MARGIN);
    }
}

/// Saved view state for graph editor and canvas camera.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ViewState {
    /// Graph editor offset (snarl viewport center in graph space).
    #[serde(default)]
    pub graph_offset: [f32; 2],
    /// Graph editor zoom scale.
    #[serde(default = "default_scale")]
    pub graph_scale: f32,
    /// Canvas camera center (world coordinates).
    #[serde(default)]
    pub canvas_center: [f32; 2],
    /// Canvas camera zoom level.
    #[serde(default = "default_scale")]
    pub canvas_zoom: f32,
}

fn default_scale() -> f32 {
    1.0
}

fn default_canvas_width() -> u32 {
    640
}

fn default_canvas_height() -> u32 {
    480
}

fn default_fps() -> f32 {
    30.0
}

/// Project-level settings: canvas dimensions and background color.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    #[serde(default = "default_canvas_width")]
    pub canvas_width: u32,
    #[serde(default = "default_canvas_height")]
    pub canvas_height: u32,
    /// Background color as \[r, g, b, a\] in 0..1 range. `None` = transparent.
    #[serde(default)]
    pub background_color: Option<[f32; 4]>,
    #[serde(default = "default_fps")]
    pub fps: f32,
}

impl ProjectSettings {
    /// Approximate equality for dirty tracking.
    pub fn approx_eq(&self, other: &Self) -> bool {
        self.canvas_width == other.canvas_width
            && self.canvas_height == other.canvas_height
            && self.background_color == other.background_color
            && (self.fps - other.fps).abs() < 0.01
    }
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            canvas_width: 640,
            canvas_height: 480,
            background_color: Some([40.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0, 1.0]),
            fps: 30.0,
        }
    }
}

impl ViewState {
    /// Approximate equality check for dirty tracking (tolerates small float drift).
    pub fn approx_eq(&self, other: &Self) -> bool {
        const EPS: f32 = 0.5;
        const SCALE_EPS: f32 = 0.001;
        (self.graph_offset[0] - other.graph_offset[0]).abs() < EPS
            && (self.graph_offset[1] - other.graph_offset[1]).abs() < EPS
            && (self.graph_scale - other.graph_scale).abs() < SCALE_EPS
            && (self.canvas_center[0] - other.canvas_center[0]).abs() < EPS
            && (self.canvas_center[1] - other.canvas_center[1]).abs() < EPS
            && (self.canvas_zoom - other.canvas_zoom).abs() < SCALE_EPS
    }
}

#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    pub graph: Graph,
    pub snarl: Snarl<UiNode>,
    #[serde(default)]
    pub view_state: Option<ViewState>,
    #[serde(default)]
    pub window_geometry: Option<WindowGeometry>,
    #[serde(default)]
    pub settings: ProjectSettings,
}

impl ProjectFile {
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut project: Self = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        project.graph.rebuild_caches();
        Ok(project)
    }
}

/// Rebuild an IdMap from the current snarl nodes.
pub fn rebuild_id_map(snarl: &Snarl<UiNode>) -> IdMap {
    let mut id_map = IdMap::new();
    for (snarl_id, node) in snarl.node_ids() {
        id_map.insert(node.core_id, snarl_id);
    }
    id_map
}

/// Show a save-file dialog and return the chosen path.
/// Ensures the `.vflow` extension is present.
/// If `current` is provided, the dialog starts in that file's directory with its name.
pub fn save_dialog(current: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save Project")
        .add_filter("Vector Flow Project", &["vflow"]);

    if let Some(cur) = current {
        if let Some(dir) = cur.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = cur.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
    } else {
        dialog = dialog.set_file_name("untitled.vflow");
    }

    let mut path = dialog.save_file()?;
    if path.extension().is_none() {
        path.set_extension("vflow");
    }
    Some(path)
}

/// Show an open-file dialog and return the chosen path.
pub fn open_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open Project")
        .add_filter("Vector Flow Project", &["vflow"])
        .add_filter("All Files", &["*"])
        .pick_file()
}
