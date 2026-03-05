use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use egui_snarl::ui::SnarlStyle;
use egui_snarl::{NodeId as SnarlNodeId, Snarl};
use glam::Vec2;

use vector_flow_core::graph::Graph;
use vector_flow_core::node::NodeOp;
use vector_flow_core::scheduler::{EvalResult, Scheduler};
use vector_flow_core::types::NodeId as CoreNodeId;
use vector_flow_compute::CpuBackend;
use vector_flow_render::overlay::CanvasRenderResources;
use vector_flow_render::renderer::CanvasRenderer;
use vector_flow_render::camera::Camera;
use vector_flow_render::{collect_shapes, prepare_scene, PreparedScene};

use crate::canvas_panel::{self, CameraState};
use crate::id_map::IdMap;
use crate::project::{self, ProjectFile, ViewState, WindowGeometry};
use crate::properties_panel;
use crate::transport_panel::{self, TransportState};
use crate::ui_node::{node_catalog, CatalogEntry, UiNode};
use crate::viewer::GraphViewer;

const NODE_EDITOR_ID: &str = "node_editor";

/// Tracks what action triggered the "unsaved changes" dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    Close,
    Open,
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
            last_eval_gen: u64::MAX,
            last_eval_frame: u64::MAX,
            prepared_scene: None,
            project_path: None,
            saved_gen: 0,
            saved_panel_widths: None,
            saved_view_state: None,
            saved_node_pos_hash,
            node_editor_ui_id: egui::Id::NULL,
            pending_view_restore: None,
            pending_canvas_restore: None,
            pending_action: None,
            close_confirmed: false,
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

    /// Save window geometry to the sidecar file for the current project.
    fn save_window_geometry(&self, ctx: &egui::Context) {
        if let Some(ref path) = self.project_path {
            if let Some(geom) = Self::current_window_geometry(ctx) {
                project::save_window_geometry(path, &geom);
            }
        }
    }

    /// Restore window geometry from the sidecar file, clamped to screen.
    fn restore_window_geometry(ctx: &egui::Context, project_path: &std::path::Path) {
        if let Some(mut geom) = project::load_window_geometry(project_path) {
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

    fn needs_eval(&self) -> bool {
        let gen = self.graph.generation();
        let frame = self.transport.time_ctx.frame;
        gen != self.last_eval_gen || frame != self.last_eval_frame
    }

    fn evaluate(&mut self) {
        // Clear cache so downstream nodes pick up upstream changes.
        self.scheduler.clear_cache();
        match self.scheduler.evaluate(&mut self.graph, &self.transport.time_ctx) {
            Ok(result) => {
                self.last_eval = Some(result);
            }
            Err(e) => {
                log::error!("Evaluation failed: {e}");
            }
        }
        self.last_eval_gen = self.graph.generation();
        self.last_eval_frame = self.transport.time_ctx.frame;
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
            };
            match pf.save(&path) {
                Ok(()) => {
                    log::info!("Saved project to {}", path.display());
                    self.project_path = Some(path);
                    self.saved_gen = self.graph.generation();
                    self.saved_panel_widths = Self::current_panel_widths(ctx);
                    self.saved_view_state = Some(self.current_view_state(ctx));
                    self.saved_node_pos_hash = Self::node_position_hash(&self.snarl);
                    self.save_window_geometry(ctx);
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
        } else {
            // Save geometry on clean close.
            self.save_window_geometry(ctx);
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
                Self::restore_window_geometry(ctx, path);
                self.saved_panel_widths = Self::current_panel_widths(ctx);
                self.saved_node_pos_hash = Self::node_position_hash(&self.snarl);
                self.saved_view_state = pf.view_state.clone();

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
            let shapes = collect_shapes(eval, visible.as_ref());
            let scene = prepare_scene(&shapes, 0.5);
            self.prepared_scene = Some(scene);
        } else {
            self.prepared_scene = None;
        }
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
                self.save_window_geometry(ctx);
                self.close_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

impl eframe::App for VectorFlowApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
        let (do_save, do_save_as, do_open, do_duplicate, do_fit_all, do_quit) = ctx.input_mut(|i| {
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
            let duplicate = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::D,
            ));
            let fit_all = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::NONE,
                egui::Key::F,
            ));
            let quit = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Q,
            ));
            (save, save_as, open, duplicate, fit_all, quit)
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
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open...  Ctrl+O").clicked() {
                        ui.close_menu();
                        menu_open = true;
                    }
                    if ui.button("Save  Ctrl+S").clicked() {
                        ui.close_menu();
                        menu_save = true;
                    }
                    if ui.button("Save As...  Ctrl+Shift+S").clicked() {
                        ui.close_menu();
                        menu_save_as = true;
                    }
                    ui.separator();
                    if ui.button("Quit  Ctrl+Q").clicked() {
                        ui.close_menu();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
            ui.separator();
            transport_changed = transport_panel::show_transport_bar(ui, &mut self.transport);
        });
        if menu_open {
            self.request_open(ctx);
        }
        if menu_save {
            self.save_project(ctx);
        }
        if menu_save_as {
            self.save_project_as(ctx);
        }

        // 3. Left panel: node editor (fills full height).
        // Rendered first so snarl updates selection state before we query it.
        let mut snarl_viewport = egui::Rect::NOTHING;
        egui::SidePanel::left("node_editor_panel")
            .default_width(ctx.screen_rect().width() * 0.55)
            .resizable(true)
            .show(ctx, |ui| {
                self.node_editor_ui_id = ui.id();
                snarl_viewport = ui.max_rect();

                let mut viewer = GraphViewer {
                    graph: &mut self.graph,
                    id_map: &mut self.id_map,
                    catalog: &self.catalog,
                };
                self.snarl.show(&mut viewer, &self.snarl_style, NODE_EDITOR_ID, ui);
            });

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

        // Overlay "Show All" button on the node editor panel.
        {
            let inset = canvas_panel::TOOLBAR_INSET * 2.0;
            let mut fit_requested = do_fit_all;
            egui::Area::new(egui::Id::new("graph_toolbar"))
                .fixed_pos(egui::pos2(snarl_viewport.left() + inset, snarl_viewport.top() + inset))
                .order(egui::Order::Foreground)
                .interactable(true)
                .show(ctx, |ui| {
                    if ui.button("Show All").on_hover_text("Fit all nodes in view (F)").clicked() {
                        fit_requested = true;
                    }
                });
            if fit_requested {
                Snarl::<UiNode>::request_fit_all(NODE_EDITOR_ID, self.node_editor_ui_id, ctx);
            }
        }

        // Query snarl's built-in selection (must happen after snarl.show).
        let mut selected_snarl =
            Snarl::<UiNode>::get_selected_nodes_at(NODE_EDITOR_ID, self.node_editor_ui_id, ctx);

        // 3b. Duplicate selected nodes on Ctrl+D.
        if do_duplicate && !selected_snarl.is_empty() {
            let mut viewer = GraphViewer {
                graph: &mut self.graph,
                id_map: &mut self.id_map,
                catalog: &self.catalog,
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

        // 4. Right panel: properties inspector.
        let mut props_changed = false;
        egui::SidePanel::right("properties")
            .default_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Properties");
                ui.separator();

                let selected_core: Vec<CoreNodeId> = selected_snarl
                    .iter()
                    .filter_map(|sid| {
                        self.snarl.get_node(*sid).map(|n| n.core_id)
                    })
                    .collect();

                props_changed =
                    properties_panel::show_properties_panel(ui, &mut self.graph, &selected_core);
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

        // 8. Apply camera commands.
        if let Some(render_state) = frame.wgpu_render_state() {
            // Restore canvas camera if pending from a project load.
            if let Some((center, zoom)) = self.pending_canvas_restore.take() {
                canvas_panel::restore_camera(render_state, &mut self.cam_state, center, zoom);
            }
            canvas_panel::apply_camera_commands(render_state, &mut self.cam_state);
        }
    }
}
