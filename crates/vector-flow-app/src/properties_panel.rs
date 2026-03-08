use std::collections::HashMap;

use egui::Ui;

use vector_flow_core::graph::Graph;
use vector_flow_core::node::{NodeOp, ParamValue, PortDef, PortId, PortIndex};
use vector_flow_core::scheduler::EvalResult;
use vector_flow_core::types::{DataType, NodeData, NodeId as CoreNodeId};

use crate::project::ProjectSettings;
use crate::ui_node::node_op_label;

/// DataTypes available for DSL script ports.
const DSL_PORT_TYPES: &[(DataType, &str)] = &[
    (DataType::Scalar, "Scalar"),
    (DataType::Int, "Int"),
    (DataType::Color, "Color"),
];

/// Show properties inspector for the selected node.
/// Returns `true` if any parameter was changed.
pub fn show_properties_panel(
    ui: &mut Ui,
    graph: &mut Graph,
    selected_core_ids: &[CoreNodeId],
    node_errors: &HashMap<CoreNodeId, String>,
    project_settings: &mut ProjectSettings,
    eval_result: Option<&EvalResult>,
) -> bool {
    let mut changed = false;

    match selected_core_ids.len() {
        0 => {
            show_project_settings(ui, project_settings);
        }
        1 => {
            let core_id = selected_core_ids[0];
            changed = show_node_properties(ui, graph, core_id, node_errors, eval_result);
        }
        n => {
            ui.label(format!("{n} nodes selected"));
        }
    }

    changed
}

fn show_project_settings(ui: &mut Ui, settings: &mut ProjectSettings) {
    ui.heading("Project Settings");
    ui.add_space(4.0);

    egui::Grid::new("project_settings_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Canvas Width:");
            let mut w = settings.canvas_width as i64;
            if ui.add(egui::DragValue::new(&mut w).range(1..=8192)).changed() {
                settings.canvas_width = w.max(1) as u32;
            }
            ui.end_row();

            ui.label("Canvas Height:");
            let mut h = settings.canvas_height as i64;
            if ui.add(egui::DragValue::new(&mut h).range(1..=8192)).changed() {
                settings.canvas_height = h.max(1) as u32;
            }
            ui.end_row();

            ui.label("Background:");
            ui.horizontal(|ui| {
                let has_color = settings.background_color.is_some();
                let mut use_color = has_color;
                if ui.checkbox(&mut use_color, "").changed() {
                    if use_color {
                        settings.background_color = Some([1.0, 1.0, 1.0, 1.0]);
                    } else {
                        settings.background_color = None;
                    }
                }
                if let Some(ref mut color) = settings.background_color {
                    let mut rgba = egui::Color32::from_rgba_unmultiplied(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                        (color[3] * 255.0) as u8,
                    );
                    if ui.color_edit_button_srgba(&mut rgba).changed() {
                        color[0] = rgba.r() as f32 / 255.0;
                        color[1] = rgba.g() as f32 / 255.0;
                        color[2] = rgba.b() as f32 / 255.0;
                        color[3] = rgba.a() as f32 / 255.0;
                    }
                } else {
                    ui.label("None (transparent)");
                }
            });
            ui.end_row();
        });
}

