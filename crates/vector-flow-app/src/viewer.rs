use std::collections::HashMap;

use egui::{Color32, Frame, Label, Pos2, Rect, RichText, Ui};
use egui::layers::ShapeIdx;
use egui_snarl::ui::{PinInfo, SnarlViewer};
use egui_snarl::{InPin, InPinId, NodeId as SnarlNodeId, OutPin, OutPinId, Snarl};

use vector_flow_core::graph::Graph;
use vector_flow_core::node::{PortId, PortIndex};
use vector_flow_core::types::{NetworkBoxId, NodeId as CoreNodeId};

use crate::id_map::IdMap;
use crate::ui_node::{data_type_color, node_op_label, CatalogEntry, NodeCategory, UiNode};

/// Actions from context menus that need to be handled by the app after snarl.show().
#[derive(Default)]
pub struct ViewerActions {
    pub create_box_from: Vec<SnarlNodeId>,
    pub add_to_box: Option<(SnarlNodeId, NetworkBoxId)>,
    pub remove_from_box: Option<SnarlNodeId>,
    pub delete_box: Option<NetworkBoxId>,
}

/// Temporary per-frame viewer that borrows app state.
pub struct GraphViewer<'a> {
    pub graph: &'a mut Graph,
    pub id_map: &'a mut IdMap,
    pub catalog: &'a [CatalogEntry],
    /// Populated during `snarl.show()` via `final_node_rect` callback.
    pub node_rects: &'a mut HashMap<SnarlNodeId, Rect>,
    /// Actions to apply after snarl.show() completes.
    pub actions: &'a mut ViewerActions,
    /// Shape slots reserved during draw_background for network box rects (behind nodes).
    pub box_shape_slots: &'a mut Vec<(NetworkBoxId, ShapeIdx, ShapeIdx)>,
}

impl<'a> SnarlViewer<UiNode> for GraphViewer<'a> {
    fn title(&mut self, node: &UiNode) -> String {
        node.display_name.clone()
    }

