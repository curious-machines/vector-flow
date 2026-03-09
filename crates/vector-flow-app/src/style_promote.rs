//! Promotion and demotion between SetFill, SetStroke, and SetStyle nodes.
//!
//! Promotion replaces a simpler styling node with the combined SetStyle node.
//! Demotion replaces SetStyle with a simpler node (or a chained pair).
//! Values and connections are preserved where port semantics match.

use egui_snarl::{NodeId as SnarlNodeId, Snarl};

use vector_flow_core::graph::Graph;
use vector_flow_core::node::{NodeDef, NodeOp, ParamValue, PortId, PortIndex};
use vector_flow_core::types::NodeId as CoreNodeId;

use crate::id_map::IdMap;
use crate::ui_node::UiNode;

/// What kind of style promotion/demotion to perform.
pub enum StyleConversion {
    /// SetFill → SetStyle
    FillToStyle,
    /// SetStroke → SetStyle
    StrokeToStyle,
    /// SetStyle → SetFill (discards stroke settings)
    StyleToFill,
    /// SetStyle → SetStroke (discards fill settings)
    StyleToStroke,
    /// SetStyle → SetFill + SetStroke chain
    StyleToFillAndStroke,
    /// Chained SetFill + SetStroke → SetStyle (SetFill feeds SetStroke or vice versa)
    ChainToStyle {
        /// The snarl ID of the other node in the chain.
        other_snarl_id: SnarlNodeId,
    },
}

/// Port index mapping from source node type to SetStyle.
/// Each entry is (source_port_index, style_port_index).
fn fill_to_style_port_map() -> &'static [(usize, usize)] {
    &[
        (0, 0),  // geometry → path
        (1, 1),  // color → fill_color
    ]
}

fn stroke_to_style_port_map() -> &'static [(usize, usize)] {
    &[
        (0, 0),   // geometry → path
        (1, 4),   // color → stroke_color
        (2, 5),   // width → stroke_width
        (3, 8),   // cap → cap
        (4, 9),   // join → join
        (5, 10),  // miter_limit → miter_limit
        (6, 11),  // dash_offset → dash_offset
        // port 7 (tolerance) has no equivalent in SetStyle
    ]
}

fn style_to_fill_port_map() -> &'static [(usize, usize)] {
    &[
        (0, 0),  // path → geometry
        (1, 1),  // fill_color → color
    ]
}

fn style_to_stroke_port_map() -> &'static [(usize, usize)] {
    &[
        (0, 0),   // path → geometry
        (4, 1),   // stroke_color → color
        (5, 2),   // stroke_width → width
        (8, 3),   // cap → cap
        (9, 4),   // join → join
        (10, 5),  // miter_limit → miter_limit
        (11, 6),  // dash_offset → dash_offset
    ]
}

/// Result of a style conversion. Contains a status message for the status bar.
pub struct ConversionResult {
    pub message: String,
}

/// Perform a simple promotion/demotion that replaces a single node.
///
/// Returns `None` if the source node doesn't exist or the conversion is invalid.
pub fn convert_style_node(
    snarl_id: SnarlNodeId,
    conversion: StyleConversion,
    graph: &mut Graph,
    snarl: &mut Snarl<UiNode>,
    id_map: &mut IdMap,
) -> Option<ConversionResult> {
    match conversion {
        StyleConversion::FillToStyle => promote_single(snarl_id, true, graph, snarl, id_map),
        StyleConversion::StrokeToStyle => promote_single(snarl_id, false, graph, snarl, id_map),
        StyleConversion::StyleToFill => demote_single(snarl_id, true, graph, snarl, id_map),
        StyleConversion::StyleToStroke => demote_single(snarl_id, false, graph, snarl, id_map),
        StyleConversion::StyleToFillAndStroke => demote_to_chain(snarl_id, graph, snarl, id_map),
        StyleConversion::ChainToStyle { other_snarl_id } => {
            promote_chain(snarl_id, other_snarl_id, graph, snarl, id_map)
        }
    }
}

/// Promote SetFill or SetStroke to SetStyle.
fn promote_single(
    snarl_id: SnarlNodeId,
    is_fill: bool,
    graph: &mut Graph,
    snarl: &mut Snarl<UiNode>,
    _id_map: &mut IdMap,
) -> Option<ConversionResult> {
    let ui_node = snarl.get_node(snarl_id)?;
    let core_id = ui_node.core_id;
    let old_node = graph.node(core_id)?.clone();

    // Validate source node type.
    match &old_node.op {
        NodeOp::SetFill if is_fill => {}
        NodeOp::SetStroke { .. } if !is_fill => {}
        _ => return None,
    }

    // Build the new SetStyle node.
    let mut new_node = NodeDef::set_style(core_id);
    new_node.position = old_node.position;

    // Transfer port values.
    let port_map = if is_fill {
        fill_to_style_port_map()
    } else {
        stroke_to_style_port_map()
    };
    transfer_port_values(&old_node, &mut new_node, port_map);

    // Disable the "other" style pass so promotion doesn't add unexpected styling.
    if is_fill {
        // Promoting from SetFill: disable stroke (has_stroke = port 7).
        new_node.inputs[7].default_value = Some(ParamValue::Bool(false));
    } else {
        // Promoting from SetStroke: disable fill (has_fill = port 3).
        new_node.inputs[3].default_value = Some(ParamValue::Bool(false));
    }

    // Transfer dash_pattern from SetStroke.
    if let NodeOp::SetStroke { dash_pattern } = &old_node.op {
        if let NodeOp::SetStyle { dash_pattern: ref mut dp } = new_node.op {
            *dp = dash_pattern.clone();
        }
    }

    // Initialize visibility.
    new_node.init_visibility();

    // Make any ports visible that had connections or were visible on the source.
    propagate_visibility(&old_node, &mut new_node, port_map);

    // Rewire connections and replace the node.
    replace_node_in_graph(core_id, new_node, port_map, graph);

    // Update snarl UI node.
    if let Some(ui) = snarl.get_node_mut(snarl_id) {
        ui.display_name = "Set Style".to_string();
    }

    let source_name = if is_fill { "Set Fill" } else { "Set Stroke" };
    Some(ConversionResult {
        message: format!("Promoted {source_name} to Set Style"),
    })
}

