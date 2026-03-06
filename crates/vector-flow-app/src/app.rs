use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use egui_snarl::ui::SnarlStyle;
use egui_snarl::{NodeId as SnarlNodeId, Snarl};
use glam::Vec2;

use vector_flow_core::graph::Graph;
use vector_flow_core::node::{NodeOp, ParamValue};
use vector_flow_core::scheduler::{EvalResult, Scheduler};
use vector_flow_core::types::{NetworkBoxId, NodeData, NodeId as CoreNodeId};
use vector_flow_compute::CpuBackend;
use vector_flow_render::overlay::CanvasRenderResources;
use vector_flow_render::renderer::CanvasRenderer;
use vector_flow_render::camera::Camera;
use vector_flow_render::{collect_scene, prepare_scene_full, PreparedScene};

use crate::canvas_panel::{self, CameraState};
use crate::export::{self, ExportState, VideoExportConfig};
use crate::export_dialog::{self, ImageExportDialog, VideoExportDialog};
use crate::id_map::IdMap;
use crate::project::{self, ProjectFile, ViewState, WindowGeometry};
use crate::properties_panel;
use crate::transport_panel::{self, TransportState};
use crate::ui_node::{node_catalog, CatalogEntry, UiNode};
use crate::viewer::{GraphViewer, ViewerActions};

const NODE_EDITOR_ID: &str = "node_editor";

#[derive(Clone, Copy)]
enum AlignMode {
    Left,
    Right,
    Top,
    Bottom,
    CenterH,
    CenterV,
}

/// Tracks what action triggered the "unsaved changes" dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    Close,
    Open,
    New,
    CloseFile,
}

pub struct VectorFlowApp {
    graph: Graph,
    snarl: Snarl<UiNode>,
    id_map: IdMap,
    catalog: Vec<CatalogEntry>,
    snarl_style: SnarlStyle,

    scheduler: Scheduler,

    transport: TransportState,
    cam_state: CameraState,

    /// Cached eval result from last graph evaluation.
    last_eval: Option<EvalResult>,

    /// Per-node errors from last evaluation (e.g. DSL compile errors).
    node_errors: HashMap<CoreNodeId, String>,

    /// Last graph generation we evaluated at.
    last_eval_gen: u64,

    /// Last frame number we evaluated at.
    last_eval_frame: u64,

    /// Prepared scene for the canvas.
    prepared_scene: Option<PreparedScene>,

    /// Current project file path (None if unsaved).
    project_path: Option<PathBuf>,

    /// Graph generation at the last save/load (or initial empty state).
    saved_gen: u64,

    /// Panel widths at last save/load, for dirty tracking.
    saved_panel_widths: Option<[f32; 2]>,

    /// View state snapshot at last save/load, for dirty tracking.
    saved_view_state: Option<ViewState>,

    /// Hash of node positions at last save/load, for layout dirty tracking.
    saved_node_pos_hash: u64,

    /// Window position/size at last save/load, for dirty tracking.
    saved_window_geom: Option<[f32; 4]>,

    /// Frames remaining before re-snapshotting saved state after load.
    /// Needed because viewport commands (window resize/move) apply asynchronously.
    pending_snapshot_frames: u8,

    /// The egui UI Id for the node editor panel (needed to access snarl view state).
    node_editor_ui_id: egui::Id,

    /// Pending view state to restore after the next frame establishes UI ids.
    pending_view_restore: Option<ViewState>,

    /// Pending canvas camera restore (center, zoom) — applied when render_state is available.
    pending_canvas_restore: Option<(Vec2, f32)>,

    /// When set, the unsaved-changes dialog is shown for this action.
    pending_action: Option<PendingAction>,

    /// Set to true once the user confirms they want to close despite unsaved changes.
    close_confirmed: bool,

    /// Cached node rects (in graph coordinates) from the last frame's `snarl.show()`.
    node_rects: HashMap<SnarlNodeId, egui::Rect>,

    /// Currently selected network box (if any). Cleared when nodes are selected.
    selected_box: Option<NetworkBoxId>,

    // ── Export state ──────────────────────────────────────────────────
    export_state: ExportState,
    image_export_dialog: ImageExportDialog,
    video_export_dialog: VideoExportDialog,

    /// When true, a graph screenshot has been requested. We send the viewport
    /// command this frame and check for the result next frame.
    pending_graph_screenshot: bool,
    /// Path to save the graph screenshot to.
    graph_screenshot_path: Option<PathBuf>,
    /// Rect of the node editor panel in screen pixels (for cropping screenshots).
    graph_panel_rect: egui::Rect,
}