fn show_node_properties(ui: &mut Ui, graph: &mut Graph, core_id: CoreNodeId, node_errors: &HashMap<CoreNodeId, String>, eval_result: Option<&EvalResult>) -> bool {
    let Some(node) = graph.node(core_id) else {
        ui.label("Node not found");
        return false;
    };

    let label = node_op_label(&node.op);
    let is_variadic = node.is_variadic();
    let input_count = node.inputs.len();

    // Get DSL info if this is a DSL Code or Map node.
    let dsl_source = match &node.op {
        NodeOp::DslCode { source, .. } | NodeOp::Map { source, .. } | NodeOp::Generate { source, .. } => Some(source.clone()),
        _ => None,
    };
    // Snapshot current script port definitions for the editor.
    let dsl_port_info = match &node.op {
        NodeOp::DslCode { script_inputs, script_outputs, .. }
        | NodeOp::Map { script_inputs, script_outputs, .. }
        | NodeOp::Generate { script_inputs, script_outputs, .. } => {
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

    // Get PathBoolean operation if applicable.
    let boolean_op = match &node.op {
        NodeOp::PathBoolean { operation } => Some(*operation),
        _ => None,
    };

    // Get Merge keep_separate flag if applicable.
    let merge_keep_separate = match &node.op {
        NodeOp::Merge { keep_separate } => Some(*keep_separate),
        _ => None,
    };

    // Get Text node fields if applicable.
    let text_info = match &node.op {
        NodeOp::Text { text, font_family, font_path } => {
            Some((text.clone(), font_family.clone(), font_path.clone()))
        }
        _ => None,
    };

    // Get GraphOutput order if applicable.
    let graph_output_order = match &node.op {
        NodeOp::GraphOutput { order, .. } => Some(*order),
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

    // Collect color output port info for display.
    let color_outputs: Vec<_> = node
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.data_type == DataType::Color)
        .map(|(i, p)| (i, p.name.clone()))
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

    // GraphOutput order editor.
    if let Some(mut ord) = graph_output_order {
        ui.horizontal(|ui| {
            ui.label("Order");
            if ui.add(egui::DragValue::new(&mut ord)).on_hover_text("Render order (lower draws first)").changed() {
                if let Some(node) = graph.node_mut(core_id) {
                    if let NodeOp::GraphOutput { order, .. } = &mut node.op {
                        *order = ord;
                    }
                    node.touch();
                    changed = true;
                }
            }
        });
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

    // Path Boolean operation selector.
    if let Some(mut bool_op) = boolean_op {
        let labels = ["Union", "Intersect", "Difference", "Xor"];
        let current_label = labels.get(bool_op as usize).unwrap_or(&"Union");
        ui.horizontal(|ui| {
            ui.label("Operation");
            egui::ComboBox::from_id_salt("path_boolean_op")
                .selected_text(*current_label)
                .width(100.0)
                .show_ui(ui, |ui| {
                    for (i, label) in labels.iter().enumerate() {
                        if ui.selectable_label(bool_op == i as i32, *label).clicked() {
                            bool_op = i as i32;
                        }
                    }
                });
        });
        if boolean_op != Some(bool_op) {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::PathBoolean { operation } = &mut node.op {
                    *operation = bool_op;
                    node.touch();
                    changed = true;
                }
            }
        }
        ui.separator();
    }

    // Merge keep_separate toggle.
    if let Some(mut ks) = merge_keep_separate {
        if ui.checkbox(&mut ks, "Keep Separate")
            .on_hover_text("When enabled, paths are promoted to shapes so each input stays as a distinct batch element instead of merging contours")
            .changed()
        {
            if let Some(node) = graph.node_mut(core_id) {
                if let NodeOp::Merge { keep_separate } = &mut node.op {
                    *keep_separate = ks;
                    node.touch();
                    changed = true;
                }
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
                match &mut node.op {
                    NodeOp::DslCode { source: ref mut s, .. }
                    | NodeOp::Map { source: ref mut s, .. }
                    | NodeOp::Generate { source: ref mut s, .. } => {
                        *s = source;
                    }
                    _ => {}
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
        let has_builtins = matches!(&graph.node(core_id).map(|n| &n.op), Some(NodeOp::Map { .. } | NodeOp::Generate { .. }));
        changed |= show_dsl_port_editor(ui, graph, core_id, "Inputs", &script_inputs, true, has_builtins);
        changed |= show_dsl_port_editor(ui, graph, core_id, "Outputs", &script_outputs, false, false);
        ui.separator();
    }

    for (idx, name, data_type, default_value) in port_info {
        // For connected color ports, show a read-only swatch of the actual computed value.
        let port_id = PortId { node: core_id, port: PortIndex(idx) };
        if data_type == DataType::Color {
            if let Some(edge) = graph.input_connection(port_id) {
                if let Some(color) = eval_result
                    .and_then(|er| er.outputs.get(&edge.from.node))
                    .and_then(|outputs| outputs.get(edge.from.port.0))
                {
                    match color {
                        NodeData::Color(c) => {
                            let rgba = [c.r, c.g, c.b, c.a];
                            show_connected_color(ui, &name, &rgba);
                        }
                        NodeData::Colors(colors) => {
                            if let Some(c) = colors.first() {
                                let rgba = [c.r, c.g, c.b, c.a];
                                let batch_name = format!("{name} ({} colors)", colors.len());
                                show_connected_color(ui, &batch_name, &rgba);
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
        }
        let Some(param) = default_value else { continue };
        if let Some(new_val) = show_param_editor(ui, &name, data_type, &param) {
            if let Some(node) = graph.node_mut(core_id) {
                node.inputs[idx].default_value = Some(new_val);
                node.touch();
                changed = true;
            }
        }
    }

    // Show computed color outputs.
    if !color_outputs.is_empty() {
        if let Some(outputs) = eval_result.and_then(|er| er.outputs.get(&core_id)) {
            for (idx, name) in &color_outputs {
                if let Some(NodeData::Color(c)) = outputs.get(*idx) {
                    let rgba = [c.r, c.g, c.b, c.a];
                    let output_label = format!("{name} (out)");
                    show_connected_color(ui, &output_label, &rgba);
                }
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

/// Show a read-only color swatch for a connected color input port.
/// Uses egui's built-in color button for correct sRGB handling.
fn show_connected_color(ui: &mut Ui, name: &str, rgba: &[f32; 4]) {
    ui.horizontal(|ui| {
        ui.label(name);
        let mut val = *rgba;
        ui.color_edit_button_rgba_unmultiplied(&mut val);
    });
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

/// For Map input ports, these names are built-in (populated from the batch iteration).
/// They don't have corresponding graph input ports.
const MAP_BUILTIN_INPUTS: &[&str] = &["element", "index", "count"];

/// Check if a Map script input is a built-in (element/index/count).
fn is_map_builtin_input(name: &str) -> bool {
    MAP_BUILTIN_INPUTS.contains(&name)
}

const GENERATE_BUILTIN_INPUTS: &[&str] = &["index", "count"];

/// Check if a Generate script input is a built-in (index/count).
fn is_generate_builtin_input(name: &str) -> bool {
    GENERATE_BUILTIN_INPUTS.contains(&name)
}

/// For a Map node, compute the graph port index for script input at position `script_idx`.
/// Built-in inputs (element/index/count) return None. Others map to graph ports 1, 2, 3, ...
fn map_script_input_to_graph_port(ports: &[(String, DataType)], script_idx: usize) -> Option<usize> {
    if is_map_builtin_input(&ports[script_idx].0) {
        return None;
    }
    // Count non-builtin inputs before this index to determine graph port offset.
    let extra_idx = ports[..script_idx].iter()
        .filter(|(n, _)| !is_map_builtin_input(n))
        .count();
    Some(1 + extra_idx) // graph port 0 is "batch"
}

/// For a Generate node, compute the graph port index for script input at position `script_idx`.
/// Built-in inputs (index/count) return None. Others map to graph ports 2, 3, 4, ...
fn generate_script_input_to_graph_port(ports: &[(String, DataType)], script_idx: usize) -> Option<usize> {
    if is_generate_builtin_input(&ports[script_idx].0) {
        return None;
    }
    let extra_idx = ports[..script_idx].iter()
        .filter(|(n, _)| !is_generate_builtin_input(n))
        .count();
    Some(2 + extra_idx) // graph ports 0,1 are start/end
}

/// Compute the graph port index for a script input, taking into account
/// the node type (Map has builtins element/index/count; Generate has index/count).
/// For plain DslCode nodes or outputs, returns Some(script_idx) directly.
fn script_input_to_graph_port(op: &NodeOp, ports: &[(String, DataType)], script_idx: usize) -> Option<usize> {
    match op {
        NodeOp::Map { .. } => map_script_input_to_graph_port(ports, script_idx),
        NodeOp::Generate { .. } => generate_script_input_to_graph_port(ports, script_idx),
        _ => Some(script_idx),
    }
}

/// Show port editor for DSL node inputs or outputs.
/// `has_builtins`: true when editing Map/Generate node's script inputs (special handling for builtins).
/// Returns true if anything changed.
fn show_dsl_port_editor(
    ui: &mut Ui,
    graph: &mut Graph,
    core_id: CoreNodeId,
    section_label: &str,
    ports: &[(String, DataType)],
    is_input: bool,
    has_builtins: bool,
) -> bool {
    let mut changed = false;
    let mut remove_idx: Option<usize> = None;

    // Snapshot the op kind for builtin checks.
    let op_snapshot = graph.node(core_id).map(|n| n.op.clone());

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

            // Delete button — can't delete built-in script variables.
            let is_non_deletable = has_builtins && op_snapshot.as_ref()
                .map(|op| match op {
                    NodeOp::Map { .. } => is_map_builtin_input(name),
                    _ => false,
                })
                .unwrap_or(false);
            if !is_non_deletable && ui.small_button("\u{2715}").clicked() {
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
                let op_list = match &mut node.op {
                    NodeOp::DslCode { script_inputs, script_outputs, .. }
                    | NodeOp::Map { script_inputs, script_outputs, .. }
                    | NodeOp::Generate { script_inputs, script_outputs, .. } => {
                        Some(if is_input { script_inputs } else { script_outputs })
                    }
                    _ => None,
                };
                if let Some(list) = op_list {
                    if let Some(entry) = list.get_mut(i) {
                        entry.0 = port_name.clone();
                        entry.1 = port_type;
                    }
                }
                // Sync NodeDef graph ports.
                if has_builtins {
                    if let Some(ref op) = op_snapshot {
                        if let Some(gp) = script_input_to_graph_port(op, ports, i) {
                            if let Some(port) = node.inputs.get_mut(gp) {
                                port.name = port_name;
                                port.data_type = port_type;
                            }
                        }
                    }
                } else {
                    let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
                    if let Some(port) = port_list.get_mut(i) {
                        port.name = port_name;
                        port.data_type = port_type;
                    }
                }
                node.touch();
                changed = true;
            }
        }
    }

    // Handle deletion.
    if let Some(idx) = remove_idx {
        let graph_port_idx = if has_builtins {
            op_snapshot.as_ref().and_then(|op| script_input_to_graph_port(op, ports, idx))
        } else {
            Some(idx)
        };

        if let Some(node) = graph.node_mut(core_id) {
            {
                let op_list = match &mut node.op {
                    NodeOp::DslCode { script_inputs, script_outputs, .. }
                    | NodeOp::Map { script_inputs, script_outputs, .. }
                    | NodeOp::Generate { script_inputs, script_outputs, .. } => {
                        Some(if is_input { script_inputs } else { script_outputs })
                    }
                    _ => None,
                };
                if let Some(list) = op_list {
                    if idx < list.len() {
                        list.remove(idx);
                    }
                }
            }
            // Remove the corresponding graph port (if any).
            if let Some(gp) = graph_port_idx {
                let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
                if gp < port_list.len() {
                    if is_input {
                        graph.disconnect_input_port(core_id, PortIndex(gp));
                    }
                    if let Some(node) = graph.node_mut(core_id) {
                        let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
                        port_list.remove(gp);
                    }
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
            {
                let op_list = match &mut node.op {
                    NodeOp::DslCode { script_inputs, script_outputs, .. }
                    | NodeOp::Map { script_inputs, script_outputs, .. }
                    | NodeOp::Generate { script_inputs, script_outputs, .. } => {
                        Some(if is_input { script_inputs } else { script_outputs })
                    }
                    _ => None,
                };
                if let Some(list) = op_list {
                    list.push((default_name.clone(), DataType::Scalar));
                }
            }
            let port_list = if is_input { &mut node.inputs } else { &mut node.outputs };
            port_list.push(PortDef::new(default_name, DataType::Scalar));
            node.touch();
            changed = true;
        }
    }

    changed
}