/// Demote SetStyle to SetFill or SetStroke.
fn demote_single(
    snarl_id: SnarlNodeId,
    to_fill: bool,
    graph: &mut Graph,
    snarl: &mut Snarl<UiNode>,
    id_map: &mut IdMap,
) -> Option<ConversionResult> {
    let _ = id_map; // unused for single replacement
    let ui_node = snarl.get_node(snarl_id)?;
    let core_id = ui_node.core_id;
    let old_node = graph.node(core_id)?.clone();

    // Validate source node type.
    if !matches!(&old_node.op, NodeOp::SetStyle { .. }) {
        return None;
    }

    // Check if we're discarding non-default values.
    let discarded = check_discarded_values(&old_node, to_fill);

    // Build the new node.
    let mut new_node = if to_fill {
        NodeDef::set_fill(core_id)
    } else {
        NodeDef::set_stroke(core_id)
    };
    new_node.position = old_node.position;

    let port_map = if to_fill {
        style_to_fill_port_map()
    } else {
        style_to_stroke_port_map()
    };
    transfer_port_values(&old_node, &mut new_node, port_map);

    // Transfer dash_pattern to SetStroke.
    if !to_fill {
        if let NodeOp::SetStyle { dash_pattern } = &old_node.op {
            if let NodeOp::SetStroke { dash_pattern: ref mut dp } = new_node.op {
                *dp = dash_pattern.clone();
            }
        }
    }

    new_node.init_visibility();
    propagate_visibility(&old_node, &mut new_node, port_map);
    replace_node_in_graph(core_id, new_node, port_map, graph);

    // Update snarl UI node.
    let target_name = if to_fill { "Set Fill" } else { "Set Stroke" };
    if let Some(ui) = snarl.get_node_mut(snarl_id) {
        ui.display_name = target_name.to_string();
    }

    let message = if let Some(what) = discarded {
        format!("Demoted to {target_name} — {what} settings discarded")
    } else {
        format!("Demoted to {target_name}")
    };
    Some(ConversionResult { message })
}

/// Demote SetStyle to a SetFill + SetStroke chain.
fn demote_to_chain(
    snarl_id: SnarlNodeId,
    graph: &mut Graph,
    snarl: &mut Snarl<UiNode>,
    id_map: &mut IdMap,
) -> Option<ConversionResult> {
    let ui_node = snarl.get_node(snarl_id)?;
    let core_id = ui_node.core_id;
    let old_node = graph.node(core_id)?.clone();
    let pos = snarl.get_node_info(snarl_id)?.pos;

    if !matches!(&old_node.op, NodeOp::SetStyle { .. }) {
        return None;
    }

    // Collect old connections before modifying.
    let old_input_connections: Vec<_> = (0..old_node.inputs.len())
        .filter_map(|i| {
            let pid = PortId { node: core_id, port: PortIndex(i) };
            graph.input_connection(pid).map(|e| (i, e.from))
        })
        .collect();
    let old_output_connections: Vec<_> = graph.edges()
        .iter()
        .filter(|e| e.from.node == core_id)
        .map(|e| (e.from.port.0, e.to))
        .collect();

    // We'll keep the original node as SetFill (reuse its ID and position)
    // and create a new SetStroke node downstream.
    let mut fill_node = NodeDef::set_fill(core_id);
    fill_node.position = old_node.position;

    // Transfer fill values: path → geometry, fill_color → color.
    transfer_port_values(&old_node, &mut fill_node, style_to_fill_port_map());
    fill_node.init_visibility();

    // Create stroke node.
    let mut stroke_def = NodeDef::set_stroke(CoreNodeId(0));
    // Position it to the right of the fill node.
    stroke_def.position = [old_node.position[0] + 200.0, old_node.position[1]];

    // Transfer stroke values.
    transfer_port_values(&old_node, &mut stroke_def, style_to_stroke_port_map());
    if let NodeOp::SetStyle { dash_pattern } = &old_node.op {
        if let NodeOp::SetStroke { dash_pattern: ref mut dp } = stroke_def.op {
            *dp = dash_pattern.clone();
        }
    }
    stroke_def.init_visibility();

    // Remove old connections first.
    for &(_, from) in &old_input_connections {
        let to = PortId { node: core_id, port: PortIndex(0) }; // will be cleaned up
        graph.disconnect(from, to);
    }
    // Disconnect all edges touching this node.
    disconnect_all(core_id, graph);

    // Replace the node in the graph with the fill node.
    if let Some(n) = graph.node_mut(core_id) {
        *n = fill_node;
        n.id = core_id;
    }

    // Add stroke node to graph.
    let stroke_id = graph.add_node(stroke_def);
    if let Some(n) = graph.node_mut(stroke_id) {
        n.position = [old_node.position[0] + 200.0, old_node.position[1]];
    }

    // Add stroke node to snarl.
    let stroke_snarl_pos = egui::Pos2::new(pos.x + 200.0, pos.y);
    let stroke_ui = UiNode {
        core_id: stroke_id,
        display_name: "Set Stroke".to_string(),
        color: crate::ui_node::category_color(crate::ui_node::NodeCategory::Styling),
        pinned: false,
    };
    let stroke_snarl_id = snarl.insert_node(stroke_snarl_pos, stroke_ui);
    id_map.insert(stroke_id, stroke_snarl_id);

    // Update fill node in snarl.
    if let Some(ui) = snarl.get_node_mut(snarl_id) {
        ui.display_name = "Set Fill".to_string();
    }

    // Rewire: upstream → fill input (port 0 = geometry).
    // Find the connection that was on the path/geometry input (port 0).
    for &(old_port, from) in &old_input_connections {
        if old_port == 0 {
            // geometry input → fill geometry input
            let to = PortId { node: core_id, port: PortIndex(0) };
            let _ = graph.connect(from, to);
            // Also connect in snarl.
            if let Some(from_snarl) = id_map.core_to_snarl(from.node) {
                if let Some(from_node) = graph.node(from.node) {
                    if let Some(vis_out) = from_node.port_to_visible_output(from.port.0) {
                        snarl.connect(
                            egui_snarl::OutPinId { node: from_snarl, output: vis_out },
                            egui_snarl::InPinId { node: snarl_id, input: 0 },
                        );
                    }
                }
            }
        }
        // Fill-specific connections (fill_color = port 1 → fill color = port 1).
        if old_port == 1 {
            let to = PortId { node: core_id, port: PortIndex(1) };
            let _ = graph.connect(from, to);
            connect_in_snarl(from, to, graph, snarl, id_map);
        }
        // Stroke-specific connections.
        for &(style_port, stroke_port) in style_to_stroke_port_map() {
            if old_port == style_port && style_port != 0 {
                let to = PortId { node: stroke_id, port: PortIndex(stroke_port) };
                let _ = graph.connect(from, to);
                connect_in_snarl(from, to, graph, snarl, id_map);
            }
        }
    }

    // Wire fill output → stroke geometry input.
    let fill_out = PortId { node: core_id, port: PortIndex(0) };
    let stroke_in = PortId { node: stroke_id, port: PortIndex(0) };
    let _ = graph.connect(fill_out, stroke_in);
    // Snarl connection: fill visible output 0 → stroke visible input 0.
    snarl.connect(
        egui_snarl::OutPinId { node: snarl_id, output: 0 },
        egui_snarl::InPinId { node: stroke_snarl_id, input: 0 },
    );

    // Wire stroke output → downstream (whatever was connected to SetStyle output).
    for &(old_out_port, to) in &old_output_connections {
        if old_out_port == 0 {
            let from = PortId { node: stroke_id, port: PortIndex(0) };
            let _ = graph.connect(from, to);
            connect_in_snarl(from, to, graph, snarl, id_map);
        }
    }

    // Copy network box membership.
    if let Some(box_id) = old_node_box_membership(&old_node, core_id, graph) {
        graph.add_node_to_box(stroke_id, box_id);
    }

    Some(ConversionResult {
        message: "Demoted to Set Fill + Set Stroke chain".to_string(),
    })
}

