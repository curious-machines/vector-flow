use std::path::{Path, PathBuf};

use egui_snarl::Snarl;
use serde::{Deserialize, Serialize};

use vector_flow_core::graph::Graph;

use crate::id_map::IdMap;
use crate::ui_node::UiNode;

// ---------------------------------------------------------------------------
// Window geometry (stored as .vflow.meta sidecar)
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

fn meta_path(project_path: &Path) -> PathBuf {
    let mut p = project_path.to_owned();
    let mut ext = p
        .extension()
        .unwrap_or_default()
        .to_os_string();
    ext.push(".meta");
    p.set_extension(ext);
    p
}

pub fn save_window_geometry(project_path: &Path, geom: &WindowGeometry) {
    let path = meta_path(project_path);
    match serde_json::to_string_pretty(geom) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("Failed to save window geometry: {e}");
            }
        }
        Err(e) => log::warn!("Failed to serialize window geometry: {e}"),
    }
}

pub fn load_window_geometry(project_path: &Path) -> Option<WindowGeometry> {
    let path = meta_path(project_path);
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    pub graph: Graph,
    pub snarl: Snarl<UiNode>,
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
