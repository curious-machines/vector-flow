#!/usr/bin/env -S cargo +nightly -Zscript
//! Generate a christmas tree .vflow project file using skeletal strokes.
//!
//! Compile and run:
//!   rustc scripts/gen_christmas_tree.rs -o /tmp/gen_tree && /tmp/gen_tree
//!
//! Output: files/christmas-tree.vflow
//!
//! Graph structure:
//!
//!   Rectangle (trunk)  → Set Fill (brown) ─┐
//!                                           ├→ Merge → Warp to Curve → Graph Output
//!   SVG Path (tree)    → Set Fill (green) ─┘        ↑
//!                                                    │ (curved backbone)
//!   Generate (wind VFS) → Pack Points → Spline from Points
//!
//! The Generate node computes 5 control points each frame. The bottom
//! point is fixed (rooted), higher points sway more. Pack Points zips
//! the x/y scalar arrays into Points. Spline from Points creates a
//! smooth backbone. Warp to Curve deforms the tree onto the backbone.

use std::fmt::Write as FmtWrite;

fn main() {
    let mut b = ProjectBuilder::new();

    // Layout: 6 columns, ~200px apart
    let c = 200.0; // column spacing

    // =====================================================================
    // ROW 1 (y=0): Tree trunk — brown rectangle
    // =====================================================================
    let trunk_rect = b.add_node(
        "Rectangle", "Rectangle", 0.0, 0.0, GEN,
        &[
            port_f("width", "Scalar", 20.0),
            port_f("height", "Scalar", 80.0),
            port_v2("center", [0.0, 100.0]), // below origin — trunk base at y=140
        ],
        &[port("path", "Path")],
    );

    let trunk_fill = b.add_node(
        "Set Fill", "SetFill", c, 0.0, STYLE,
        &[
            port("geometry", "Any"),
            port_c("color", [0.36, 0.20, 0.09, 1.0]), // brown
        ],
        &[port("geometry", "Any")],
    );
    b.wire(trunk_rect, 0, trunk_fill, 0);

    // =====================================================================
    // ROW 2 (y=-200): Tree silhouette — 3-tier green SVG path
    // =====================================================================
    let tree_svg = b.add_node_op(
        "SVG Path",
        r#"{"SvgPath":{"data":"M -90 -10 L 90 -10 L 55 -55 L -55 -55 L -35 -95 L 35 -95 L 0 -160 Z"}}"#,
        0.0, -200.0, GEN,
        &[port("path", "Path")],
    );

    let tree_fill = b.add_node(
        "Set Fill", "SetFill", c, -200.0, STYLE,
        &[
            port("geometry", "Any"),
            port_c("color", [0.10, 0.45, 0.15, 1.0]), // forest green
        ],
        &[port("geometry", "Any")],
    );
    b.wire(tree_svg, 0, tree_fill, 0);

    // =====================================================================
    // Merge trunk + tree
    // =====================================================================
    let merge = b.add_node_op(
        "Merge",
        r#"{"Merge":{"keep_separate":false}}"#,
        c * 2.0, -100.0, UTIL,
        &[port("geometry", "Any")],
    );
    let merge = b.replace_inputs(merge, &[port("input_0", "Any"), port("input_1", "Any")]);
    b.wire(trunk_fill, 0, merge, 0);
    b.wire(tree_fill, 0, merge, 1);

    // =====================================================================
    // ROW 3 (y=-400): Animated backbone via Generate + Pack Points + Spline
    // =====================================================================

    // Generate 0..5: produces 5 spine control points with wind sway
    // Bottom point fixed at (0, 140), higher points sway more
    let wind_gen = b.add_node_op(
        "Generate",
        &gen_op(
            // VFS source: compute sway per control point
            r#"let base_y = 140.0;
let peak_y = -170.0;
let frac = index / (count - 1);
let height = base_y + (peak_y - base_y) * frac;
let wind = sin(time * 0.8) * 0.7 + sin(time * 2.3) * 0.3;
let sway = wind * frac * frac * 40.0;
out_x = sway;
out_y = height;"#,
            // script_inputs (index and count are built-in)
            &[("index", "Int"), ("count", "Int")],
            // script_outputs
            &[("out_x", "Scalar"), ("out_y", "Scalar")],
        ),
        0.0, -400.0, CODE,
        &[],
    );
    // Generate has fixed graph ports: start(0), end(1)
    let wind_gen = b.replace_inputs(wind_gen, &[
        port_i("start", 0),
        port_i("end", 5),
    ]);
    // Generate outputs: one per script_output
    let wind_gen = b.replace_outputs(wind_gen, &[
        port("out_x", "Scalar"),
        port("out_y", "Scalar"),
    ]);

    // Pack Points: zip xs + ys → Points
    let pack = b.add_node(
        "Pack Points", "PackPoints", c, -400.0, UTIL,
        &[
            port("xs", "Scalars"),
            port("ys", "Scalars"),
        ],
        &[port("points", "Points")],
    );
    b.wire(wind_gen, 0, pack, 0); // out_x → xs
    b.wire(wind_gen, 1, pack, 1); // out_y → ys

    // Spline from Points: smooth backbone curve
    let spline = b.add_node(
        "Spline from Points", "SplineFromPoints", c * 2.0, -400.0, PATHOPS,
        &[
            port("points", "Points"),
            port_b("close", false),
            port_f("tension", "Scalar", 0.0),
        ],
        &[port("path", "Path")],
    );
    b.wire(pack, 0, spline, 0);

    // =====================================================================
    // Warp to Curve: deform merged tree onto backbone
    // =====================================================================
    let warp = b.add_node(
        "Warp to Curve", "WarpToCurve", c * 3.0, -200.0, XFORM,
        &[
            port("geometry", "Any"),
            port("curve", "Path"),
            port_i("mode", 0),
            port_f("tolerance", "Scalar", 0.5),
        ],
        &[port("geometry", "Any")],
    );
    b.wire(merge, 0, warp, 0);   // merged geometry → warp
    b.wire(spline, 0, warp, 1);  // backbone curve → warp

    // =====================================================================
    // Graph Output
    // =====================================================================
    let output = b.add_node_op(
        "Graph Output",
        r#"{"GraphOutput":{"name":"output","data_type":"Any","order":0}}"#,
        c * 4.0, -200.0, GRAPH_IO,
        &[],
    );
    let output = b.replace_inputs(output, &[port("output", "Any")]);
    b.set_pinned(output);
    b.wire(warp, 0, output, 0);

    // =====================================================================
    // Serialize
    // =====================================================================
    let json = b.to_json();
    let out_path = "files/christmas-tree.vflow";
    std::fs::write(out_path, &json).expect("failed to write file");
    println!("Wrote {}", out_path);
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: build Generate op JSON
// ─────────────────────────────────────────────────────────────────────────────

fn gen_op(source: &str, inputs: &[(&str, &str)], outputs: &[(&str, &str)]) -> String {
    let esc_source = source.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    let si: Vec<String> = inputs.iter().map(|(n, t)| format!(r#"["{}","{}"]"#, n, t)).collect();
    let so: Vec<String> = outputs.iter().map(|(n, t)| format!(r#"["{}","{}"]"#, n, t)).collect();
    format!(
        r#"{{"Generate":{{"source":"{}","script_inputs":[{}],"script_outputs":[{}]}}}}"#,
        esc_source,
        si.join(","),
        so.join(","),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Category colors (must match ui_node.rs cat_color)
// ─────────────────────────────────────────────────────────────────────────────

const GEN: [u8; 3] = [80, 160, 80];
const XFORM: [u8; 3] = [80, 120, 200];
const PATHOPS: [u8; 3] = [200, 120, 60];
const STYLE: [u8; 3] = [180, 80, 180];
const CODE: [u8; 3] = [120, 200, 160];
const UTIL: [u8; 3] = [140, 140, 140];
const GRAPH_IO: [u8; 3] = [200, 200, 80];

// ─────────────────────────────────────────────────────────────────────────────
// Minimal JSON builder for .vflow project files
// ─────────────────────────────────────────────────────────────────────────────

struct NodeInfo {
    core_id: usize,
    snarl_id: usize,
    op_json: String,
    name: String,
    display_name: String,
    inputs: Vec<PortJson>,
    outputs: Vec<PortJson>,
    x: f32,
    y: f32,
    color: [u8; 3],
    pinned: bool,
}

#[derive(Clone)]
struct PortJson {
    name: String,
    data_type: String,
    default_value: Option<String>,
}

struct Wire {
    from_core: usize,
    from_port: usize,
    to_core: usize,
    to_port: usize,
    from_snarl: usize,
    to_snarl: usize,
}

struct ProjectBuilder {
    nodes: Vec<NodeInfo>,
    wires: Vec<Wire>,
    next_core_id: usize,
    next_snarl_id: usize,
}

fn port(name: &str, dt: &str) -> PortJson {
    PortJson { name: name.into(), data_type: dt.into(), default_value: None }
}

fn port_f(name: &str, dt: &str, val: f64) -> PortJson {
    PortJson {
        name: name.into(),
        data_type: dt.into(),
        default_value: Some(format!(r#"{{"Float":{}}}"#, val)),
    }
}

fn port_i(name: &str, val: i64) -> PortJson {
    PortJson {
        name: name.into(),
        data_type: "Int".into(),
        default_value: Some(format!(r#"{{"Int":{}}}"#, val)),
    }
}

fn port_b(name: &str, val: bool) -> PortJson {
    PortJson {
        name: name.into(),
        data_type: "Bool".into(),
        default_value: Some(format!(r#"{{"Bool":{}}}"#, val)),
    }
}

fn port_v2(name: &str, v: [f32; 2]) -> PortJson {
    PortJson {
        name: name.into(),
        data_type: "Vec2".into(),
        default_value: Some(format!(r#"{{"Vec2":[{},{}]}}"#, v[0], v[1])),
    }
}

fn port_c(name: &str, c: [f32; 4]) -> PortJson {
    PortJson {
        name: name.into(),
        data_type: "Color".into(),
        default_value: Some(format!(r#"{{"Color":[{},{},{},{}]}}"#, c[0], c[1], c[2], c[3])),
    }
}

impl ProjectBuilder {
    fn new() -> Self {
        Self { nodes: Vec::new(), wires: Vec::new(), next_core_id: 1, next_snarl_id: 0 }
    }

    fn add_node(
        &mut self, display: &str, op: &str,
        x: f32, y: f32, color: [u8; 3],
        inputs: &[PortJson], outputs: &[PortJson],
    ) -> usize {
        let op_json = format!(r#""{}""#, op);
        self.add_node_inner(display, &op_json, x, y, color, inputs, outputs)
    }

    fn add_node_op(
        &mut self, display: &str, op_json: &str,
        x: f32, y: f32, color: [u8; 3],
        outputs: &[PortJson],
    ) -> usize {
        self.add_node_inner(display, op_json, x, y, color, &[], outputs)
    }

    fn add_node_inner(
        &mut self, display: &str, op_json: &str,
        x: f32, y: f32, color: [u8; 3],
        inputs: &[PortJson], outputs: &[PortJson],
    ) -> usize {
        let core_id = self.next_core_id;
        let snarl_id = self.next_snarl_id;
        self.next_core_id += 1;
        self.next_snarl_id += 1;
        self.nodes.push(NodeInfo {
            core_id, snarl_id,
            op_json: op_json.to_string(),
            name: display.into(),
            display_name: display.into(),
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
            x, y, color,
            pinned: false,
        });
        core_id
    }

    fn replace_inputs(&mut self, id: usize, inputs: &[PortJson]) -> usize {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.core_id == id) {
            n.inputs = inputs.to_vec();
        }
        id
    }

    fn replace_outputs(&mut self, id: usize, outputs: &[PortJson]) -> usize {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.core_id == id) {
            n.outputs = outputs.to_vec();
        }
        id
    }

    fn set_pinned(&mut self, id: usize) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.core_id == id) {
            n.pinned = true;
        }
    }

    fn wire(&mut self, from: usize, from_port: usize, to: usize, to_port: usize) {
        let from_snarl = self.nodes.iter().find(|n| n.core_id == from).unwrap().snarl_id;
        let to_snarl = self.nodes.iter().find(|n| n.core_id == to).unwrap().snarl_id;
        self.wires.push(Wire {
            from_core: from, from_port, to_core: to, to_port,
            from_snarl, to_snarl,
        });
    }

    fn to_json(&self) -> String {
        let mut s = String::new();
        s.push_str("{\n");

        // graph
        s.push_str("  \"graph\": {\n    \"nodes\": {\n");
        for (i, n) in self.nodes.iter().enumerate() {
            write!(s, "      \"{}\": {}", n.core_id, self.node_json(n)).unwrap();
            if i + 1 < self.nodes.len() { s.push(','); }
            s.push('\n');
        }
        s.push_str("    },\n    \"edges\": [\n");
        for (i, w) in self.wires.iter().enumerate() {
            write!(s, "      {{\"from\":{{\"node\":{},\"port\":{}}},\"to\":{{\"node\":{},\"port\":{}}}}}",
                w.from_core, w.from_port, w.to_core, w.to_port).unwrap();
            if i + 1 < self.wires.len() { s.push(','); }
            s.push('\n');
        }
        write!(s, "    ],\n    \"next_id\": {},\n", self.next_core_id).unwrap();
        s.push_str("    \"generation\": 0,\n    \"network_boxes\": {},\n    \"next_box_id\": 0\n  },\n");

        // snarl
        s.push_str("  \"snarl\": {\n    \"nodes\": {\n");
        for (i, n) in self.nodes.iter().enumerate() {
            write!(s,
                "      \"{}\": {{\"value\":{{\"core_id\":{},\"display_name\":\"{}\",\"color\":[{},{},{},255],\"pinned\":{}}},\"pos\":{{\"x\":{:.1},\"y\":{:.1}}},\"open\":true}}",
                n.snarl_id, n.core_id, n.display_name,
                n.color[0], n.color[1], n.color[2], n.pinned, n.x, n.y
            ).unwrap();
            if i + 1 < self.nodes.len() { s.push(','); }
            s.push('\n');
        }
        s.push_str("    },\n    \"wires\": [\n");
        for (i, w) in self.wires.iter().enumerate() {
            write!(s, "      {{\"out_pin\":{{\"node\":{},\"output\":{}}},\"in_pin\":{{\"node\":{},\"input\":{}}}}}",
                w.from_snarl, w.from_port, w.to_snarl, w.to_port).unwrap();
            if i + 1 < self.wires.len() { s.push(','); }
            s.push('\n');
        }
        s.push_str("    ]\n  },\n");

        // view, window, settings
        s.push_str(r#"  "view_state": {"graph_offset":[300.0,-200.0],"graph_scale":0.65,"canvas_center":[0.0,0.0],"canvas_zoom":1.0},"#);
        s.push('\n');
        s.push_str(r#"  "window_geometry": {"x":300.0,"y":200.0,"width":1400.0,"height":900.0,"node_editor_height":400.0,"properties_width":350.0},"#);
        s.push('\n');
        s.push_str(r#"  "settings": {"canvas_width":640,"canvas_height":480,"background_color":[0.05,0.05,0.15,1.0],"fps":30.0}"#);
        s.push('\n');
        s.push_str("}\n");
        s
    }

    fn node_json(&self, n: &NodeInfo) -> String {
        let mut s = String::new();
        write!(s, "{{\"id\":{},\"name\":\"{}\",\"op\":{},\"inputs\":[",
            n.core_id, n.name, n.op_json).unwrap();
        for (i, p) in n.inputs.iter().enumerate() {
            if i > 0 { s.push(','); }
            write!(s, "{{\"name\":\"{}\",\"data_type\":\"{}\",\"description\":\"\",\"default_value\":{},\"expression\":null}}",
                p.name, p.data_type, p.default_value.as_deref().unwrap_or("null")).unwrap();
        }
        s.push_str("],\"outputs\":[");
        for (i, p) in n.outputs.iter().enumerate() {
            if i > 0 { s.push(','); }
            write!(s, "{{\"name\":\"{}\",\"data_type\":\"{}\",\"description\":\"\",\"default_value\":null,\"expression\":null}}",
                p.name, p.data_type).unwrap();
        }
        write!(s, "],\"position\":[{:.1},{:.1}],\"generation\":0,\"version\":0}}", n.x, n.y).unwrap();
        s
    }
}