/// Promote a chained SetFill + SetStroke (in either order) to SetStyle.
fn promote_chain(
    snarl_id: SnarlNodeId,
    other_snarl_id: SnarlNodeId,
    graph: &mut Graph,
    snarl: &mut Snarl<UiNode>,
    id_map: &mut IdMap,
) -> Option<ConversionResult> {
    let ui_a = snarl.get_node(snarl_id)?;
    let ui_b = snarl.get_node(other_snarl_id)?;
    let core_a = ui_a.core_id;
    let core_b = ui_b.core_id;

    let node_a = graph.node(core_a)?.clone();
    let node_b = graph.node(core_b)?.clone();

    // Determine which is fill and which is stroke, and which is upstream.
    let (fill_core, stroke_core, fill_node, stroke_node, fill_is_upstream) =
        match (&node_a.op, &node_b.op) {
            (NodeOp::SetFill, NodeOp::SetStroke { .. }) => {
                // Check if A feeds B.
                let a_feeds_b = graph.edges().iter().any(|e| {
                    e.from.node == core_a && e.to.node == core_b && e.to.port.0 == 0
                });
                if a_feeds_b {
                    (core_a, core_b, node_a, node_b, true)
                } else {
                    (core_a, core_b, node_a, node_b, false)
                }
            }
            (NodeOp::SetStroke { .. }, NodeOp::SetFill) => {
                let b_feeds_a = graph.edges().iter().any(|e| {
                    e.from.node == core_b && e.to.node == core_a && e.to.port.0 == 0
                });
                if b_feeds_a {
                    (core_b, core_a, node_b, node_a, true)
                } else {
                    (core_b, core_a, node_b, node_a, false)
                }
            }
            _ => return None,
        };

    // The upstream node's geometry input connects to SetStyle's path input.
    // The downstream node's output connections move to SetStyle's output.
    let (upstream_core, downstream_core) = if fill_is_upstream {
        (fill_core, stroke_core)
    } else {
        (stroke_core, fill_core)
    };

    // Collect external connections (skip the internal fill↔stroke edge).
    let upstream_input_connections: Vec<_> = graph.edges()
        .iter()
        .filter(|e| e.to.node == upstream_core && e.from.node != downstream_core)
        .map(|e| (e.to.port.0, e.from))
        .collect();

    // Also collect upstream-specific port connections (fill or stroke params, not geometry).
    let downstream_param_connections: Vec<_> = graph.edges()
        .iter()
        .filter(|e| e.to.node == downstream_core && e.from.node != upstream_core)
        .map(|e| (e.to.port.0, e.from))
        .collect();

    let downstream_output_connections: Vec<_> = graph.edges()
        .iter()
        .filter(|e| e.from.node == downstream_core)
        .map(|e| (e.from.port.0, e.to))
        .collect();

    // We'll keep the upstream node and replace it with SetStyle,
    // then remove the downstream node.
    let keep_core = upstream_core;
    let remove_core = downstream_core;
    let keep_snarl = if upstream_core == core_a { snarl_id } else { other_snarl_id };
    let remove_snarl = if upstream_core == core_a { other_snarl_id } else { snarl_id };

    // Build SetStyle node.
    let mut style_node = NodeDef::set_style(keep_core);
    let upstream_node = graph.node(upstream_core)?.clone();
    style_node.position = upstream_node.position;

    // Transfer values from both nodes.
    transfer_port_values(&fill_node, &mut style_node, fill_to_style_port_map());
    transfer_port_values(&stroke_node, &mut style_node, stroke_to_style_port_map());

    // Transfer dash_pattern.
    if let NodeOp::SetStroke { dash_pattern } = &stroke_node.op {
        if let NodeOp::SetStyle { dash_pattern: ref mut dp } = style_node.op {
            *dp = dash_pattern.clone();
        }
    }

    style_node.init_visibility();

    // Disconnect everything.
    disconnect_all(upstream_core, graph);
    disconnect_all(downstream_core, graph);

    // Replace the kept node.
    if let Some(n) = graph.node_mut(keep_core) {
        *n = style_node;
        n.id = keep_core;
    }

    // Remove the other node.
    let _ = graph.remove_node(remove_core);
    id_map.remove_by_core(remove_core);
    snarl.remove_node(remove_snarl);

    // Update snarl display.
    if let Some(ui) = snarl.get_node_mut(keep_snarl) {
        ui.display_name = "Set Style".to_string();
    }

    // Rewire upstream geometry input.
    for &(port, from) in &upstream_input_connections {
        // Map port through the correct port map.
        let port_map = if upstream_core == fill_core {
            fill_to_style_port_map()
        } else {
            stroke_to_style_port_map()
        };
        if let Some(&(_, style_port)) = port_map.iter().find(|&&(src, _)| src == port) {
            let to = PortId { node: keep_core, port: PortIndex(style_port) };
            let _ = graph.connect(from, to);
            connect_in_snarl(from, to, graph, snarl, id_map);
        }
    }

    // Rewire downstream param connections.
    for &(port, from) in &downstream_param_connections {
        let port_map = if downstream_core == fill_core {
            fill_to_style_port_map()
        } else {
            stroke_to_style_port_map()
        };
        if let Some(&(_, style_port)) = port_map.iter().find(|&&(src, _)| src == port) {
            let to = PortId { node: keep_core, port: PortIndex(style_port) };
            let _ = graph.connect(from, to);
            connect_in_snarl(from, to, graph, snarl, id_map);
        }
    }

    // Rewire downstream output → SetStyle output.
    for &(_, to) in &downstream_output_connections {
        let from = PortId { node: keep_core, port: PortIndex(0) };
        let _ = graph.connect(from, to);
        connect_in_snarl(from, to, graph, snarl, id_map);
    }

    Some(ConversionResult {
        message: "Promoted Set Fill + Set Stroke chain to Set Style".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Transfer port values (default_value, expression) between nodes using a port map.
fn transfer_port_values(
    src: &NodeDef,
    dst: &mut NodeDef,
    port_map: &[(usize, usize)],
) {
    for &(src_port, dst_port) in port_map {
        if let (Some(src_pd), Some(dst_pd)) = (src.inputs.get(src_port), dst.inputs.get_mut(dst_port)) {
            if src_pd.default_value != dst_pd.default_value {
                dst_pd.default_value = src_pd.default_value.clone();
            }
            if src_pd.expression.is_some() {
                dst_pd.expression = src_pd.expression.clone();
            }
        }
    }
}

/// Propagate visibility: if a source port was visible, make the corresponding dest port visible.
fn propagate_visibility(
    src: &NodeDef,
    dst: &mut NodeDef,
    port_map: &[(usize, usize)],
) {
    for &(src_port, dst_port) in port_map {
        if src_port < src.input_visibility.len()
            && dst_port < dst.input_visibility.len()
            && src.input_visibility[src_port]
        {
            dst.input_visibility[dst_port] = true;
        }
    }
}

/// Replace a node's definition in the graph and rewire existing connections
/// through the port map. Connections to unmapped ports are dropped.
fn replace_node_in_graph(
    core_id: CoreNodeId,
    mut new_node: NodeDef,
    port_map: &[(usize, usize)],
    graph: &mut Graph,
) {
    // Collect existing connections.
    let input_connections: Vec<_> = graph.edges()
        .iter()
        .filter(|e| e.to.node == core_id)
        .map(|e| (e.to.port.0, e.from))
        .collect();
    let output_connections: Vec<_> = graph.edges()
        .iter()
        .filter(|e| e.from.node == core_id)
        .map(|e| (e.from.port.0, e.to))
        .collect();

    // Disconnect all edges touching this node.
    disconnect_all(core_id, graph);

    // Replace the node definition.
    new_node.id = core_id;
    if let Some(n) = graph.node_mut(core_id) {
        *n = new_node;
    }

    // Rewire input connections through the port map.
    for (old_port, from) in input_connections {
        if let Some(&(_, new_port)) = port_map.iter().find(|&&(src, _)| src == old_port) {
            let to = PortId { node: core_id, port: PortIndex(new_port) };
            let _ = graph.connect(from, to);
        }
    }

    // Rewire output connections (output port 0 → output port 0 for all styling nodes).
    for (old_port, to) in output_connections {
        // All styling nodes have a single output at index 0.
        if old_port == 0 {
            let from = PortId { node: core_id, port: PortIndex(0) };
            let _ = graph.connect(from, to);
        }
    }
}

/// Disconnect all edges touching a node.
fn disconnect_all(core_id: CoreNodeId, graph: &mut Graph) {
    let edges: Vec<_> = graph.edges()
        .iter()
        .filter(|e| e.from.node == core_id || e.to.node == core_id)
        .cloned()
        .collect();
    for e in edges {
        graph.disconnect(e.from, e.to);
    }
}

/// Connect two ports in snarl, mapping core port indices to visible indices.
fn connect_in_snarl(
    from: PortId,
    to: PortId,
    graph: &Graph,
    snarl: &mut Snarl<UiNode>,
    id_map: &IdMap,
) {
    let Some(from_snarl) = id_map.core_to_snarl(from.node) else { return };
    let Some(to_snarl) = id_map.core_to_snarl(to.node) else { return };

    let Some(from_node) = graph.node(from.node) else { return };
    let Some(to_node) = graph.node(to.node) else { return };

    // Ensure ports are visible before connecting in snarl.
    let Some(vis_out) = from_node.port_to_visible_output(from.port.0) else { return };
    let Some(vis_in) = to_node.port_to_visible_input(to.port.0) else { return };

    snarl.connect(
        egui_snarl::OutPinId { node: from_snarl, output: vis_out },
        egui_snarl::InPinId { node: to_snarl, input: vis_in },
    );
}

/// Check what non-default values would be discarded by demotion.
fn check_discarded_values(node: &NodeDef, to_fill: bool) -> Option<&'static str> {
    // Build a fresh SetStyle to compare defaults against.
    let defaults = NodeDef::set_style(CoreNodeId(0));

    if to_fill {
        // Demoting to fill — check if any stroke port (4..=11) has a non-default value or expression.
        let has_stroke_customization = (4..=11).any(|i| {
            let has_custom_value = node.inputs.get(i).is_some_and(|p| {
                p.default_value != defaults.inputs.get(i).and_then(|d| d.default_value.clone())
            });
            let has_expr = node.inputs.get(i).is_some_and(|p| p.expression.is_some());
            has_custom_value || has_expr
        });
        if has_stroke_customization {
            Some("stroke")
        } else {
            None
        }
    } else {
        // Demoting to stroke — check if any fill port (1..=3) has a non-default value or expression.
        let has_fill_customization = (1..=3).any(|i| {
            let has_custom_value = node.inputs.get(i).is_some_and(|p| {
                p.default_value != defaults.inputs.get(i).and_then(|d| d.default_value.clone())
            });
            let has_expr = node.inputs.get(i).is_some_and(|p| p.expression.is_some());
            has_custom_value || has_expr
        });
        if has_fill_customization {
            Some("fill")
        } else {
            None
        }
    }
}

/// Check if the node is in a network box.
fn old_node_box_membership(
    _old_node: &NodeDef,
    core_id: CoreNodeId,
    graph: &Graph,
) -> Option<vector_flow_core::types::NetworkBoxId> {
    graph.node_network_box(core_id)
}

/// Determine what conversions are available for a given node.
pub fn available_conversions(
    snarl_id: SnarlNodeId,
    graph: &Graph,
    snarl: &Snarl<UiNode>,
    id_map: &IdMap,
) -> Vec<(String, StyleConversion)> {
    let Some(ui_node) = snarl.get_node(snarl_id) else { return vec![] };
    let core_id = ui_node.core_id;
    let Some(node) = graph.node(core_id) else { return vec![] };

    let mut results = Vec::new();

    match &node.op {
        NodeOp::SetFill => {
            results.push(("Promote to Set Style".to_string(), StyleConversion::FillToStyle));

            // Check if there's a chained SetStroke connected.
            if let Some(chain_snarl) = find_chain_partner(core_id, graph, snarl, id_map, true) {
                results.push((
                    "Promote Fill + Stroke to Set Style".to_string(),
                    StyleConversion::ChainToStyle { other_snarl_id: chain_snarl },
                ));
            }
        }
        NodeOp::SetStroke { .. } => {
            results.push(("Promote to Set Style".to_string(), StyleConversion::StrokeToStyle));

            // Check if there's a chained SetFill connected.
            if let Some(chain_snarl) = find_chain_partner(core_id, graph, snarl, id_map, false) {
                results.push((
                    "Promote Fill + Stroke to Set Style".to_string(),
                    StyleConversion::ChainToStyle { other_snarl_id: chain_snarl },
                ));
            }
        }
        NodeOp::SetStyle { .. } => {
            results.push(("Demote to Set Fill".to_string(), StyleConversion::StyleToFill));
            results.push(("Demote to Set Stroke".to_string(), StyleConversion::StyleToStroke));
            results.push(("Demote to Set Fill + Set Stroke".to_string(), StyleConversion::StyleToFillAndStroke));
        }
        _ => {}
    }

    results
}

/// Find a chain partner: a SetFill/SetStroke node directly connected via geometry ports.
fn find_chain_partner(
    core_id: CoreNodeId,
    graph: &Graph,
    _snarl: &Snarl<UiNode>,
    id_map: &IdMap,
    this_is_fill: bool,
) -> Option<SnarlNodeId> {
    let target_op_check: fn(&NodeOp) -> bool = if this_is_fill {
        |op| matches!(op, NodeOp::SetStroke { .. })
    } else {
        |op| matches!(op, NodeOp::SetFill)
    };

    // Check downstream: does our output feed the geometry input (port 0) of a partner?
    for edge in graph.edges() {
        if edge.from.node == core_id && edge.from.port.0 == 0 {
            if let Some(node) = graph.node(edge.to.node) {
                if edge.to.port.0 == 0 && target_op_check(&node.op) {
                    return id_map.core_to_snarl(edge.to.node);
                }
            }
        }
    }

    // Check upstream: does a partner's output feed our geometry input (port 0)?
    for edge in graph.edges() {
        if edge.to.node == core_id && edge.to.port.0 == 0 {
            if let Some(node) = graph.node(edge.from.node) {
                if edge.from.port.0 == 0 && target_op_check(&node.op) {
                    return id_map.core_to_snarl(edge.from.node);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::graph::Graph;
    use vector_flow_core::node::{NodeDef, NodeOp, ParamValue, PortId, PortIndex};
    use vector_flow_core::types::NodeId as CoreNodeId;

    /// Helper: set up a graph with a generator → styling node → output chain.
    /// Returns (graph, snarl, id_map, generator_core_id, style_snarl_id, style_core_id, output_core_id).
    fn setup_chain_with_node(
        make_style: fn(CoreNodeId) -> NodeDef,
    ) -> (Graph, Snarl<UiNode>, IdMap, CoreNodeId, SnarlNodeId, CoreNodeId, CoreNodeId) {
        let mut graph = Graph::new();
        let mut snarl = Snarl::new();
        let mut id_map = IdMap::new();

        // Generator node (e.g., circle).
        let gen_def = NodeDef::circle(CoreNodeId(0));
        let gen_id = graph.add_node(gen_def);
        let gen_snarl = snarl.insert_node(egui::Pos2::new(0.0, 0.0), UiNode {
            core_id: gen_id,
            display_name: "Circle".into(),
            color: egui::Color32::WHITE,
            pinned: false,
        });
        id_map.insert(gen_id, gen_snarl);

        // Style node.
        let style_def = make_style(CoreNodeId(0));
        let style_id = graph.add_node(style_def);
        let style_snarl = snarl.insert_node(egui::Pos2::new(200.0, 0.0), UiNode {
            core_id: style_id,
            display_name: "Style".into(),
            color: egui::Color32::WHITE,
            pinned: false,
        });
        id_map.insert(style_id, style_snarl);

        // Output node.
        let out_def = NodeDef::graph_output(CoreNodeId(0), "output".into(), vector_flow_core::types::DataType::Any);
        let out_id = graph.add_node(out_def);
        let out_snarl = snarl.insert_node(egui::Pos2::new(400.0, 0.0), UiNode {
            core_id: out_id,
            display_name: "Output".into(),
            color: egui::Color32::WHITE,
            pinned: false,
        });
        id_map.insert(out_id, out_snarl);

        // Connect: generator output 0 → style input 0 (geometry/path).
        let from = PortId { node: gen_id, port: PortIndex(0) };
        let to = PortId { node: style_id, port: PortIndex(0) };
        graph.connect(from, to).unwrap();

        // Connect: style output 0 → output input 0.
        let from = PortId { node: style_id, port: PortIndex(0) };
        let to = PortId { node: out_id, port: PortIndex(0) };
        graph.connect(from, to).unwrap();

        (graph, snarl, id_map, gen_id, style_snarl, style_id, out_id)
    }

    #[test]
    fn test_fill_to_style_preserves_connections() {
        let (mut graph, mut snarl, mut id_map, gen_id, style_snarl, style_id, out_id) =
            setup_chain_with_node(NodeDef::set_fill);

        // Set a custom fill color.
        if let Some(n) = graph.node_mut(style_id) {
            n.inputs[1].default_value = Some(ParamValue::Color([1.0, 0.0, 0.0, 1.0]));
        }

        let result = convert_style_node(
            style_snarl,
            StyleConversion::FillToStyle,
            &mut graph,
            &mut snarl,
            &mut id_map,
        );
        assert!(result.is_some());
        assert!(result.unwrap().message.contains("Promoted"));

        // Node should now be SetStyle.
        let node = graph.node(style_id).unwrap();
        assert!(matches!(node.op, NodeOp::SetStyle { .. }));

        // Fill color should be preserved at port 1 (fill_color in SetStyle).
        assert_eq!(
            node.inputs[1].default_value,
            Some(ParamValue::Color([1.0, 0.0, 0.0, 1.0]))
        );

        // has_stroke (port 7) should be disabled — promoting from fill shouldn't add stroke.
        assert_eq!(node.inputs[7].default_value, Some(ParamValue::Bool(false)));
        // has_fill (port 3) should remain true.
        assert_eq!(node.inputs[3].default_value, Some(ParamValue::Bool(true)));

        // Upstream connection: generator → style path input (port 0).
        let edge = graph.input_connection(PortId { node: style_id, port: PortIndex(0) });
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().from.node, gen_id);

        // Downstream connection: style output → output input.
        let edge = graph.input_connection(PortId { node: out_id, port: PortIndex(0) });
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().from.node, style_id);
    }

    #[test]
    fn test_stroke_to_style_preserves_values() {
        let (mut graph, mut snarl, mut id_map, _gen_id, style_snarl, style_id, _out_id) =
            setup_chain_with_node(NodeDef::set_stroke);

        // Set custom stroke values.
        if let Some(n) = graph.node_mut(style_id) {
            n.inputs[1].default_value = Some(ParamValue::Color([0.0, 1.0, 0.0, 1.0])); // stroke color
            n.inputs[2].default_value = Some(ParamValue::Float(5.0)); // stroke width
            if let NodeOp::SetStroke { ref mut dash_pattern } = n.op {
                *dash_pattern = "10,5".to_string();
            }
        }

        let result = convert_style_node(
            style_snarl,
            StyleConversion::StrokeToStyle,
            &mut graph,
            &mut snarl,
            &mut id_map,
        );
        assert!(result.is_some());

        let node = graph.node(style_id).unwrap();
        assert!(matches!(node.op, NodeOp::SetStyle { .. }));

        // Stroke color → SetStyle port 4 (stroke_color).
        assert_eq!(
            node.inputs[4].default_value,
            Some(ParamValue::Color([0.0, 1.0, 0.0, 1.0]))
        );
        // Stroke width → SetStyle port 5.
        assert_eq!(
            node.inputs[5].default_value,
            Some(ParamValue::Float(5.0))
        );
        // Dash pattern should be transferred.
        if let NodeOp::SetStyle { ref dash_pattern } = node.op {
            assert_eq!(dash_pattern, "10,5");
        } else {
            panic!("Expected SetStyle");
        }

        // has_fill (port 3) should be disabled — promoting from stroke shouldn't add fill.
        assert_eq!(node.inputs[3].default_value, Some(ParamValue::Bool(false)));
        // has_stroke (port 7) should remain true.
        assert_eq!(node.inputs[7].default_value, Some(ParamValue::Bool(true)));
    }

    #[test]
    fn test_style_to_fill_demote() {
        let (mut graph, mut snarl, mut id_map, _gen_id, style_snarl, style_id, out_id) =
            setup_chain_with_node(NodeDef::set_style);

        // Set custom fill color on SetStyle.
        if let Some(n) = graph.node_mut(style_id) {
            n.inputs[1].default_value = Some(ParamValue::Color([0.0, 0.0, 1.0, 1.0]));
        }

        let result = convert_style_node(
            style_snarl,
            StyleConversion::StyleToFill,
            &mut graph,
            &mut snarl,
            &mut id_map,
        );
        assert!(result.is_some());
        assert!(result.unwrap().message.contains("Set Fill"));

        let node = graph.node(style_id).unwrap();
        assert!(matches!(node.op, NodeOp::SetFill));

        // Fill color preserved at port 1.
        assert_eq!(
            node.inputs[1].default_value,
            Some(ParamValue::Color([0.0, 0.0, 1.0, 1.0]))
        );

        // Connections preserved.
        assert!(graph.input_connection(PortId { node: style_id, port: PortIndex(0) }).is_some());
        assert!(graph.input_connection(PortId { node: out_id, port: PortIndex(0) }).is_some());
    }

    #[test]
    fn test_style_to_stroke_demote() {
        let (mut graph, mut snarl, mut id_map, _gen_id, style_snarl, style_id, _out_id) =
            setup_chain_with_node(NodeDef::set_style);

        // Set custom stroke color.
        if let Some(n) = graph.node_mut(style_id) {
            n.inputs[4].default_value = Some(ParamValue::Color([1.0, 1.0, 0.0, 1.0]));
            n.inputs[5].default_value = Some(ParamValue::Float(3.0));
            if let NodeOp::SetStyle { ref mut dash_pattern } = n.op {
                *dash_pattern = "5,3".to_string();
            }
        }

        let result = convert_style_node(
            style_snarl,
            StyleConversion::StyleToStroke,
            &mut graph,
            &mut snarl,
            &mut id_map,
        );
        assert!(result.is_some());

        let node = graph.node(style_id).unwrap();
        assert!(matches!(node.op, NodeOp::SetStroke { .. }));

        // Stroke color → port 1 in SetStroke.
        assert_eq!(
            node.inputs[1].default_value,
            Some(ParamValue::Color([1.0, 1.0, 0.0, 1.0]))
        );
        // Stroke width → port 2.
        assert_eq!(
            node.inputs[2].default_value,
            Some(ParamValue::Float(3.0))
        );
        // Dash pattern.
        if let NodeOp::SetStroke { ref dash_pattern } = node.op {
            assert_eq!(dash_pattern, "5,3");
        }
    }

    #[test]
    fn test_style_to_fill_and_stroke_chain() {
        let (mut graph, mut snarl, mut id_map, gen_id, style_snarl, style_id, out_id) =
            setup_chain_with_node(NodeDef::set_style);

        // Set custom values.
        if let Some(n) = graph.node_mut(style_id) {
            n.inputs[1].default_value = Some(ParamValue::Color([1.0, 0.0, 0.0, 1.0])); // fill color
            n.inputs[4].default_value = Some(ParamValue::Color([0.0, 1.0, 0.0, 1.0])); // stroke color
            n.inputs[5].default_value = Some(ParamValue::Float(4.0)); // stroke width
        }

        let result = convert_style_node(
            style_snarl,
            StyleConversion::StyleToFillAndStroke,
            &mut graph,
            &mut snarl,
            &mut id_map,
        );
        assert!(result.is_some());
        assert!(result.unwrap().message.contains("chain"));

        // Original node should now be SetFill.
        let fill_node = graph.node(style_id).unwrap();
        assert!(matches!(fill_node.op, NodeOp::SetFill));
        assert_eq!(
            fill_node.inputs[1].default_value,
            Some(ParamValue::Color([1.0, 0.0, 0.0, 1.0]))
        );

        // Generator → fill input 0 should be connected.
        let edge = graph.input_connection(PortId { node: style_id, port: PortIndex(0) });
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().from.node, gen_id);

        // Find the new stroke node.
        let stroke_id = graph.nodes().find(|n| {
            n.id != style_id && matches!(n.op, NodeOp::SetStroke { .. })
        }).map(|n| n.id);
        assert!(stroke_id.is_some());
        let stroke_id = stroke_id.unwrap();

        let stroke_node = graph.node(stroke_id).unwrap();
        assert_eq!(
            stroke_node.inputs[1].default_value,
            Some(ParamValue::Color([0.0, 1.0, 0.0, 1.0]))
        );
        assert_eq!(
            stroke_node.inputs[2].default_value,
            Some(ParamValue::Float(4.0))
        );

        // Fill output → stroke input 0 (geometry).
        let edge = graph.input_connection(PortId { node: stroke_id, port: PortIndex(0) });
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().from.node, style_id);

        // Stroke output → graph output input 0.
        let edge = graph.input_connection(PortId { node: out_id, port: PortIndex(0) });
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().from.node, stroke_id);
    }

    #[test]
    fn test_chain_to_style_promotion() {
        let mut graph = Graph::new();
        let mut snarl = Snarl::new();
        let mut id_map = IdMap::new();

        // Generator.
        let gen_id = graph.add_node(NodeDef::circle(CoreNodeId(0)));
        let gen_snarl = snarl.insert_node(egui::Pos2::new(0.0, 0.0), UiNode {
            core_id: gen_id, display_name: "Circle".into(),
            color: egui::Color32::WHITE, pinned: false,
        });
        id_map.insert(gen_id, gen_snarl);

        // SetFill.
        let fill_id = graph.add_node(NodeDef::set_fill(CoreNodeId(0)));
        if let Some(n) = graph.node_mut(fill_id) {
            n.inputs[1].default_value = Some(ParamValue::Color([1.0, 0.0, 0.0, 1.0]));
        }
        let fill_snarl = snarl.insert_node(egui::Pos2::new(200.0, 0.0), UiNode {
            core_id: fill_id, display_name: "Set Fill".into(),
            color: egui::Color32::WHITE, pinned: false,
        });
        id_map.insert(fill_id, fill_snarl);

        // SetStroke.
        let stroke_id = graph.add_node(NodeDef::set_stroke(CoreNodeId(0)));
        if let Some(n) = graph.node_mut(stroke_id) {
            n.inputs[1].default_value = Some(ParamValue::Color([0.0, 0.0, 1.0, 1.0]));
            n.inputs[2].default_value = Some(ParamValue::Float(3.0));
        }
        let stroke_snarl = snarl.insert_node(egui::Pos2::new(400.0, 0.0), UiNode {
            core_id: stroke_id, display_name: "Set Stroke".into(),
            color: egui::Color32::WHITE, pinned: false,
        });
        id_map.insert(stroke_id, stroke_snarl);

        // Output.
        let out_id = graph.add_node(NodeDef::graph_output(CoreNodeId(0), "out".into(), vector_flow_core::types::DataType::Any));
        let out_snarl = snarl.insert_node(egui::Pos2::new(600.0, 0.0), UiNode {
            core_id: out_id, display_name: "Output".into(),
            color: egui::Color32::WHITE, pinned: false,
        });
        id_map.insert(out_id, out_snarl);

        // Wire: gen → fill → stroke → output.
        graph.connect(
            PortId { node: gen_id, port: PortIndex(0) },
            PortId { node: fill_id, port: PortIndex(0) },
        ).unwrap();
        graph.connect(
            PortId { node: fill_id, port: PortIndex(0) },
            PortId { node: stroke_id, port: PortIndex(0) },
        ).unwrap();
        graph.connect(
            PortId { node: stroke_id, port: PortIndex(0) },
            PortId { node: out_id, port: PortIndex(0) },
        ).unwrap();

        // Promote chain from the fill node's perspective.
        let result = convert_style_node(
            fill_snarl,
            StyleConversion::ChainToStyle { other_snarl_id: stroke_snarl },
            &mut graph,
            &mut snarl,
            &mut id_map,
        );
        assert!(result.is_some());
        assert!(result.unwrap().message.contains("chain"));

        // The fill node (upstream) should now be SetStyle.
        let node = graph.node(fill_id).unwrap();
        assert!(matches!(node.op, NodeOp::SetStyle { .. }));

        // Fill color at port 1 (fill_color in SetStyle).
        assert_eq!(
            node.inputs[1].default_value,
            Some(ParamValue::Color([1.0, 0.0, 0.0, 1.0]))
        );
        // Stroke color at port 4 (stroke_color in SetStyle).
        assert_eq!(
            node.inputs[4].default_value,
            Some(ParamValue::Color([0.0, 0.0, 1.0, 1.0]))
        );
        // Stroke width at port 5.
        assert_eq!(
            node.inputs[5].default_value,
            Some(ParamValue::Float(3.0))
        );

        // Stroke node should be removed.
        assert!(graph.node(stroke_id).is_none());

        // Generator → SetStyle path input.
        let edge = graph.input_connection(PortId { node: fill_id, port: PortIndex(0) });
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().from.node, gen_id);

        // SetStyle output → graph output.
        let edge = graph.input_connection(PortId { node: out_id, port: PortIndex(0) });
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().from.node, fill_id);
    }

    #[test]
    fn test_available_conversions_fill() {
        let (graph, snarl, id_map, _, style_snarl, _, _) =
            setup_chain_with_node(NodeDef::set_fill);
        let conversions = available_conversions(style_snarl, &graph, &snarl, &id_map);
        assert_eq!(conversions.len(), 1);
        assert!(conversions[0].0.contains("Set Style"));
    }

    #[test]
    fn test_available_conversions_style() {
        let (graph, snarl, id_map, _, style_snarl, _, _) =
            setup_chain_with_node(NodeDef::set_style);
        let conversions = available_conversions(style_snarl, &graph, &snarl, &id_map);
        assert_eq!(conversions.len(), 3);
        // Should have: Demote to Fill, Demote to Stroke, Demote to Fill + Stroke.
        let labels: Vec<&str> = conversions.iter().map(|(l, _)| l.as_str()).collect();
        assert!(labels.iter().any(|l| l.contains("Set Fill")));
        assert!(labels.iter().any(|l| l.contains("Set Stroke")));
        assert!(labels.iter().any(|l| l.contains("Set Fill + Set Stroke")));
    }

    #[test]
    fn test_discard_warning_on_demote() {
        let (mut graph, mut snarl, mut id_map, _, style_snarl, style_id, _) =
            setup_chain_with_node(NodeDef::set_style);

        // Set non-default stroke values.
        if let Some(n) = graph.node_mut(style_id) {
            n.inputs[5].default_value = Some(ParamValue::Float(10.0)); // stroke_width
        }

        // Demote to fill — should warn about discarded stroke settings.
        let result = convert_style_node(
            style_snarl,
            StyleConversion::StyleToFill,
            &mut graph,
            &mut snarl,
            &mut id_map,
        );
        assert!(result.is_some());
        assert!(result.unwrap().message.contains("stroke settings discarded"));
    }

    #[test]
    fn test_chain_detection() {
        let mut graph = Graph::new();
        let mut snarl = Snarl::new();
        let mut id_map = IdMap::new();

        // SetFill → SetStroke chain.
        let fill_id = graph.add_node(NodeDef::set_fill(CoreNodeId(0)));
        let fill_snarl = snarl.insert_node(egui::Pos2::ZERO, UiNode {
            core_id: fill_id, display_name: "Set Fill".into(),
            color: egui::Color32::WHITE, pinned: false,
        });
        id_map.insert(fill_id, fill_snarl);

        let stroke_id = graph.add_node(NodeDef::set_stroke(CoreNodeId(0)));
        let stroke_snarl = snarl.insert_node(egui::Pos2::new(200.0, 0.0), UiNode {
            core_id: stroke_id, display_name: "Set Stroke".into(),
            color: egui::Color32::WHITE, pinned: false,
        });
        id_map.insert(stroke_id, stroke_snarl);

        // Wire fill → stroke (geometry).
        graph.connect(
            PortId { node: fill_id, port: PortIndex(0) },
            PortId { node: stroke_id, port: PortIndex(0) },
        ).unwrap();

        // Fill should see chain promotion option.
        let conversions = available_conversions(fill_snarl, &graph, &snarl, &id_map);
        assert!(conversions.iter().any(|(l, _)| l.contains("Fill + Stroke")));

        // Stroke should also see chain promotion option.
        let conversions = available_conversions(stroke_snarl, &graph, &snarl, &id_map);
        assert!(conversions.iter().any(|(l, _)| l.contains("Fill + Stroke")));
    }

    #[test]
    fn test_roundtrip_fill_to_style_to_fill() {
        let (mut graph, mut snarl, mut id_map, gen_id, style_snarl, style_id, out_id) =
            setup_chain_with_node(NodeDef::set_fill);

        // Set custom fill color.
        if let Some(n) = graph.node_mut(style_id) {
            n.inputs[1].default_value = Some(ParamValue::Color([1.0, 0.5, 0.0, 1.0]));
        }

        // Promote to SetStyle.
        convert_style_node(
            style_snarl,
            StyleConversion::FillToStyle,
            &mut graph,
            &mut snarl,
            &mut id_map,
        ).unwrap();
        assert!(matches!(graph.node(style_id).unwrap().op, NodeOp::SetStyle { .. }));

        // Demote back to SetFill.
        convert_style_node(
            style_snarl,
            StyleConversion::StyleToFill,
            &mut graph,
            &mut snarl,
            &mut id_map,
        ).unwrap();

        let node = graph.node(style_id).unwrap();
        assert!(matches!(node.op, NodeOp::SetFill));
        assert_eq!(
            node.inputs[1].default_value,
            Some(ParamValue::Color([1.0, 0.5, 0.0, 1.0]))
        );

        // Connections still intact.
        assert_eq!(
            graph.input_connection(PortId { node: style_id, port: PortIndex(0) }).unwrap().from.node,
            gen_id
        );
        assert_eq!(
            graph.input_connection(PortId { node: out_id, port: PortIndex(0) }).unwrap().from.node,
            style_id
        );
    }
}