impl VectorFlowApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Initialize compute backend.
        let backend = CpuBackend::new().expect("Failed to create CPU backend");
        let scheduler = Scheduler::new(Arc::new(backend));

        // Initialize wgpu render resources.
        if let Some(render_state) = &cc.wgpu_render_state {
            let renderer = CanvasRenderer::new(
                &render_state.device,
                render_state.target_format,
            );
            let camera = Camera::new(Vec2::new(800.0, 600.0));

            render_state
                .renderer
                .write()
                .callback_resources
                .insert(CanvasRenderResources { renderer, camera });
        }

        let snarl = Snarl::new();
        let saved_node_pos_hash = Self::node_position_hash(&snarl);

        Self {
            graph: Graph::new(),
            snarl,
            id_map: IdMap::new(),
            catalog: node_catalog(),
            snarl_style: SnarlStyle::default(),
            scheduler,
            transport: TransportState::default(),
            cam_state: CameraState::default(),
            last_eval: None,
            node_errors: HashMap::new(),
            last_eval_gen: u64::MAX,
            last_eval_frame: u64::MAX,
            prepared_scene: None,
            project_path: None,
            saved_gen: 0,
            saved_panel_widths: None,
            saved_view_state: None,
            saved_node_pos_hash,
            saved_window_geom: None,
            pending_snapshot_frames: 0,
            node_editor_ui_id: egui::Id::NULL,
            pending_view_restore: None,
            pending_canvas_restore: None,
            export_state: ExportState::default(),
            image_export_dialog: ImageExportDialog::default(),
            video_export_dialog: VideoExportDialog::default(),
            pending_graph_screenshot: false,
            graph_screenshot_path: None,
            graph_panel_rect: egui::Rect::NOTHING,
            pending_action: None,
            close_confirmed: false,
            node_rects: HashMap::new(),
            selected_box: None,
        }
    }

    /// Compute a simple hash of all node positions in the snarl.
    fn node_position_hash(snarl: &Snarl<UiNode>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (id, pos, _) in snarl.nodes_pos_ids() {
            id.0.hash(&mut hasher);
            pos.x.to_bits().hash(&mut hasher);
            pos.y.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Returns `true` if the project has unsaved changes.
    fn is_dirty(&self, ctx: &egui::Context) -> bool {
        if self.graph.generation() != self.saved_gen {
            return true;
        }
        // Check if any panel width has changed since last save/load.
        if let (Some(saved), Some(current)) =
            (self.saved_panel_widths, Self::current_panel_widths(ctx))
        {
            for (s, c) in saved.iter().zip(current.iter()) {
                if (c - s).abs() > 1.0 {
                    return true;
                }
            }
        }
        // Check if node positions changed (layout drag).
        if Self::node_position_hash(&self.snarl) != self.saved_node_pos_hash {
            return true;
        }
        // Check if window position/size changed.
        if let (Some(saved), Some(current)) =
            (self.saved_window_geom, Self::current_window_rect(ctx))
        {
            for (s, c) in saved.iter().zip(current.iter()) {
                if (c - s).abs() > 1.0 {
                    return true;
                }
            }
        }
        // Check if view state changed (pan/zoom in either view).
        if let Some(ref saved_vs) = self.saved_view_state {
            let current_vs = self.current_view_state(ctx);
            if !saved_vs.approx_eq(&current_vs) {
                return true;
            }
        }
        false
    }

    /// Build a window title reflecting the current file and dirty state.
    fn window_title(&self, ctx: &egui::Context) -> String {
        let name = self
            .project_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        if self.is_dirty(ctx) {
            format!("{name}* — Vector Flow")
        } else {
            format!("{name} — Vector Flow")
        }
    }

    /// Panel IDs we track for dirty-checking layout changes.
    const TRACKED_PANELS: [&str; 2] = ["node_editor_panel", "properties"];

    /// Read current widths of all tracked panels.
    fn current_panel_widths(ctx: &egui::Context) -> Option<[f32; 2]> {
        let mut widths = [0.0f32; 2];
        for (i, id_str) in Self::TRACKED_PANELS.iter().enumerate() {
            widths[i] = egui::containers::panel::PanelState::load(
                ctx,
                egui::Id::new(*id_str),
            )?.rect.width();
        }
        Some(widths)
    }

    /// Read current window position and size as [x, y, w, h].
    fn current_window_rect(ctx: &egui::Context) -> Option<[f32; 4]> {
        ctx.input(|i| {
            let outer = i.viewport().outer_rect?;
            Some([outer.min.x, outer.min.y, outer.width(), outer.height()])
        })
    }

    /// Read current window geometry from the egui context.
    fn current_window_geometry(ctx: &egui::Context) -> Option<WindowGeometry> {
        let load_panel = |id: &str| {
            egui::containers::panel::PanelState::load(ctx, egui::Id::new(id))
                .map(|state| state.rect.width())
        };
        let node_editor_width = load_panel("node_editor_panel");
        let properties_width = load_panel("properties");

        ctx.input(|i| {
            let vp = i.viewport();
            let outer = vp.outer_rect?;
            Some(WindowGeometry {
                x: outer.min.x,
                y: outer.min.y,
                width: outer.width(),
                height: outer.height(),
                node_editor_width,
                properties_width,
            })
        })
    }

    /// Restore window geometry, clamped to screen.
    fn restore_window_geometry(ctx: &egui::Context, geom: Option<WindowGeometry>) {
        if let Some(mut geom) = geom {
            // Clamp to whatever screen info we have.
            let screen_size = ctx.input(|i| i.viewport().monitor_size);
            if let Some(monitor) = screen_size {
                geom.clamp_to_screen(monitor.x, monitor.y);
            } else {
                let rect = ctx.screen_rect();
                geom.clamp_to_screen(rect.width(), rect.height());
            }

            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                egui::Vec2::new(geom.width, geom.height),
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                egui::Pos2::new(geom.x, geom.y),
            ));

            // Restore panel widths by writing into egui's persisted data.
            let panels: [(&str, Option<f32>); 2] = [
                ("node_editor_panel", geom.node_editor_width),
                ("properties", geom.properties_width),
            ];
            for (id_str, width) in panels {
                if let Some(w) = width {
                    let panel_id = egui::Id::new(id_str);
                    let panel_rect = egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(w, geom.height),
                    );
                    let state = egui::containers::panel::PanelState { rect: panel_rect };
                    ctx.data_mut(|d| d.insert_persisted(panel_id, state));
                }
            }
        }
    }

    /// Determine which nodes' shapes to display on the canvas.
    fn visible_node_set(&self, selected_snarl_ids: &[SnarlNodeId]) -> Option<HashSet<CoreNodeId>> {
        // Pinned nodes always shown.
        let pinned: Vec<CoreNodeId> = self
            .snarl
            .node_ids()
            .filter_map(|(_, node)| {
                if node.pinned {
                    Some(node.core_id)
                } else {
                    None
                }
            })
            .collect();

        // Selected nodes (mapped to core IDs).
        let selected: Vec<CoreNodeId> = selected_snarl_ids
            .iter()
            .filter_map(|sid| {
                self.snarl
                    .get_node(*sid)
                    .map(|n| n.core_id)
            })
            .collect();

        if !pinned.is_empty() || !selected.is_empty() {
            // Show pinned + selected nodes.
            let mut set = HashSet::new();
            set.extend(pinned);
            set.extend(selected);
            Some(set)
        } else {
            // Nothing selected/pinned — show only GraphOutput nodes.
            // If no GraphOutput exists, show leaf nodes (those with no downstream connections).
            let graph_outputs: HashSet<CoreNodeId> = self
                .graph
                .nodes()
                .filter(|n| matches!(n.op, NodeOp::GraphOutput { .. }))
                .map(|n| n.id)
                .collect();

            if !graph_outputs.is_empty() {
                Some(graph_outputs)
            } else {
                // Find leaf nodes: nodes that never appear as a source in any edge.
                let sources: HashSet<CoreNodeId> = self
                    .graph
                    .edges()
                    .iter()
                    .map(|e| e.from.node)
                    .collect();
                let leaves: HashSet<CoreNodeId> = self
                    .graph
                    .nodes()
                    .filter(|n| !sources.contains(&n.id))
                    .map(|n| n.id)
                    .collect();

                if leaves.is_empty() {
                    None // Degenerate case — show all
                } else {
                    Some(leaves)
                }
            }
        }
    }

    // ── Align / Distribute helpers ──────────────────────────────────

    fn align_nodes(&mut self, selected: &[SnarlNodeId], mode: AlignMode) {
        if selected.len() < 2 {
            return;
        }

        // Collect each node's position and size (from cached rects).
        struct NodeGeo {
            id: SnarlNodeId,
            pos: egui::Pos2,
            width: f32,
            height: f32,
        }
        let nodes: Vec<NodeGeo> = selected
            .iter()
            .filter_map(|&id| {
                let info = self.snarl.get_node_info(id)?;
                let rect = self.node_rects.get(&id);
                let (w, h) = rect.map_or((0.0, 0.0), |r| (r.width(), r.height()));
                Some(NodeGeo { id, pos: info.pos, width: w, height: h })
            })
            .collect();
        if nodes.len() < 2 {
            return;
        }

        match mode {
            AlignMode::Left => {
                let target = nodes.iter().map(|n| n.pos.x).fold(f32::INFINITY, f32::min);
                for n in &nodes {
                    if let Some(info) = self.snarl.get_node_info_mut(n.id) {
                        info.pos.x = target;
                    }
                }
            }
            AlignMode::Right => {
                // Align right edges: find the max right edge, then set pos.x = target - width.
                let target = nodes.iter().map(|n| n.pos.x + n.width).fold(f32::NEG_INFINITY, f32::max);
                for n in &nodes {
                    if let Some(info) = self.snarl.get_node_info_mut(n.id) {
                        info.pos.x = target - n.width;
                    }
                }
            }
            AlignMode::Top => {
                let target = nodes.iter().map(|n| n.pos.y).fold(f32::INFINITY, f32::min);
                for n in &nodes {
                    if let Some(info) = self.snarl.get_node_info_mut(n.id) {
                        info.pos.y = target;
                    }
                }
            }
            AlignMode::Bottom => {
                // Align bottom edges: find the max bottom edge, then set pos.y = target - height.
                let target = nodes.iter().map(|n| n.pos.y + n.height).fold(f32::NEG_INFINITY, f32::max);
                for n in &nodes {
                    if let Some(info) = self.snarl.get_node_info_mut(n.id) {
                        info.pos.y = target - n.height;
                    }
                }
            }
            AlignMode::CenterH => {
                // Align horizontal centers to the first selected node's center.
                let target = nodes[0].pos.x + nodes[0].width * 0.5;
                for n in &nodes[1..] {
                    if let Some(info) = self.snarl.get_node_info_mut(n.id) {
                        info.pos.x = target - n.width * 0.5;
                    }
                }
            }
            AlignMode::CenterV => {
                // Align vertical centers to the first selected node's center.
                let target = nodes[0].pos.y + nodes[0].height * 0.5;
                for n in &nodes[1..] {
                    if let Some(info) = self.snarl.get_node_info_mut(n.id) {
                        info.pos.y = target - n.height * 0.5;
                    }
                }
            }
        }
    }

    fn distribute_nodes(&mut self, selected: &[SnarlNodeId], horizontal: bool) {
        if selected.len() < 3 {
            return;
        }
        let mut items: Vec<(SnarlNodeId, egui::Pos2)> = selected
            .iter()
            .filter_map(|&id| {
                self.snarl
                    .get_node_info(id)
                    .map(|info| (id, info.pos))
            })
            .collect();
        if items.len() < 3 {
            return;
        }

        // Sort by the relevant axis.
        if horizontal {
            items.sort_by(|a, b| a.1.x.partial_cmp(&b.1.x).unwrap());
        } else {
            items.sort_by(|a, b| a.1.y.partial_cmp(&b.1.y).unwrap());
        }

        let first = if horizontal { items.first().unwrap().1.x } else { items.first().unwrap().1.y };
        let last = if horizontal { items.last().unwrap().1.x } else { items.last().unwrap().1.y };
        let count = items.len() as f32;
        let step = (last - first) / (count - 1.0);

        for (i, (id, _)) in items.iter().enumerate() {
            if let Some(info) = self.snarl.get_node_info_mut(*id) {
                let val = first + step * i as f32;
                if horizontal {
                    info.pos.x = val;
                } else {
                    info.pos.y = val;
                }
            }
        }
    }

    // ── Network Box helpers ──────────────────────────────────────

    /// Compute the bounding rect of a network box in graph coordinates.
    fn network_box_rect(&self, nb: &vector_flow_core::graph::NetworkBox) -> Option<egui::Rect> {
        let mut rects = Vec::new();
        for &node_id in &nb.members {
            if let Some(snarl_id) = self.id_map.core_to_snarl(node_id) {
                if let Some(rect) = self.node_rects.get(&snarl_id) {
                    rects.push(*rect);
                }
            }
        }
        if rects.is_empty() {
            return None;
        }
        let mut combined = rects[0];
        for r in &rects[1..] {
            combined = combined.union(*r);
        }
        // Apply padding and space for title bar.
        let title_height = 20.0;
        Some(egui::Rect::from_min_max(
            egui::pos2(combined.min.x - nb.padding, combined.min.y - nb.padding - title_height),
            egui::pos2(combined.max.x + nb.padding, combined.max.y + nb.padding),
        ))
    }

    fn show_network_box_properties(&mut self, ui: &mut egui::Ui, box_id: NetworkBoxId) -> bool {
        let Some(nb) = self.graph.network_box(box_id) else {
            ui.label("Box not found");
            return false;
        };

        let mut title = nb.title.clone();
        let mut fill = [
            nb.fill_color[0] as f32 / 255.0,
            nb.fill_color[1] as f32 / 255.0,
            nb.fill_color[2] as f32 / 255.0,
            nb.fill_color[3] as f32 / 255.0,
        ];
        let mut stroke = [
            nb.stroke_color[0] as f32 / 255.0,
            nb.stroke_color[1] as f32 / 255.0,
            nb.stroke_color[2] as f32 / 255.0,
            nb.stroke_color[3] as f32 / 255.0,
        ];
        let mut stroke_width = nb.stroke_width;

        ui.heading("Network Box");
        ui.separator();

        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("Title");
            if ui.text_edit_singleline(&mut title).changed() {
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Fill");
            if ui.color_edit_button_rgba_unmultiplied(&mut fill).changed() {
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Stroke");
            if ui.color_edit_button_rgba_unmultiplied(&mut stroke).changed() {
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Stroke Width");
            if ui.add(egui::DragValue::new(&mut stroke_width).speed(0.1).range(0.0..=10.0)).changed() {
                changed = true;
            }
        });

        if changed {
            if let Some(nb) = self.graph.network_box_mut(box_id) {
                nb.title = title;
                nb.fill_color = [
                    (fill[0] * 255.0) as u8,
                    (fill[1] * 255.0) as u8,
                    (fill[2] * 255.0) as u8,
                    (fill[3] * 255.0) as u8,
                ];
                nb.stroke_color = [
                    (stroke[0] * 255.0) as u8,
                    (stroke[1] * 255.0) as u8,
                    (stroke[2] * 255.0) as u8,
                    (stroke[3] * 255.0) as u8,
                ];
                nb.stroke_width = stroke_width;
            }
        }

        changed
    }

    /// Compute the bounding rect of all nodes and network boxes in graph space.
    fn graph_content_bounds(&self) -> Option<egui::Rect> {
        let mut bounds: Option<egui::Rect> = None;

        // Include all node rects.
        for rect in self.node_rects.values() {
            bounds = Some(match bounds {
                Some(b) => b.union(*rect),
                None => *rect,
            });
        }

        // Include all network box rects (which extend beyond member nodes).
        for nb in self.graph.network_boxes() {
            if let Some(box_rect) = self.network_box_rect(nb) {
                bounds = Some(match bounds {
                    Some(b) => b.union(box_rect),
                    None => box_rect,
                });
            }
        }

        bounds
    }

    /// Custom fit-all that accounts for network boxes.
    fn fit_all(&self, ctx: &egui::Context, viewport: egui::Rect) {
        let Some(bounds) = self.graph_content_bounds() else { return };
        if bounds.is_negative() || bounds.area() == 0.0 {
            return;
        }

        let margin = 40.0; // screen-space margin
        let available = egui::vec2(
            (viewport.width() - margin * 2.0).max(1.0),
            (viewport.height() - margin * 2.0).max(1.0),
        );

        let scale_x = available.x / bounds.width();
        let scale_y = available.y / bounds.height();
        let scale = scale_x.min(scale_y).min(2.0); // cap at 2x zoom

        // Snarl offset is in screen-scaled space: screen = graph * scale - offset + viewport_center
        let offset = egui::vec2(bounds.center().x * scale, bounds.center().y * scale);

        Snarl::<UiNode>::set_view_state(
            NODE_EDITOR_ID,
            self.node_editor_ui_id,
            ctx,
            offset,
            scale,
        );
    }

    fn needs_eval(&self) -> bool {
        let gen = self.graph.generation();
        let frame = self.transport.eval_ctx.frame;
        gen != self.last_eval_gen || frame != self.last_eval_frame
    }

    fn evaluate(&mut self) {
        // Sync project directory for relative path resolution.
        self.transport.eval_ctx.project_dir = self
            .project_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Clear cache so downstream nodes pick up upstream changes.
        self.scheduler.clear_cache();
        match self.scheduler.evaluate(&mut self.graph, &self.transport.eval_ctx) {
            Ok(result) => {
                // Auto-populate LoadImage width/height from native dimensions.
                self.auto_populate_image_dims(&result);
                self.node_errors = result.errors.clone();
                self.last_eval = Some(result);
            }
            Err(e) => {
                log::error!("Evaluation failed: {e}");
            }
        }
        self.last_eval_gen = self.graph.generation();
        self.last_eval_frame = self.transport.eval_ctx.frame;
    }

    /// After evaluation, auto-populate LoadImage width/height defaults from
    /// native image dimensions when they are still at zero.
    fn auto_populate_image_dims(&mut self, result: &EvalResult) {
        // Collect updates first to avoid borrow conflict.
        let updates: Vec<(CoreNodeId, f64, f64)> = self
            .graph
            .nodes()
            .filter(|n| matches!(n.op, NodeOp::LoadImage { .. }))
            .filter(|n| {
                matches!(
                    n.inputs.get(1).and_then(|p| p.default_value.as_ref()),
                    Some(ParamValue::Float(v)) if *v == 0.0
                ) && matches!(
                    n.inputs.get(2).and_then(|p| p.default_value.as_ref()),
                    Some(ParamValue::Float(v)) if *v == 0.0
                )
            })
            .filter_map(|n| {
                let outputs = result.outputs.get(&n.id)?;
                let nw = match outputs.get(1) {
                    Some(NodeData::Scalar(v)) if *v > 0.0 => *v,
                    _ => return None,
                };
                let nh = match outputs.get(2) {
                    Some(NodeData::Scalar(v)) if *v > 0.0 => *v,
                    _ => return None,
                };
                Some((n.id, nw, nh))
            })
            .collect();

        for (node_id, nw, nh) in updates {
            if let Some(node) = self.graph.node_mut(node_id) {
                if let Some(port) = node.inputs.get_mut(1) {
                    port.default_value = Some(ParamValue::Float(nw));
                }
                if let Some(port) = node.inputs.get_mut(2) {
                    port.default_value = Some(ParamValue::Float(nh));
                }
            }
        }
    }

    /// Collect current view state (graph editor + canvas camera).
    fn current_view_state(&self, ctx: &egui::Context) -> ViewState {
        let (graph_offset, graph_scale) = Snarl::<UiNode>::get_view_state(
            NODE_EDITOR_ID,
            self.node_editor_ui_id,
            ctx,
        )
        .unwrap_or((egui::Vec2::ZERO, 1.0));

        ViewState {
            graph_offset: [graph_offset.x, graph_offset.y],
            graph_scale,
            canvas_center: [self.cam_state.current_center.x, self.cam_state.current_center.y],
            canvas_zoom: self.cam_state.current_zoom,
        }
    }

    fn save_project(&mut self, ctx: &egui::Context) {
        let path = if let Some(ref p) = self.project_path {
            Some(p.clone())
        } else {
            project::save_dialog(None)
        };

        if let Some(path) = path {
            let pf = ProjectFile {
                graph: self.graph.clone(),
                snarl: self.snarl.clone(),
                view_state: Some(self.current_view_state(ctx)),
                window_geometry: Self::current_window_geometry(ctx),
            };
            match pf.save(&path) {
                Ok(()) => {
                    log::info!("Saved project to {}", path.display());
                    self.project_path = Some(path);
                    self.saved_gen = self.graph.generation();
                    self.saved_panel_widths = Self::current_panel_widths(ctx);
                    self.saved_view_state = Some(self.current_view_state(ctx));
                    self.saved_node_pos_hash = Self::node_position_hash(&self.snarl);
                    self.saved_window_geom = Self::current_window_rect(ctx);
                }
                Err(e) => log::error!("Failed to save: {e}"),
            }
        }
    }

    fn save_project_as(&mut self, ctx: &egui::Context) {
        if let Some(path) = project::save_dialog(self.project_path.as_deref()) {
            self.project_path = Some(path);
            self.save_project(ctx);
        }
    }

    fn open_project(&mut self, ctx: &egui::Context) {
        if let Some(path) = project::open_dialog() {
            self.load_project_from(&path, ctx);
        }
    }

    /// Reset to a blank project (new file state).
    fn reset_to_new(&mut self) {
        self.graph = Graph::new();
        self.snarl = Snarl::new();
        self.id_map = IdMap::new();
        self.last_eval = None;
        self.node_errors.clear();
        self.last_eval_gen = u64::MAX;
        self.last_eval_frame = u64::MAX;
        self.prepared_scene = None;
        self.project_path = None;
        self.saved_gen = self.graph.generation();
        self.saved_node_pos_hash = Self::node_position_hash(&self.snarl);
        self.cam_state.do_reset = true;
        self.selected_box = None;
        self.saved_window_geom = None;
    }

    /// Request a new file, prompting for unsaved changes if dirty.
    fn request_new(&mut self, ctx: &egui::Context) {
        if self.is_dirty(ctx) {
            self.pending_action = Some(PendingAction::New);
        } else {
            self.reset_to_new();
        }
    }

    /// Close the current file: reset to new, then show Open dialog.
    /// If the user cancels the Open dialog, quit the app.
    fn close_file(&mut self, ctx: &egui::Context) {
        self.reset_to_new();
        if let Some(path) = project::open_dialog() {
            self.load_project_from(&path, ctx);
        } else {
            // User cancelled — quit.
            self.close_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Request a file close, prompting for unsaved changes if dirty.
    fn request_close_file(&mut self, ctx: &egui::Context) {
        if self.is_dirty(ctx) {
            self.pending_action = Some(PendingAction::CloseFile);
        } else {
            self.close_file(ctx);
        }
    }

    /// Request an open, prompting for unsaved changes if dirty.
    fn request_open(&mut self, ctx: &egui::Context) {
        if self.is_dirty(ctx) {
            self.pending_action = Some(PendingAction::Open);
        } else {
            self.open_project(ctx);
        }
    }

    /// Request a close, prompting for unsaved changes if dirty.
    fn request_close(&mut self, ctx: &egui::Context) {
        if self.is_dirty(ctx) && !self.close_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_action = Some(PendingAction::Close);
        }
    }

    fn load_project_from(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        match ProjectFile::load(path) {
            Ok(pf) => {
                self.graph = pf.graph;
                self.snarl = pf.snarl;
                self.id_map = project::rebuild_id_map(&self.snarl);
                self.last_eval = None;
                self.last_eval_gen = u64::MAX;
                self.last_eval_frame = u64::MAX;
                self.prepared_scene = None;
                self.project_path = Some(path.to_owned());
                self.saved_gen = self.graph.generation();
                Self::restore_window_geometry(ctx, pf.window_geometry);
                self.saved_panel_widths = Self::current_panel_widths(ctx);
                self.saved_view_state = pf.view_state.clone();
                self.saved_node_pos_hash = Self::node_position_hash(&self.snarl);
                self.saved_window_geom = Self::current_window_rect(ctx);

                // Re-snapshot after viewport commands settle (async resize/move).
                self.pending_snapshot_frames = 3;

                // Restore view state (graph editor + canvas camera).
                self.pending_view_restore = pf.view_state;

                log::info!("Loaded project from {}", path.display());
            }
            Err(e) => log::error!("Failed to load: {e}"),
        }
    }

    fn update_scene(&mut self, selected_snarl_ids: &[SnarlNodeId]) {
        if let Some(ref eval) = self.last_eval {
            let visible = self.visible_node_set(selected_snarl_ids);
            let collected = collect_scene(eval, visible.as_ref());
            let scene = prepare_scene_full(&collected, 0.5);
            self.prepared_scene = Some(scene);
        } else {
            self.prepared_scene = None;
        }
    }

    // ── Export helpers ─────────────────────────────────────────────────

    fn handle_exports(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Image export dialog.
        let img_action = export_dialog::show_image_export_dialog(ctx, &mut self.image_export_dialog);
        if matches!(img_action, export_dialog::ImageDialogAction::Export) {
            if let Some(render_state) = frame.wgpu_render_state() {
                let config = export::ImageExportConfig {
                    path: self.image_export_dialog.path.clone(),
                    width: self.image_export_dialog.width,
                    height: self.image_export_dialog.height,
                    camera: self.image_export_dialog.camera,
                };
                // Ensure we have a scene to export.
                let empty_scene = PreparedScene {
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    batches: Vec::new(),
                    image_batches: Vec::new(),
                };
                let scene = self.prepared_scene.as_ref().unwrap_or(&empty_scene);
                match export::export_canvas_image(
                    &render_state.device,
                    &render_state.queue,
                    scene,
                    &config,
                    self.cam_state.current_center,
                    self.cam_state.current_zoom,
                ) {
                    Ok(()) => {
                        self.image_export_dialog.last_success =
                            Some(format!("Exported to {}", config.path.display()));
                    }
                    Err(e) => {
                        self.image_export_dialog.last_error = Some(e);
                    }
                }
            } else {
                self.image_export_dialog.last_error =
                    Some("No render state available".to_string());
            }
        }

        // Video export dialog.
        let vid_action = export_dialog::show_video_export_dialog(
            ctx,
            &mut self.video_export_dialog,
            &self.export_state,
            self.transport.eval_ctx.fps,
        );
        match vid_action {
            export_dialog::VideoDialogAction::Start => {
                if let Some(render_state) = frame.wgpu_render_state() {
                    let d = &self.video_export_dialog;
                    let config = VideoExportConfig {
                        output_dir: d.output_dir.clone(),
                        mp4_path: if d.format == export::VideoFormat::Mp4 {
                            Some(d.mp4_path.clone())
                        } else {
                            None
                        },
                        format: d.format,
                        width: d.width,
                        height: d.height,
                        camera: d.camera,
                        start_frame: d.start_frame,
                        end_frame: d.end_frame,
                        fps: self.transport.eval_ctx.fps,
                    };
                    self.export_state =
                        export::start_video_export(&render_state.device, config);
                    ctx.request_repaint();
                }
            }
            export_dialog::VideoDialogAction::Cancel => {
                if let Some(err) = export::finish_video_export(&mut self.export_state) {
                    self.video_export_dialog.last_error = Some(err);
                } else {
                    self.video_export_dialog.last_success =
                        Some("Export cancelled".to_string());
                }
            }
            export_dialog::VideoDialogAction::None => {}
        }

        // Video export frame loop.
        if let ExportState::ExportingVideo {
            ref config,
            current_frame,
            ref error,
            ..
        } = self.export_state
        {
            if error.is_none() && current_frame <= config.end_frame {
                // Set transport to current export frame and re-evaluate.
                let export_frame = current_frame;
                self.transport.eval_ctx.frame = export_frame;
                self.transport.eval_ctx.time_secs =
                    export_frame as f32 / self.transport.eval_ctx.fps;
                self.evaluate();

                // Build scene for all nodes (no selection filter for export).
                let empty_scene;
                let scene = if let Some(ref eval) = self.last_eval {
                    let collected = collect_scene(eval, None);
                    empty_scene = prepare_scene_full(&collected, 0.5);
                    &empty_scene
                } else {
                    empty_scene = PreparedScene {
                        vertices: Vec::new(),
                        indices: Vec::new(),
                        batches: Vec::new(),
                        image_batches: Vec::new(),
                    };
                    &empty_scene
                };

                if let Some(render_state) = frame.wgpu_render_state() {
                    let done = export::export_video_frame(
                        &render_state.device,
                        &render_state.queue,
                        scene,
                        &mut self.export_state,
                        self.cam_state.current_center,
                        self.cam_state.current_zoom,
                    );

                    if done {
                        let result =
                            export::finish_video_export(&mut self.export_state);
                        if let Some(err) = result {
                            self.video_export_dialog.last_error = Some(err);
                        } else {
                            self.video_export_dialog.last_success =
                                Some("Video export complete".to_string());
                        }
                    } else {
                        ctx.request_repaint();
                    }
                }
            }
        }
    }

    fn handle_graph_screenshot(&mut self, ctx: &egui::Context) {
        if !self.pending_graph_screenshot {
            return;
        }

        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });

        if let Some(image) = screenshot {
            self.pending_graph_screenshot = false;
            if let Some(path) = self.graph_screenshot_path.take() {
                let ppp = ctx.pixels_per_point();

                // Compute tight crop around graph content in screen space.
                let crop = if let Some(content_rect) = self.graph_screenshot_content_rect(ctx) {
                    // Clamp to panel bounds and add padding.
                    let padding = 20.0;
                    let padded = content_rect.expand(padding);
                    let clamped = padded.intersect(self.graph_panel_rect);
                    // Convert to physical pixels.
                    egui::Rect::from_min_max(
                        egui::pos2(clamped.min.x * ppp, clamped.min.y * ppp),
                        egui::pos2(clamped.max.x * ppp, clamped.max.y * ppp),
                    )
                } else {
                    // Fallback: full panel rect.
                    egui::Rect::from_min_max(
                        egui::pos2(
                            self.graph_panel_rect.min.x * ppp,
                            self.graph_panel_rect.min.y * ppp,
                        ),
                        egui::pos2(
                            self.graph_panel_rect.max.x * ppp,
                            self.graph_panel_rect.max.y * ppp,
                        ),
                    )
                };

                match export::save_graph_screenshot(&image, crop, &path) {
                    Ok(()) => log::info!("Graph screenshot saved to {}", path.display()),
                    Err(e) => log::error!("Failed to save graph screenshot: {e}"),
                }
            }
        }
    }

    /// Compute the screen-space bounding rect of all graph content (nodes + network boxes).
    fn graph_screenshot_content_rect(&self, ctx: &egui::Context) -> Option<egui::Rect> {
        let (view_offset, view_scale) = Snarl::<UiNode>::get_view_state(
            NODE_EDITOR_ID,
            self.node_editor_ui_id,
            ctx,
        )?;

        let viewport_center = self.graph_panel_rect.center();
        let to_screen = |p: egui::Pos2| -> egui::Pos2 {
            egui::pos2(
                p.x * view_scale - view_offset.x + viewport_center.x,
                p.y * view_scale - view_offset.y + viewport_center.y,
            )
        };

        let mut bounds: Option<egui::Rect> = None;
        let mut extend = |rect: egui::Rect| {
            let screen_rect = egui::Rect::from_min_max(
                to_screen(rect.min),
                to_screen(rect.max),
            );
            bounds = Some(match bounds {
                Some(b) => b.union(screen_rect),
                None => screen_rect,
            });
        };

        for rect in self.node_rects.values() {
            extend(*rect);
        }
        for nb in self.graph.network_boxes() {
            if let Some(box_rect) = self.network_box_rect(nb) {
                extend(box_rect);
            }
        }

        bounds
    }

    /// Show the unsaved-changes dialog. Returns `true` if the pending action was resolved.
    fn show_unsaved_dialog(&mut self, ctx: &egui::Context) {
        let action = match self.pending_action {
            Some(a) => a,
            None => return,
        };

        let mut save = false;
        let mut discard = false;
        let mut cancel = false;

        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. What would you like to do?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    save = ui.button("Save").clicked();
                    discard = ui.button("Discard").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });

        if save {
            self.save_project(ctx);
            self.pending_action = None;
            self.finish_pending_action(action, ctx);
        } else if discard {
            self.pending_action = None;
            self.finish_pending_action(action, ctx);
        } else if cancel {
            self.pending_action = None;
        }
    }

    /// Execute the action after the unsaved-changes dialog is resolved.
    fn finish_pending_action(&mut self, action: PendingAction, ctx: &egui::Context) {
        match action {
            PendingAction::Open => {
                self.open_project(ctx);
            }
            PendingAction::Close => {
                self.close_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingAction::New => {
                self.reset_to_new();
            }
            PendingAction::CloseFile => {
                self.close_file(ctx);
            }
        }
    }
}

