use egui::Ui;

use vector_flow_core::graph::Graph;
use vector_flow_core::node::{NodeOp, ParamValue, PortIndex};
use vector_flow_core::types::{DataType, NodeId as CoreNodeId};

use crate::ui_node::node_op_label;

/// Show properties inspector for the selected node.
/// Returns `true` if any parameter was changed.
pub fn show_properties_panel(
    ui: &mut Ui,
    graph: &mut Graph,
    selected_core_ids: &[CoreNodeId],
) -> bool {
    let mut changed = false;

    match selected_core_ids.len() {
        0 => {
            ui.label("No selection");
        }
        1 => {
            let core_id = selected_core_ids[0];
            changed = show_node_properties(ui, graph, core_id);
        }
        n => {
            ui.label(format!("{n} nodes selected"));
        }
    }

    changed
}

fn show_node_properties(ui: &mut Ui, graph: &mut Graph, core_id: CoreNodeId) -> bool {
    let Some(node) = graph.node(core_id) else {
        ui.label("Node not found");
        return false;
    };

    let label = node_op_label(&node.op);
    let is_variadic = node.is_variadic();
    let input_count = node.inputs.len();

    // Get portal label if this is a portal node.
    let portal_label = match &node.op {
        NodeOp::PortalSend { label } | NodeOp::PortalReceive { label } => Some(label.clone()),
        _ => None,
    };

    ui.heading(label);
    ui.separator();

    // Collect port info we need for editing (avoid borrow issues).
    let port_info: Vec<_> = node
        .inputs
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.name.clone(), p.data_type, p.default_value.clone()))
        .collect();

    let mut changed = false;

    // Portal label editor.
    if let Some(mut plabel) = portal_label {
        let mut label_changed = false;
        ui.horizontal(|ui| {
            ui.label("Label");
            if ui.text_edit_singleline(&mut plabel).changed() {
                label_changed = true;
            }
        });
        if label_changed {
            if let Some(node) = graph.node_mut(core_id) {
                match &mut node.op {
                    NodeOp::PortalSend { label } => {
                        *label = plabel.clone();
                        node.name = format!("Send: {plabel}");
                    }
                    NodeOp::PortalReceive { label } => {
                        *label = plabel.clone();
                        node.name = format!("Receive: {plabel}");
                    }
                    _ => {}
                }
                node.touch();
                changed = true;
            }
        }
        ui.separator();
    }

    for (idx, name, data_type, default_value) in port_info {
        let Some(param) = default_value else { continue };
        if let Some(new_val) = show_param_editor(ui, &name, data_type, &param) {
            if let Some(node) = graph.node_mut(core_id) {
                node.inputs[idx].default_value = Some(new_val);
                node.touch();
                changed = true;
            }
        }
    }

    // Variadic input controls.
    if is_variadic {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!("{input_count} inputs"));
            if ui.button("+").clicked() {
                if let Some(node) = graph.node_mut(core_id) {
                    node.add_variadic_input();
                    changed = true;
                }
            }
            if input_count > 2 && ui.button("\u{2212}").clicked() {
                // Disconnect any edge to the last port before removing it.
                let last_port = PortIndex(input_count - 1);
                graph.disconnect_input_port(core_id, last_port);
                if let Some(node) = graph.node_mut(core_id) {
                    node.remove_variadic_input();
                    changed = true;
                }
            }
        });
    }

    changed
}

fn show_param_editor(
    ui: &mut Ui,
    name: &str,
    data_type: DataType,
    value: &ParamValue,
) -> Option<ParamValue> {
    match (data_type, value) {
        (DataType::Scalar, ParamValue::Float(v)) => {
            let mut val = *v;
            ui.horizontal(|ui| {
                ui.label(name);
                ui.add(egui::DragValue::new(&mut val).speed(0.1));
            });
            if val != *v {
                return Some(ParamValue::Float(val));
            }
        }
        (DataType::Int, ParamValue::Int(v)) => {
            let mut val = *v;
            ui.horizontal(|ui| {
                ui.label(name);
                ui.add(egui::DragValue::new(&mut val).speed(1.0));
            });
            if val != *v {
                return Some(ParamValue::Int(val));
            }
        }
        (DataType::Bool, ParamValue::Bool(v)) => {
            let mut val = *v;
            ui.horizontal(|ui| {
                ui.label(name);
                ui.checkbox(&mut val, "");
            });
            if val != *v {
                return Some(ParamValue::Bool(val));
            }
        }
        (DataType::Vec2, ParamValue::Vec2(v)) => {
            let mut val = *v;
            ui.horizontal(|ui| {
                ui.label(name);
                ui.add(egui::DragValue::new(&mut val[0]).speed(0.1).prefix("x: "));
                ui.add(egui::DragValue::new(&mut val[1]).speed(0.1).prefix("y: "));
            });
            if val != *v {
                return Some(ParamValue::Vec2(val));
            }
        }
        (DataType::Color, ParamValue::Color(v)) => {
            let mut val = *v;
            ui.horizontal(|ui| {
                ui.label(name);
                ui.color_edit_button_rgba_unmultiplied(&mut val);
            });
            if val != *v {
                return Some(ParamValue::Color(val));
            }
        }
        // Scalars used as Scalar params (e.g. angle in degrees).
        (_, ParamValue::Float(v)) => {
            let mut val = *v;
            ui.horizontal(|ui| {
                ui.label(name);
                ui.add(egui::DragValue::new(&mut val).speed(0.1));
            });
            if val != *v {
                return Some(ParamValue::Float(val));
            }
        }
        (_, ParamValue::Int(v)) => {
            let mut val = *v;
            ui.horizontal(|ui| {
                ui.label(name);
                ui.add(egui::DragValue::new(&mut val).speed(1.0));
            });
            if val != *v {
                return Some(ParamValue::Int(val));
            }
        }
        _ => {
            ui.horizontal(|ui| {
                ui.label(name);
                ui.label("(no editor)");
            });
        }
    }
    None
}