    fn show_header(
        &mut self,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<UiNode>,
    ) {
        let ui_node = &snarl[node];
        let title = self.title(ui_node);
        if ui_node.pinned {
            // Show a small colored pin indicator before the title.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let pin_icon = RichText::new("\u{25C6}") // ◆ diamond
                    .color(Color32::from_rgb(255, 200, 60));
                ui.add(Label::new(pin_icon).selectable(false));
                ui.add(Label::new(title).selectable(false));
            });
        } else {
            ui.add(Label::new(title).selectable(false));
        }
    }

    fn inputs(&mut self, node: &UiNode) -> usize {
        self.graph
            .node(node.core_id)
            .map(|n| n.inputs.len())
            .unwrap_or(0)
    }

    fn outputs(&mut self, node: &UiNode) -> usize {
        self.graph
            .node(node.core_id)
            .map(|n| n.outputs.len())
            .unwrap_or(0)
    }

    fn show_input(
        &mut self,
        pin: &InPin,
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<UiNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let Some(ui_node) = snarl.get_node(pin.id.node) else {
            ui.label("?");
            return PinInfo::circle().with_fill(Color32::DARK_GRAY);
        };
        let core_id = ui_node.core_id;
        let idx = pin.id.input;

        if let Some(node_def) = self.graph.node(core_id) {
            if let Some(port) = node_def.inputs.get(idx) {
                ui.label(&port.name);
                let color = data_type_color(port.data_type);
                return PinInfo::circle().with_fill(color);
            }
        }
        ui.label("?");
        PinInfo::circle().with_fill(Color32::DARK_GRAY)
    }

    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<UiNode>,
    ) -> impl egui_snarl::ui::SnarlPin + 'static {
        let Some(ui_node) = snarl.get_node(pin.id.node) else {
            ui.label("?");
            return PinInfo::circle().with_fill(Color32::DARK_GRAY);
        };
        let core_id = ui_node.core_id;
        let idx = pin.id.output;

        if let Some(node_def) = self.graph.node(core_id) {
            if let Some(port) = node_def.outputs.get(idx) {
                ui.label(&port.name);
                let color = data_type_color(port.data_type);
                return PinInfo::circle().with_fill(color);
            }
        }
        ui.label("?");
        PinInfo::circle().with_fill(Color32::DARK_GRAY)
    }

    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<UiNode>) {
        let Some(from_ui) = snarl.get_node(from.id.node) else { return };
        let Some(to_ui) = snarl.get_node(to.id.node) else { return };
        let from_core = from_ui.core_id;
        let to_core = to_ui.core_id;

        let core_from = PortId {
            node: from_core,
            port: PortIndex(from.id.output),
        };
        let core_to = PortId {
            node: to_core,
            port: PortIndex(to.id.input),
        };

        // If input already has a connection, disconnect it first (in both core and snarl).
        if let Some(existing) = self.graph.input_connection(core_to).cloned() {
            self.graph.disconnect(existing.from, existing.to);
            // Also disconnect in snarl: find the snarl node for existing.from.node
            if let Some(snarl_from_node) = self.id_map.core_to_snarl(existing.from.node) {
                let snarl_out = OutPinId {
                    node: snarl_from_node,
                    output: existing.from.port.0,
                };
                snarl.disconnect(snarl_out, to.id);
            }
        }

        // Try connecting in core graph.
        match self.graph.connect(core_from, core_to) {
            Ok(()) => {
                snarl.connect(from.id, to.id);

                // Auto-expand variadic nodes: if all inputs are now connected, add one more.
                if let Some(node) = self.graph.node(to_core) {
                    if node.is_variadic() {
                        let all_connected = (0..node.inputs.len()).all(|i| {
                            let pid = PortId { node: to_core, port: PortIndex(i) };
                            self.graph.input_connection(pid).is_some()
                        });
                        if all_connected {
                            if let Some(node) = self.graph.node_mut(to_core) {
                                node.add_variadic_input();
                            }
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("Connection rejected: {e}");
            }
        }
    }

    fn disconnect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<UiNode>) {
        let Some(from_ui) = snarl.get_node(from.id.node) else { return };
        let Some(to_ui) = snarl.get_node(to.id.node) else { return };
        let to_core = to_ui.core_id;

        let core_from = PortId {
            node: from_ui.core_id,
            port: PortIndex(from.id.output),
        };
        let core_to = PortId {
            node: to_core,
            port: PortIndex(to.id.input),
        };

        self.graph.disconnect(core_from, core_to);
        snarl.disconnect(from.id, to.id);

        // Auto-shrink variadic nodes: keep exactly one empty trailing port.
        if let Some(node) = self.graph.node(to_core) {
            if node.is_variadic() && node.inputs.len() > 2 {
                // Count trailing unconnected ports.
                let mut trailing_empty = 0;
                for i in (0..node.inputs.len()).rev() {
                    let pid = PortId { node: to_core, port: PortIndex(i) };
                    if self.graph.input_connection(pid).is_some() {
                        break;
                    }
                    trailing_empty += 1;
                }
                // Remove all but one trailing empty port (keep at least 2 total).
                if trailing_empty > 1 {
                    let to_remove = trailing_empty - 1;
                    if let Some(node) = self.graph.node_mut(to_core) {
                        for _ in 0..to_remove {
                            if !node.remove_variadic_input() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    fn header_frame(
        &mut self,
        mut frame: Frame,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<UiNode>,
    ) -> Frame {
        // Tint header by category color.
        if let Some(ui_node) = snarl.get_node(node) {
            let c = ui_node.color;
            frame.fill = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 40);

            // If node belongs to a network box, tint the border with the box's fill color.
            if let Some(box_id) = self.graph.node_network_box(ui_node.core_id) {
                if let Some(nb) = self.graph.network_box(box_id) {
                    let bc = nb.fill_color;
                    frame.stroke.color = Color32::from_rgba_unmultiplied(bc[0], bc[1], bc[2], 200);
                    frame.stroke.width = 2.0;
                }
            }
        }
        frame
    }

    fn final_node_rect(
        &mut self,
        node: SnarlNodeId,
        _ui_rect: Rect,
        graph_rect: Rect,
        _ui: &mut Ui,
        _scale: f32,
        _snarl: &mut Snarl<UiNode>,
    ) {
        self.node_rects.insert(node, graph_rect);
    }

    fn draw_background(
        &mut self,
        background: Option<&egui_snarl::ui::BackgroundPattern>,
        viewport: &egui_snarl::ui::Viewport,
        snarl_style: &egui_snarl::ui::SnarlStyle,
        style: &egui::Style,
        painter: &egui::Painter,
        _snarl: &Snarl<UiNode>,
    ) {
        // Draw the default background pattern first.
        if let Some(bg) = background {
            bg.draw(viewport, snarl_style, style, painter);
        }

        // Reserve placeholder shapes for each network box.
        // These slots are in paint order AFTER the background but BEFORE nodes.
        self.box_shape_slots.clear();
        for nb in self.graph.network_boxes() {
            let rect_idx = painter.add(egui::epaint::Shape::Noop);
            let text_idx = painter.add(egui::epaint::Shape::Noop);
            self.box_shape_slots.push((nb.id, rect_idx, text_idx));
        }
    }

    fn has_graph_menu(&mut self, _pos: Pos2, _snarl: &mut Snarl<UiNode>) -> bool {
        true
    }

    fn show_graph_menu(
        &mut self,
        pos: Pos2,
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<UiNode>,
    ) {
        ui.label("Add Node");
        ui.separator();

        // Group catalog by category.
        let categories = [
            NodeCategory::Generators,
            NodeCategory::Transforms,
            NodeCategory::PathOps,
            NodeCategory::Styling,
            NodeCategory::Color,
            NodeCategory::Text,
            NodeCategory::Utility,
            NodeCategory::GraphIO,
        ];

        for cat in categories {
            ui.menu_button(cat.label(), |ui| {
                for entry in self.catalog {
                    if entry.category != cat {
                        continue;
                    }
                    if ui.button(entry.label).clicked() {
                        self.add_node_from_catalog(entry, pos, snarl);
                        ui.close_menu();
                    }
                }
            });
        }

        // Network box deletion: check if right-click is on a box title bar.
        let boxes: Vec<_> = self.graph.network_boxes()
            .map(|nb| (nb.id, nb.title.clone()))
            .collect();
        if !boxes.is_empty() {
            ui.separator();
            ui.menu_button("Delete Network Box", |ui| {
                for (box_id, title) in &boxes {
                    if ui.button(title.as_str()).clicked() {
                        self.actions.delete_box = Some(*box_id);
                        ui.close_menu();
                    }
                }
            });
        }
    }

    fn has_node_menu(&mut self, _node: &UiNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: SnarlNodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        _scale: f32,
        snarl: &mut Snarl<UiNode>,
    ) {
        if let Some(ui_node) = snarl.get_node_mut(node) {
            let pin_label = if ui_node.pinned { "Unpin" } else { "Pin" };
            if ui.button(pin_label).clicked() {
                ui_node.pinned = !ui_node.pinned;
                ui.close_menu();
            }
        }

        if ui.button("Duplicate  Ctrl+D").clicked() {
            self.duplicate_nodes(&[node], snarl);
            ui.close_menu();
        }

        ui.separator();

        // "Create Network Box" — uses the current selection (handled in app).
        if ui.button("Create Network Box").clicked() {
            self.actions.create_box_from = vec![node];
            ui.close_menu();
        }

        // Network box context menu items.
        if let Some(ui_node) = snarl.get_node(node) {
            let core_id = ui_node.core_id;
            let in_box = self.graph.node_network_box(core_id);

            // "Add to Network Box" submenu (only if boxes exist).
            let boxes: Vec<_> = self.graph.network_boxes()
                .map(|nb| (nb.id, nb.title.clone()))
                .collect();
            if !boxes.is_empty() {
                ui.menu_button("Add to Network Box", |ui| {
                    for (box_id, title) in &boxes {
                        if ui.button(title.as_str()).clicked() {
                            self.actions.add_to_box = Some((node, *box_id));
                            ui.close_menu();
                        }
                    }
                });
            }

            // "Remove from Network Box" (only if node is in one).
            if in_box.is_some() && ui.button("Remove from Network Box").clicked() {
                self.actions.remove_from_box = Some(node);
                ui.close_menu();
            }
        }

        ui.separator();

        if ui.button("Delete").clicked() {
            self.remove_node(node, snarl);
            ui.close_menu();
        }
    }
}

impl<'a> GraphViewer<'a> {
    fn add_node_from_catalog(
        &mut self,
        entry: &CatalogEntry,
        pos: Pos2,
        snarl: &mut Snarl<UiNode>,
    ) {
        // Create a temp NodeDef with a placeholder ID, then add_node assigns the real one.
        let node_def = (entry.factory)(vector_flow_core::types::NodeId(0));
        let display_name = node_op_label(&node_def.op).to_string();
        let core_id = self.graph.add_node(node_def);

        // Set position in core graph.
        if let Some(n) = self.graph.node_mut(core_id) {
            n.position = [pos.x, pos.y];
        }

        let ui_node = UiNode {
            core_id,
            display_name,
            color: entry.color,
            pinned: false,
        };
        let snarl_id = snarl.insert_node(pos, ui_node);
        self.id_map.insert(core_id, snarl_id);
    }

    fn remove_node(&mut self, snarl_id: SnarlNodeId, snarl: &mut Snarl<UiNode>) {
        if let Some(core_id) = self.id_map.remove_by_snarl(snarl_id) {
            let _ = self.graph.remove_node(core_id);
        }
        snarl.remove_node(snarl_id);
    }

    /// Duplicate a set of nodes. Internal edges (both endpoints in the set) are
    /// duplicated too. Returns the snarl IDs of the newly created nodes.
    pub fn duplicate_nodes(
        &mut self,
        snarl_ids: &[SnarlNodeId],
        snarl: &mut Snarl<UiNode>,
    ) -> Vec<SnarlNodeId> {
        const OFFSET: f32 = 30.0;

        // Map old core id -> new core id for edge remapping.
        let mut core_map: HashMap<CoreNodeId, CoreNodeId> = HashMap::new();
        // Map old snarl id -> new snarl id.
        let mut snarl_map: HashMap<SnarlNodeId, SnarlNodeId> = HashMap::new();
        let mut new_snarl_ids = Vec::with_capacity(snarl_ids.len());

        // 1. Clone each node.
        for &sid in snarl_ids {
            let Some(ui_node) = snarl.get_node(sid) else { continue };
            let old_core = ui_node.core_id;
            let color = ui_node.color;
            let pinned = ui_node.pinned;
            let display_name = ui_node.display_name.clone();

            let Some(node_def) = self.graph.node(old_core).cloned() else { continue };

            let pos = snarl
                .get_node_info(sid)
                .map(|info| Pos2::new(info.pos.x + OFFSET, info.pos.y + OFFSET))
                .unwrap_or(Pos2::new(OFFSET, OFFSET));

            // Add cloned node to core graph (gets a new ID).
            let new_core = self.graph.add_node(node_def);
            if let Some(n) = self.graph.node_mut(new_core) {
                n.position = [pos.x, pos.y];
            }

            let new_ui = UiNode {
                core_id: new_core,
                display_name,
                color,
                pinned,
            };
            let new_sid = snarl.insert_node(pos, new_ui);
            self.id_map.insert(new_core, new_sid);

            core_map.insert(old_core, new_core);
            snarl_map.insert(sid, new_sid);
            new_snarl_ids.push(new_sid);
        }

        // 2. Duplicate internal edges (both endpoints are in the duplicated set).
        let edges: Vec<_> = self.graph.edges().to_vec();
        for edge in &edges {
            let Some(&new_from_core) = core_map.get(&edge.from.node) else { continue };
            let Some(&new_to_core) = core_map.get(&edge.to.node) else { continue };

            let new_from = PortId { node: new_from_core, port: edge.from.port };
            let new_to = PortId { node: new_to_core, port: edge.to.port };

            if self.graph.connect(new_from, new_to).is_ok() {
                // Mirror in snarl.
                if let (Some(&new_from_sid), Some(&new_to_sid)) = (
                    self.id_map.core_to_snarl(new_from_core).as_ref(),
                    self.id_map.core_to_snarl(new_to_core).as_ref(),
                ) {
                    let out = OutPinId { node: new_from_sid, output: edge.from.port.0 };
                    let inp = InPinId { node: new_to_sid, input: edge.to.port.0 };
                    snarl.connect(out, inp);
                }
            }
        }

        new_snarl_ids
    }
}