impl eframe::App for VectorFlowApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Re-snapshot saved state after load once viewport commands have settled.
        if self.pending_snapshot_frames > 0 {
            self.pending_snapshot_frames -= 1;
            if self.pending_snapshot_frames == 0 {
                self.saved_window_geom = Self::current_window_rect(ctx);
                self.saved_panel_widths = Self::current_panel_widths(ctx);
                self.saved_view_state = Some(self.current_view_state(ctx));
                self.saved_node_pos_hash = Self::node_position_hash(&self.snarl);
            }
            ctx.request_repaint();
        }

        // Update window title to reflect dirty state.
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title(ctx)));

        // Handle close request (window X button).
        if ctx.input(|i| i.viewport().close_requested()) {
            self.request_close(ctx);
        }

        // Show unsaved-changes dialog if pending.
        self.show_unsaved_dialog(ctx);

        // If a dialog is open, skip the rest of the UI to act as a modal.
        if self.pending_action.is_some() {
            return;
        }

        // 0. Keyboard shortcuts.
        // Skip non-modifier shortcuts (F, arrows) when a text widget has focus.
        let text_editing = ctx.memory(|m| m.focused().is_some())
            && ctx.input(|i| !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.alt);
        let (do_save, do_save_as, do_open, do_new, do_close_file, do_duplicate, do_fit_all, do_quit,
         do_export_image,
         do_align_left, do_align_right, do_align_top, do_align_bottom,
         do_dist_h, do_dist_v, nudge) = ctx.input_mut(|i| {
            let save_as = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            ));
            let save = !save_as
                && i.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::S,
                ));
            let open = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::O,
            ));
            let new = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::N,
            ));
            let close_file = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::W,
            ));
            let duplicate = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::D,
            ));
            // Non-modifier shortcuts: only when no text widget has focus.
            let fit_all = !text_editing && i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::NONE,
                egui::Key::F,
            ));
            let export_image = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::E,
            ));
            let quit = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Q,
            ));
            // Arrow key nudge — only when no text widget has focus.
            let mut nudge = egui::Vec2::ZERO;
            if !text_editing {
                let nudge_step = if i.modifiers.shift { 10.0 } else { 1.0 };
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowLeft))
                    || i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::ArrowLeft))
                {
                    nudge.x = -nudge_step;
                }
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowRight))
                    || i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::ArrowRight))
                {
                    nudge.x = nudge_step;
                }
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowUp))
                    || i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::ArrowUp))
                {
                    nudge.y = -nudge_step;
                }
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowDown))
                    || i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::ArrowDown))
                {
                    nudge.y = nudge_step;
                }
            }

            (save, save_as, open, new, close_file, duplicate, fit_all, quit,
             export_image,
             false, false, false, false, false, false, nudge)
        });
        if do_save {
            self.save_project(ctx);
        }
        if do_save_as {
            self.save_project_as(ctx);
        }
        if do_open {
            self.request_open(ctx);
        }
        if do_new {
            self.request_new(ctx);
        }
        if do_close_file {
            self.request_close_file(ctx);
        }
        if do_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // 1. Transport tick — advance time if playing.
        let time_changed = self.transport.tick();
        if self.transport.playback == transport_panel::PlaybackState::Playing {
            ctx.request_repaint();
        }

        // 2. Top panel: menu bar + transport bar.
        let mut transport_changed = false;
        let mut menu_save = false;
        let mut menu_save_as = false;
        let mut menu_open = false;
        let mut menu_new = false;
        let mut menu_close_file = false;
        let mut menu_export_image = false;
        let mut menu_export_video = false;
        let mut menu_graph_screenshot = false;
        let mut menu_align: Option<AlignMode> = None;
        let mut menu_dist_h = false;
        let mut menu_dist_v = false;
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New  Ctrl+N").clicked() {
                        ui.close_menu();
                        menu_new = true;
                    }
                    if ui.button("Open...  Ctrl+O").clicked() {
                        ui.close_menu();
                        menu_open = true;
                    }
                    if ui.button("Close  Ctrl+W").clicked() {
                        ui.close_menu();
                        menu_close_file = true;
                    }
                    ui.separator();
                    if ui.button("Save  Ctrl+S").clicked() {
                        ui.close_menu();
                        menu_save = true;
                    }
                    if ui.button("Save As...  Ctrl+Shift+S").clicked() {
                        ui.close_menu();
                        menu_save_as = true;
                    }
                    ui.separator();
                    if ui.button("Export Canvas Image...  Ctrl+Shift+E").clicked() {
                        ui.close_menu();
                        menu_export_image = true;
                    }
                    if ui.button("Export Canvas Video...").clicked() {
                        ui.close_menu();
                        menu_export_video = true;
                    }
                    if ui.button("Save Graph Screenshot...").clicked() {
                        ui.close_menu();
                        menu_graph_screenshot = true;
                    }
                    ui.separator();
                    if ui.button("Quit  Ctrl+Q").clicked() {
                        ui.close_menu();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Arrange", |ui| {
                    ui.label("Align");
                    if ui.button("Align Left").clicked() {
                        menu_align = Some(AlignMode::Left);
                        ui.close_menu();
                    }
                    if ui.button("Align Right").clicked() {
                        menu_align = Some(AlignMode::Right);
                        ui.close_menu();
                    }
                    if ui.button("Align Top").clicked() {
                        menu_align = Some(AlignMode::Top);
                        ui.close_menu();
                    }
                    if ui.button("Align Bottom").clicked() {
                        menu_align = Some(AlignMode::Bottom);
                        ui.close_menu();
                    }
                    if ui.button("Align Centers Horizontally").clicked() {
                        menu_align = Some(AlignMode::CenterH);
                        ui.close_menu();
                    }
                    if ui.button("Align Centers Vertically").clicked() {
                        menu_align = Some(AlignMode::CenterV);
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label("Distribute");
                    if ui.button("Distribute Horizontally").clicked() {
                        menu_dist_h = true;
                        ui.close_menu();
                    }
                    if ui.button("Distribute Vertically").clicked() {
                        menu_dist_v = true;
                        ui.close_menu();
                    }
                });
            });
            ui.separator();
            transport_changed = transport_panel::show_transport_bar(ui, &mut self.transport);
        });
        if menu_new {
            self.request_new(ctx);
        }
        if menu_open {
            self.request_open(ctx);
        }
        if menu_close_file {
            self.request_close_file(ctx);
        }
        if menu_save {
            self.save_project(ctx);
        }
        if menu_save_as {
            self.save_project_as(ctx);
        }
        if menu_export_image || do_export_image {
            self.image_export_dialog.open = true;
        }
        if menu_export_video {
            self.video_export_dialog.open = true;
        }
        if menu_graph_screenshot {
            if let Some(mut path) = rfd::FileDialog::new()
                .set_title("Save Graph Screenshot")
                .add_filter("PNG Image", &["png"])
                .save_file()
            {
                if path.extension().is_none() {
                    path.set_extension("png");
                }
                self.graph_screenshot_path = Some(path);
                self.pending_graph_screenshot = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::UserData::default(),
                ));
            }
        }

        // 3. Left panel: node editor (fills full height).
        // Rendered first so snarl updates selection state before we query it.
        let mut snarl_viewport = egui::Rect::NOTHING;
        let mut snarl_layer_id = egui::LayerId::background();
        let mut viewer_actions = ViewerActions::default();
        let mut box_shape_slots = Vec::new();
        egui::SidePanel::left("node_editor_panel")
            .default_width(ctx.screen_rect().width() * 0.55)
            .resizable(true)
            .show(ctx, |ui| {
                self.node_editor_ui_id = ui.id();
                snarl_viewport = ui.max_rect();
                snarl_layer_id = ui.layer_id();

                self.graph_panel_rect = ui.max_rect();
                self.node_rects.clear();
                let mut viewer = GraphViewer {
                    graph: &mut self.graph,
                    id_map: &mut self.id_map,
                    catalog: &self.catalog,
                    node_rects: &mut self.node_rects,
                    actions: &mut viewer_actions,
                    box_shape_slots: &mut box_shape_slots,
                };
                self.snarl.show(&mut viewer, &self.snarl_style, NODE_EDITOR_ID, ui);
            });

        // 3.1. Render network boxes behind nodes and handle interaction.
        {
            let (view_offset, view_scale) = Snarl::<UiNode>::get_view_state(
                NODE_EDITOR_ID,
                self.node_editor_ui_id,
                ctx,
            ).unwrap_or((egui::Vec2::ZERO, 1.0));

            let viewport_center = snarl_viewport.center();
            let to_screen = |p: egui::Pos2| -> egui::Pos2 {
                egui::pos2(
                    p.x * view_scale - view_offset.x + viewport_center.x,
                    p.y * view_scale - view_offset.y + viewport_center.y,
                )
            };

            // Collect box rendering data (avoid borrowing graph while interacting).
            struct BoxRender {
                id: NetworkBoxId,
                screen_rect: egui::Rect,
                fill: egui::Color32,
                stroke: egui::Stroke,
                title: String,
                title_font_size: f32,
                title_bar_height: f32,
                rect_idx: egui::layers::ShapeIdx,
                text_idx: egui::layers::ShapeIdx,
            }
            let mut box_renders = Vec::new();

            for (box_id, rect_idx, text_idx) in &box_shape_slots {
                let Some(nb) = self.graph.network_box(*box_id) else { continue };
                let Some(box_rect) = self.network_box_rect(nb) else { continue };

                let screen_rect = egui::Rect::from_min_max(
                    to_screen(box_rect.min),
                    to_screen(box_rect.max),
                );
                if screen_rect.intersect(snarl_viewport).is_negative() {
                    continue;
                }

                let fill = nb.fill_color;
                let stroke_c = nb.stroke_color;

                box_renders.push(BoxRender {
                    id: *box_id,
                    screen_rect,
                    fill: egui::Color32::from_rgba_unmultiplied(fill[0], fill[1], fill[2], fill[3]),
                    stroke: egui::Stroke::new(nb.stroke_width, egui::Color32::from_rgba_unmultiplied(stroke_c[0], stroke_c[1], stroke_c[2], stroke_c[3])),
                    title: nb.title.clone(),
                    title_font_size: 14.0 * view_scale,
                    title_bar_height: 22.0 * view_scale,
                    rect_idx: *rect_idx,
                    text_idx: *text_idx,
                });
            }

            // Fill in the pre-reserved shape slots so boxes render behind nodes.
            // Clip to the snarl viewport so boxes don't bleed outside the panel.
            let painter = ctx.layer_painter(snarl_layer_id).with_clip_rect(snarl_viewport);
            for br in &box_renders {
                painter.set(
                    br.rect_idx,
                    egui::epaint::RectShape::new(
                        br.screen_rect,
                        egui::CornerRadius::same(4),
                        br.fill,
                        br.stroke,
                        egui::StrokeKind::Outside,
                    ),
                );

                let title_pos = egui::pos2(br.screen_rect.min.x + 6.0, br.screen_rect.min.y + 3.0);
                // Fully opaque version of stroke color for the title text.
                let c = br.stroke.color;
                let title_color = egui::Color32::from_rgb(c.r(), c.g(), c.b());
                let galley = ctx.fonts(|f| {
                    f.layout_no_wrap(
                        br.title.clone(),
                        egui::FontId::proportional(br.title_font_size),
                        title_color,
                    )
                });
                painter.set(
                    br.text_idx,
                    egui::epaint::TextShape::new(title_pos, galley, title_color),
                );
            }

            // Handle interaction on box title bars without blocking scroll/zoom events.
            // We use interactable(false) so the Area layer doesn't block scroll from
            // reaching snarl underneath. The inner widget still detects click/drag.
            let mut box_to_delete = None;
            for br in &box_renders {
                let title_bar = egui::Rect::from_min_size(
                    br.screen_rect.min,
                    egui::vec2(br.screen_rect.width(), br.title_bar_height),
                );
                // Clip title bar interaction to the graph panel so it can't
                // overlap menus or other panels when zoomed/panned.
                let clipped_title = title_bar.intersect(self.graph_panel_rect);
                if !clipped_title.is_positive() {
                    continue;
                }
                let area_resp = egui::Area::new(egui::Id::new(("netbox_area", br.id.0)))
                    .fixed_pos(clipped_title.min)
                    .order(egui::Order::Middle)
                    .interactable(false)
                    .show(ctx, |ui| {
                        ui.set_min_size(clipped_title.size());
                        let (_, resp) = ui.allocate_exact_size(clipped_title.size(), egui::Sense::click_and_drag());
                        resp
                    });
                let resp = area_resp.inner;

                if resp.clicked() {
                    self.selected_box = Some(br.id);
                }

                if resp.dragged() {
                    let delta = resp.drag_delta();
                    let graph_delta = egui::vec2(delta.x / view_scale, delta.y / view_scale);
                    if let Some(nb) = self.graph.network_box(br.id) {
                        let member_ids: Vec<_> = nb.members.iter().copied().collect();
                        for core_id in member_ids {
                            if let Some(snarl_id) = self.id_map.core_to_snarl(core_id) {
                                if let Some(info) = self.snarl.get_node_info_mut(snarl_id) {
                                    info.pos += graph_delta;
                                }
                            }
                        }
                    }
                }

                resp.context_menu(|ui: &mut egui::Ui| {
                    if ui.button("Delete Network Box").clicked() {
                        box_to_delete = Some(br.id);
                        ui.close_menu();
                    }
                });
            }

            if let Some(bid) = box_to_delete {
                self.graph.remove_network_box(bid);
                if self.selected_box == Some(bid) {
                    self.selected_box = None;
                }
            }
        }

        // Apply pending view restore (graph editor offset/scale).
        if let Some(vs) = self.pending_view_restore.take() {
            Snarl::<UiNode>::set_view_state(
                NODE_EDITOR_ID,
                self.node_editor_ui_id,
                ctx,
                egui::Vec2::new(vs.graph_offset[0], vs.graph_offset[1]),
                vs.graph_scale,
            );
            // Canvas camera restore is deferred to step 8 where we have render_state.
            self.pending_canvas_restore = Some((
                Vec2::new(vs.canvas_center[0], vs.canvas_center[1]),
                vs.canvas_zoom,
            ));
        }

        // 3a. Handle viewer actions (network box operations).
        // Note: selected_snarl not yet available here, so create_box_from is
        // handled after selection query below.
        if let Some((snarl_id, box_id)) = viewer_actions.add_to_box {
            if let Some(ui_node) = self.snarl.get_node(snarl_id) {
                self.graph.add_node_to_box(ui_node.core_id, box_id);
            }
        }
        if let Some(snarl_id) = viewer_actions.remove_from_box {
            if let Some(ui_node) = self.snarl.get_node(snarl_id) {
                self.graph.remove_node_from_any_box(ui_node.core_id);
            }
        }
        if let Some(box_id) = viewer_actions.delete_box {
            self.graph.remove_network_box(box_id);
            if self.selected_box == Some(box_id) {
                self.selected_box = None;
            }
        }

        // Overlay "Show All" button on the node editor panel (hidden during screenshot).
        {
            let inset = canvas_panel::TOOLBAR_INSET * 2.0;
            let mut fit_requested = do_fit_all;
            if !self.pending_graph_screenshot {
                egui::Area::new(egui::Id::new("graph_toolbar"))
                    .fixed_pos(egui::pos2(snarl_viewport.left() + inset, snarl_viewport.top() + inset))
                    .order(egui::Order::Foreground)
                    .interactable(true)
                    .show(ctx, |ui| {
                        if ui.button("Show All").on_hover_text("Fit all nodes in view (F)").clicked() {
                            fit_requested = true;
                        }
                    });
            }
            if fit_requested {
                self.fit_all(ctx, snarl_viewport);
            }
        }

        // Query snarl's built-in selection (must happen after snarl.show).
        let mut selected_snarl =
            Snarl::<UiNode>::get_selected_nodes_at(NODE_EDITOR_ID, self.node_editor_ui_id, ctx);

        // 3b. Duplicate selected nodes on Ctrl+D.
        if do_duplicate && !selected_snarl.is_empty() {
            let mut dup_actions = ViewerActions::default();
            let mut dup_slots = Vec::new();
            let mut viewer = GraphViewer {
                graph: &mut self.graph,
                id_map: &mut self.id_map,
                catalog: &self.catalog,
                node_rects: &mut self.node_rects,
                actions: &mut dup_actions,
                box_shape_slots: &mut dup_slots,
            };
            let new_ids = viewer.duplicate_nodes(&selected_snarl, &mut self.snarl);
            // Select the new duplicates instead of the originals.
            Snarl::<UiNode>::set_selected_nodes_at(
                NODE_EDITOR_ID,
                self.node_editor_ui_id,
                ctx,
                new_ids.clone(),
            );
            selected_snarl = new_ids;
        }

        // 3c. Align / distribute selected nodes.
        if let Some(mode) = menu_align {
            self.align_nodes(&selected_snarl, mode);
        }
        if do_align_left { self.align_nodes(&selected_snarl, AlignMode::Left); }
        if do_align_right { self.align_nodes(&selected_snarl, AlignMode::Right); }
        if do_align_top { self.align_nodes(&selected_snarl, AlignMode::Top); }
        if do_align_bottom { self.align_nodes(&selected_snarl, AlignMode::Bottom); }
        if menu_dist_h || do_dist_h { self.distribute_nodes(&selected_snarl, true); }
        if menu_dist_v || do_dist_v { self.distribute_nodes(&selected_snarl, false); }

        // 3d. Create Network Box from selection (deferred from viewer action).
        if !viewer_actions.create_box_from.is_empty() {
            // Use current selection if available, otherwise fall back to the right-clicked node.
            let nodes_for_box = if selected_snarl.len() > 1 {
                &selected_snarl
            } else {
                &viewer_actions.create_box_from
            };
            let members: HashSet<CoreNodeId> = nodes_for_box
                .iter()
                .filter_map(|sid| self.snarl.get_node(*sid).map(|n| n.core_id))
                .collect();
            if !members.is_empty() {
                let title = self.graph.next_box_title();
                let box_id = self.graph.add_network_box(title, members);
                self.selected_box = Some(box_id);
            }
        }

        // 3e. Nudge selected nodes with arrow keys.
        if nudge != egui::Vec2::ZERO && !selected_snarl.is_empty() {
            for &sid in &selected_snarl {
                if let Some(info) = self.snarl.get_node_info_mut(sid) {
                    info.pos += nudge;
                }
            }
        }

        // Clear box selection when nodes are selected.
        if !selected_snarl.is_empty() {
            self.selected_box = None;
        }

        // 4. Right panel: properties inspector.
        let mut props_changed = false;
        egui::SidePanel::right("properties")
            .default_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Properties");
                ui.separator();

                if let Some(box_id) = self.selected_box {
                    // Show network box properties.
                    props_changed = self.show_network_box_properties(ui, box_id);
                } else {
                    let selected_core: Vec<CoreNodeId> = selected_snarl
                        .iter()
                        .filter_map(|sid| {
                            self.snarl.get_node(*sid).map(|n| n.core_id)
                        })
                        .collect();

                    props_changed =
                        properties_panel::show_properties_panel(ui, &mut self.graph, &selected_core, &self.node_errors);

                    // Sync portal display names after properties edit.
                    if props_changed {
                        for &sid in &selected_snarl {
                            if let Some(ui_node) = self.snarl.get_node_mut(sid) {
                                if let Some(core_node) = self.graph.node(ui_node.core_id) {
                                    if matches!(core_node.op, NodeOp::PortalSend { .. } | NodeOp::PortalReceive { .. }) {
                                        ui_node.display_name = core_node.name.clone();
                                    }
                                }
                            }
                        }
                    }
                }
            });

        // 5. Remaining central panel: canvas preview.
        let mut canvas_rect = egui::Rect::NOTHING;
        egui::CentralPanel::default().show(ctx, |ui| {
            canvas_rect = canvas_panel::show_canvas_panel(
                ui,
                self.prepared_scene.take(),
                &mut self.cam_state,
            );
        });

        // Overlay toolbar buttons on the canvas panel.
        {
            let inset = canvas_panel::TOOLBAR_INSET;
            egui::Area::new(egui::Id::new("canvas_toolbar"))
                .fixed_pos(egui::pos2(canvas_rect.left() + inset, canvas_rect.top() + inset))
                .order(egui::Order::Foreground)
                .interactable(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Reset").on_hover_text("Reset zoom to 100% and center on origin").clicked() {
                            self.cam_state.do_reset = true;
                        }
                        if ui.button("Show All").on_hover_text("Fit all content in view").clicked() {
                            self.cam_state.do_show_all = true;
                        }
                    });
                });
        }

        // 6. Evaluate graph if something changed.
        if self.needs_eval() || time_changed || transport_changed || props_changed {
            self.evaluate();
        }

        // 7. Always update scene (selection may have changed visible nodes).
        self.update_scene(&selected_snarl);

        // 8. Export dialogs and video export loop.
        self.handle_exports(ctx, frame);

        // 9. Graph screenshot: check for captured image from previous frame.
        self.handle_graph_screenshot(ctx);

        // 10. Apply camera commands.
        if let Some(render_state) = frame.wgpu_render_state() {
            // Restore canvas camera if pending from a project load.
            if let Some((center, zoom)) = self.pending_canvas_restore.take() {
                canvas_panel::restore_camera(render_state, &mut self.cam_state, center, zoom);
            }
            canvas_panel::apply_camera_commands(render_state, &mut self.cam_state);
        }
    }
}
