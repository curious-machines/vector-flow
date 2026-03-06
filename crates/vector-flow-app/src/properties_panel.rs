use std::collections::HashMap;

use egui::Ui;

use vector_flow_core::graph::Graph;
use vector_flow_core::node::{NodeOp, ParamValue, PortDef, PortIndex};
use vector_flow_core::types::{DataType, NodeId as CoreNodeId};

use crate::ui_node::node_op_label;

/// DataTypes available for DSL script ports (phase 1: Scalar and Int).
const DSL_PORT_TYPES: &[(DataType, &str)] = &[
    (DataType::Scalar, "Scalar"),
    (DataType::Int, "Int"),
];

/// Show properties inspector for the selected node.
/// Returns `true` if any parameter was changed.
pub fn show_properties_panel(
    ui: &mut Ui,
    graph: &mut Graph,
    selected_core_ids: &[CoreNodeId],
    node_errors: &HashMap<CoreNodeId, String>,
) -> bool {
    let mut changed = false;

    match selected_core_ids.len() {
        0 => {
            ui.label("No selection");
        }
        1 => {
            let core_id = selected_core_ids[0];
            changed = show_node_properties(ui, graph, core_id, node_errors);
        }
        n => {
            ui.label(format!("{n} nodes selected"));
        }
    }

    changed
}

fn show_node_properties(ui: &mut Ui, graph: &mut Graph, core_id: CoreNodeId, node_errors: &HashMap<CoreNodeId, String>) -> bool {
    let Some(node) = graph.node(core_id) else {
        ui.label("Node not found");
        return false;
    };

    let label = node_op_label(&node.op);
    let is_variadic = node.is_variadic();
    let input_count = node.inputs.len();

    // Get DSL info if this is a DSL Code node.
    let dsl_source = match &node.op {
        NodeOp::DslCode { source, .. } => Some(source.clone()),
        _ => None,
    };
    // Snapshot current script port definitions for the editor.
    let dsl_port_info = match &node.op {
        NodeOp::DslCode { script_inputs, script_outputs, .. } => {
            Some((script_inputs.clone(), script_outputs.clone()))
        }
        _ => None,
    };

    // Get portal label if this is a portal node.
    let portal_label = match &node.op {
        NodeOp::PortalSend { label } | NodeOp::PortalReceive { label } => Some(label.clone()),
        _ => None,
    };

    // Get LoadImage path if applicable.
    let image_path = match &node.op {
        NodeOp::LoadImage { path } => Some(path.clone()),
        _ => None,
    };

    // Get ColorParse text if applicable.
    let color_parse_text = match &node.op {
        NodeOp::ColorParse { text } => Some(text.clone()),
        _ => None,
    };

    // Get SvgPath data if applicable.
    let svg_path_data = match &node.op {
        NodeOp::SvgPath { data } => Some(data.clone()),
        _ => None,
    };

    // Get dash pattern if this is a SetStroke or StrokeToPath node.
    let dash_pattern = match &node.op {
        NodeOp::SetStroke { dash_pattern } | NodeOp::StrokeToPath { dash_pattern } => {
            Some(dash_pattern.clone())
        }
        _ => None,
    };

    // Get Text node fields if applicable.
    let text_info = match &node.op {
        NodeOp::Text { text, font_family, font_path } => {
            Some((text.clone(), font_family.clone(), font_path.clone()))
        }
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

    // LoadImage path editor.
    if let Some(mut ipath) = image_path {
        let mut path_changed = false;
        ui.horizontal(|ui| {
            ui.label("Path");
            if ui.text_edit_singleline(&mut ipath).changed() {
                path_changed = true;
            }
            if ui.button("\u{1F4C2}").on_hover_text("Browse...").clicked() {
                if let Some(picked) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                    .add_filter("All files", &["*"])
                    .pick_file()
                {
                    ipath = picked.display().to_string();
                    path_changed = true;
                }
            }
        });
        if path_changed {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::LoadImage { path } = &mut node.op {
                    *path = ipath;
                }
                node.touch();
                changed = true;
            }
        }
        ui.separator();
    }

    // ColorParse text editor.
    if let Some(mut ctext) = color_parse_text {
        let mut text_changed = false;
        ui.horizontal(|ui| {
            ui.label("Color");
            if ui.text_edit_singleline(&mut ctext).changed() {
                text_changed = true;
            }
        });
        if text_changed {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::ColorParse { text } = &mut node.op {
                    *text = ctext;
                }
                node.touch();
                changed = true;
            }
        }
        ui.separator();
    }

    // SvgPath data editor.
    if let Some(mut svg_data) = svg_path_data {
        let mut data_changed = false;
        ui.label("Path Data (SVG d attribute)");
        if ui
            .add(egui::TextEdit::multiline(&mut svg_data).desired_rows(4).code_editor())
            .changed()
        {
            data_changed = true;
        }
        if let Some(err) = vector_flow_compute::validate_svg_path(&svg_data) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
        }
        if data_changed {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::SvgPath { data } = &mut node.op {
                    *data = svg_data;
                }
                node.touch();
                changed = true;
            }
        }
        ui.separator();
    }

    // Dash pattern editor (for SetStroke and StrokeToPath).
    if let Some(mut dpat) = dash_pattern {
        let mut pat_changed = false;
        ui.horizontal(|ui| {
            ui.label("Dash Pattern");
            if ui
                .text_edit_singleline(&mut dpat)
                .on_hover_text("Comma-separated dash/gap lengths, e.g. \"10,5\" or \"10,5,3,5\"")
                .changed()
            {
                pat_changed = true;
            }
        });
        if pat_changed {
            if let Some(node) = graph.node_mut(core_id) {
                match &mut node.op {
                    NodeOp::SetStroke { dash_pattern } => *dash_pattern = dpat,
                    NodeOp::StrokeToPath { dash_pattern } => *dash_pattern = dpat,
                    _ => {}
                }
                node.touch();
                changed = true;
            }
        }
        ui.separator();
    }

    // Text node editors.
    if let Some((mut text_content, mut font_family, mut font_path)) = text_info {
        let mut text_changed = false;
        let mut family_changed = false;
        let mut path_changed = false;

        ui.label("Text");
        if ui
            .add(
                egui::TextEdit::multiline(&mut text_content)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .hint_text("Enter text..."),
            )
            .changed()
        {
            text_changed = true;
        }

        ui.horizontal(|ui| {
            ui.label("Font Family");
            if ui
                .text_edit_singleline(&mut font_family)
                .on_hover_text("System font name, e.g. \"Arial\", \"Noto Sans\" (empty = default)")
                .changed()
            {
                family_changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Font Path");
            if ui.text_edit_singleline(&mut font_path).changed() {
                path_changed = true;
            }
            if ui.button("\u{1F4C2}").on_hover_text("Browse...").clicked() {
                if let Some(picked) = rfd::FileDialog::new()
                    .add_filter("Fonts", &["ttf", "otf", "ttc", "otc"])
                    .add_filter("All files", &["*"])
                    .pick_file()
                {
                    font_path = picked.display().to_string();
                    path_changed = true;
                }
            }
        });

        if text_changed || family_changed || path_changed {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::Text {
                    text: ref mut t,
                    font_family: ref mut ff,
                    font_path: ref mut fp,
                } = node.op
                {
                    if text_changed {
                        *t = text_content;
                    }
                    if family_changed {
                        *ff = font_family;
                    }
                    if path_changed {
                        *fp = font_path;
                    }
                }
                node.touch();
                changed = true;
            }
        }
        ui.separator();
    }

    // DSL source editor.
    if let Some(mut source) = dsl_source {
        ui.label("Expression");
        let response = ui.add(
            egui::TextEdit::multiline(&mut source)
                .desired_width(f32::INFINITY)
                .desired_rows(4)
                .code_editor()
                .hint_text("e.g. sin(time * 3.14)"),
        );
        if response.changed() {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::DslCode { source: ref mut s, .. } = node.op {
                    *s = source;
                }
                node.touch();
                changed = true;
            }
        }
        // Show compile error if any.
        if let Some(err) = node_errors.get(&core_id) {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
        }
        ui.separator();
    }

    // DSL port editor (inputs and outputs).
    if let Some((script_inputs, script_outputs)) = dsl_port_info {
        changed |= show_dsl_port_editor(ui, graph, core_id, "Inputs", &script_inputs, true);
        changed |= show_dsl_port_editor(ui, graph, core_id, "Outputs", &script_outputs, false);
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

/// Show port editor for DSL node inputs or outputs.
/// Returns true if anything changed.
fn show_dsl_port_editor(
    ui: &mut Ui,
    graph: &mut Graph,
    core_id: CoreNodeId,
    section_label: &str,
    ports: &[(String, DataType)],
    is_input: bool,
) -> bool {
    let mut changed = false;
    let mut remove_idx: Option<usize> = None;

    ui.strong(section_label);

    for (i, (name, dt)) in ports.iter().enumerate() {
        let mut port_name = name.clone();
        let mut port_type = *dt;

        ui.horizontal(|ui| {
            // Name field.
            let name_resp = ui.add(
                egui::TextEdit::singleline(&mut port_name)
                    .desired_width(80.0)
                    .hint_text("name"),
            );

            // Type dropdown.
            let current_label = DSL_PORT_TYPES
                .iter()
                .find(|(t, _)| *t == port_type)
                .map(|(_, l)| *l)
                .unwrap_or("Scalar");

            egui::ComboBox::from_id_salt(format!("dsl_{section_label}_{i}"))
                .selected_text(current_label)
                .width(60.0)
                .show_ui(ui, |ui| {
                    for &(t, label) in DSL_PORT_TYPES {
                        if ui.selectable_value(&mut port_type, t, label).changed() {
                            changed = true;
                        }
                    }
                });

            // Delete button.
            if ui.small_button("\u{2715}").clicked() {
                remove_idx = Some(i);
            }

            // Check if name changed.
            if name_resp.changed() {
                changed = true;
            }
        });

        // Apply name/type change if needed.
        if port_name != *name || port_type != *dt {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::DslCode { script_inputs, script_outputs, .. } = &mut node.op {
                    let list = if is_input { script_inputs } else { script_outputs };
                    if let Some(entry) = list.get_mut(i) {
                        entry.0 = port_name.clone();
                        entry.1 = port_type;
                    }
                }
                // Sync NodeDef ports.
                let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
                if let Some(port) = port_list.get_mut(i) {
                    port.name = port_name;
                    port.data_type = port_type;
                }
                node.touch();
                changed = true;
            }
        }
    }

    // Handle deletion.
    if let Some(idx) = remove_idx {
        if let Some(node) = graph.node_mut(core_id) {
            if let NodeOp::DslCode { script_inputs, script_outputs, .. } = &mut node.op {
                let list = if is_input { script_inputs } else { script_outputs };
                if idx < list.len() {
                    list.remove(idx);
                }
            }
            let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
            if idx < port_list.len() {
                // Disconnect any edges to this port before removing.
                if is_input {
                    graph.disconnect_input_port(core_id, PortIndex(idx));
                }
                if let Some(node) = graph.node_mut(core_id) {
                    let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
                    port_list.remove(idx);
                }
            }
            if let Some(node) = graph.node_mut(core_id) {
                node.touch();
            }
            changed = true;
        }
    }

    // Add button.
    if ui.small_button(format!("+ Add {}", if is_input { "Input" } else { "Output" })).clicked() {
        let default_name = if is_input {
            format!("in{}", ports.len() + 1)
        } else {
            format!("out{}", ports.len() + 1)
        };
        if let Some(node) = graph.node_mut(core_id) {
            if let NodeOp::DslCode { script_inputs, script_outputs, .. } = &mut node.op {
                let list = if is_input { script_inputs } else { script_outputs };
                list.push((default_name.clone(), DataType::Scalar));
            }
            let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
            port_list.push(PortDef::new(default_name, DataType::Scalar));
            node.touch();
            changed = true;
        }
    }

    changed
}
