use serde::{Deserialize, Serialize};

use crate::types::{DataType, NodeId};

// ---------------------------------------------------------------------------
// Port addressing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortIndex(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortId {
    pub node: NodeId,
    pub port: PortIndex,
}

// ---------------------------------------------------------------------------
// ParamValue — literal values storable in ports
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    Float(f64),
    Int(i64),
    Bool(bool),
    String(String),
    Vec2([f32; 2]),
    Color([f32; 4]),
}

// ---------------------------------------------------------------------------
// PortDef — unified port definition (param + input merged)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortDef {
    pub name: String,
    pub data_type: DataType,
    pub description: String,
    pub default_value: Option<ParamValue>,
    pub expression: Option<String>,
    /// Whether this port is visible on the node in the graph by default.
    /// Instance visibility is stored on `NodeDef` and initialized from this flag.
    #[serde(default = "default_visible")]
    pub visible_by_default: bool,
}

fn default_visible() -> bool {
    true
}

impl PortDef {
    pub fn new(name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            data_type,
            description: String::new(),
            default_value: None,
            expression: None,
            visible_by_default: true,
        }
    }

    pub fn with_default(mut self, value: ParamValue) -> Self {
        self.default_value = Some(value);
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Mark this port as hidden by default on the node in the graph.
    pub fn hidden(mut self) -> Self {
        self.visible_by_default = false;
        self
    }
}

// ---------------------------------------------------------------------------
// NodeOp — all built-in operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeOp {
    // Generators
    RegularPolygon,
    PointGrid,
    Circle,
    Rectangle,
    Line,
    ScatterPoints,
    // Transforms
    ApplyTransform,
    Translate,
    Rotate,
    Scale,
    // Path ops
    PathBoolean { operation: i32 },
    PathOffset,
    PathSubdivide,
    PathReverse,
    ResamplePath,
    // Styling
    SetFill,
    SetStroke { dash_pattern: String },
    SetStyle { dash_pattern: String },
    StrokeToPath { dash_pattern: String },
    // Color operations
    AdjustHue,
    AdjustSaturation,
    AdjustLightness,
    AdjustLuminance,
    InvertColor,
    Grayscale,
    MixColors,
    AdjustAlpha,
    ColorParse { text: String },
    SvgPath { data: String },
    // Constants
    ConstScalar,
    ConstInt,
    ConstVec2,
    ConstColor,
    // Portals (named nets)
    PortalSend { label: String },
    PortalReceive { label: String },
    // Utility
    Merge { #[serde(default)] keep_separate: bool },
    Duplicate,
    CopyToPoints,
    PlaceAtPoints,

    // DSL
    DslCode {
        source: String,
        /// Port definitions for the compiler (name, DataType).
        /// Kept in sync with NodeDef.inputs / NodeDef.outputs by the UI.
        script_inputs: Vec<(String, DataType)>,
        script_outputs: Vec<(String, DataType)>,
    },
    /// Map: iterate a batch, run DSL code per element, collect results.
    Map {
        source: String,
        script_inputs: Vec<(String, DataType)>,
        script_outputs: Vec<(String, DataType)>,
    },
    /// Generate: run DSL code for each index in start..end, collect results.
    Generate {
        source: String,
        script_inputs: Vec<(String, DataType)>,
        script_outputs: Vec<(String, DataType)>,
    },
    // Image
    LoadImage { path: String },
    // Text
    Text {
        text: String,
        font_family: String,
        font_path: String,
    },
    TextToPath,
    // Graph I/O
    GraphInput { name: String, data_type: DataType },
    GraphOutput { name: String, data_type: DataType, #[serde(default)] order: i32 },
}

impl NodeOp {
    /// Returns the current version number for this node operation.
    ///
    /// Built-in ops start at version 1. Bump when the port layout or behavior changes.
    /// User-defined ops (DslCode, Map, Generate, GraphInput, GraphOutput) return 0
    /// because their ports are defined per-instance, not by a fixed schema.
    pub fn current_version(&self) -> u32 {
        match self {
            // User-defined port layouts — no fixed schema to version.
            NodeOp::DslCode { .. }
            | NodeOp::Map { .. }
            | NodeOp::Generate { .. }
            | NodeOp::GraphInput { .. }
            | NodeOp::GraphOutput { .. } => 0,

            // Version 2: added parts output port + Divide operation.
            NodeOp::PathBoolean { .. } => 2,

            // Version 1: added tolerance input port.
            NodeOp::ResamplePath
            | NodeOp::CopyToPoints
            | NodeOp::SetStroke { .. }
            | NodeOp::SetStyle { .. }
            | NodeOp::StrokeToPath { .. } => 1,

            // All other built-in ops start at version 0. Bump individually when
            // a node's port layout or behavior changes.
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// NodeDef — a node instance in the graph
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDef {
    pub id: NodeId,
    pub name: String,
    pub op: NodeOp,
    pub inputs: Vec<PortDef>,
    pub outputs: Vec<PortDef>,
    pub position: [f32; 2],
    pub generation: u64,
    /// Node definition version. Used to detect outdated nodes after loading a project
    /// saved with an older version of a node's definition. Defaults to 0 for backward
    /// compatibility with files saved before versioning was introduced.
    #[serde(default)]
    pub version: u32,
    /// Per-instance visibility for input ports. Initialized from `visible_by_default`.
    /// Length matches `inputs`. Missing entries (e.g. from older saves) default to true.
    #[serde(default)]
    pub input_visibility: Vec<bool>,
    /// Per-instance visibility for output ports.
    #[serde(default)]
    pub output_visibility: Vec<bool>,
}

/// Generate a port name from an index: 0→"a", 1→"b", ..., 25→"z", 26→"a1", etc.
fn variadic_port_name(idx: usize) -> String {
    let letter = (b'a' + (idx % 26) as u8) as char;
    if idx < 26 {
        letter.to_string()
    } else {
        format!("{}{}", letter, idx / 26)
    }
}

impl NodeDef {
    /// Bump the generation counter (call when params, expressions, or structure change).
    pub fn touch(&mut self) {
        self.generation += 1;
    }

    /// Ensure visibility vectors match port counts. Missing entries get `true` (visible).
    /// Called after deserialization to handle older save files or newly added ports.
    pub fn sync_visibility(&mut self) {
        self.input_visibility.resize(self.inputs.len(), true);
        self.output_visibility.resize(self.outputs.len(), true);
    }

    /// Initialize visibility from port definitions' `visible_by_default` flags.
    pub fn init_visibility(&mut self) {
        self.input_visibility = self.inputs.iter().map(|p| p.visible_by_default).collect();
        self.output_visibility = self.outputs.iter().map(|p| p.visible_by_default).collect();
    }

    /// Returns the number of currently visible input ports.
    pub fn visible_input_count(&self) -> usize {
        self.input_visibility.iter().filter(|&&v| v).count()
    }

    /// Returns the number of currently visible output ports.
    pub fn visible_output_count(&self) -> usize {
        self.output_visibility.iter().filter(|&&v| v).count()
    }

    /// Map a visible input index to the actual port index.
    /// Returns None if the visible index is out of range.
    pub fn visible_input_to_port(&self, visible_idx: usize) -> Option<usize> {
        self.input_visibility.iter()
            .enumerate()
            .filter(|(_, &v)| v)
            .nth(visible_idx)
            .map(|(i, _)| i)
    }

    /// Map a visible output index to the actual port index.
    pub fn visible_output_to_port(&self, visible_idx: usize) -> Option<usize> {
        self.output_visibility.iter()
            .enumerate()
            .filter(|(_, &v)| v)
            .nth(visible_idx)
            .map(|(i, _)| i)
    }

    /// Map an actual port index to its visible input index, or None if hidden.
    pub fn port_to_visible_input(&self, port_idx: usize) -> Option<usize> {
        if port_idx >= self.input_visibility.len() || !self.input_visibility[port_idx] {
            return None;
        }
        Some(self.input_visibility[..port_idx].iter().filter(|&&v| v).count())
    }

    /// Map an actual port index to its visible output index, or None if hidden.
    pub fn port_to_visible_output(&self, port_idx: usize) -> Option<usize> {
        if port_idx >= self.output_visibility.len() || !self.output_visibility[port_idx] {
            return None;
        }
        Some(self.output_visibility[..port_idx].iter().filter(|&&v| v).count())
    }

    /// Returns true if this node's version is older than the current version
    /// for its operation. User-defined ops (version 0) are never outdated.
    pub fn is_outdated(&self) -> bool {
        let current = self.op.current_version();
        current > 0 && self.version < current
    }

    /// Whether this node supports a variable number of inputs.
    pub fn is_variadic(&self) -> bool {
        matches!(self.op, NodeOp::Merge { .. })
    }

    /// Add another variadic input port. Returns the new port index.
    pub fn add_variadic_input(&mut self) -> Option<usize> {
        if !self.is_variadic() {
            return None;
        }
        let idx = self.inputs.len();
        let name = variadic_port_name(idx);
        let desc = "Input";
        let port = PortDef::new(name, DataType::Any)
            .with_description(desc);
        self.inputs.push(port);
        self.touch();
        Some(idx)
    }

    /// Remove the last variadic input port. Won't go below 2 inputs.
    /// Returns true if a port was removed.
    pub fn remove_variadic_input(&mut self) -> bool {
        if !self.is_variadic() || self.inputs.len() <= 2 {
            return false;
        }
        self.inputs.pop();
        self.touch();
        true
    }
}

// ---------------------------------------------------------------------------
// Factory functions — create NodeDefs with correct port definitions
// ---------------------------------------------------------------------------

impl NodeDef {
    pub fn regular_polygon(id: NodeId) -> Self {
        Self {
            id,
            name: "Regular Polygon".into(),
            op: NodeOp::RegularPolygon,
            inputs: vec![
                PortDef::new("sides", DataType::Int)
                    .with_default(ParamValue::Int(6))
                    .with_description("Number of sides"),
                PortDef::new("radius", DataType::Scalar)
                    .with_default(ParamValue::Float(100.0))
                    .with_description("Outer radius"),
                PortDef::new("center", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Center position"),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn circle(id: NodeId) -> Self {
        Self {
            id,
            name: "Circle".into(),
            op: NodeOp::Circle,
            inputs: vec![
                PortDef::new("radius", DataType::Scalar)
                    .with_default(ParamValue::Float(100.0))
                    .with_description("Circle radius"),
                PortDef::new("center", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Center position"),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn rectangle(id: NodeId) -> Self {
        Self {
            id,
            name: "Rectangle".into(),
            op: NodeOp::Rectangle,
            inputs: vec![
                PortDef::new("width", DataType::Scalar)
                    .with_default(ParamValue::Float(200.0))
                    .with_description("Width"),
                PortDef::new("height", DataType::Scalar)
                    .with_default(ParamValue::Float(100.0))
                    .with_description("Height"),
                PortDef::new("center", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Center position"),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn line(id: NodeId) -> Self {
        Self {
            id,
            name: "Line".into(),
            op: NodeOp::Line,
            inputs: vec![
                PortDef::new("from", DataType::Vec2)
                    .with_default(ParamValue::Vec2([-100.0, 0.0]))
                    .with_description("Start point"),
                PortDef::new("to", DataType::Vec2)
                    .with_default(ParamValue::Vec2([100.0, 0.0]))
                    .with_description("End point"),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn point_grid(id: NodeId) -> Self {
        Self {
            id,
            name: "Point Grid".into(),
            op: NodeOp::PointGrid,
            inputs: vec![
                PortDef::new("columns", DataType::Int)
                    .with_default(ParamValue::Int(10))
                    .with_description("Number of columns"),
                PortDef::new("rows", DataType::Int)
                    .with_default(ParamValue::Int(10))
                    .with_description("Number of rows"),
                PortDef::new("spacing", DataType::Scalar)
                    .with_default(ParamValue::Float(20.0))
                    .with_description("Distance between points"),
            ],
            outputs: vec![PortDef::new("points", DataType::Points)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn scatter_points(id: NodeId) -> Self {
        Self {
            id,
            name: "Scatter Points".into(),
            op: NodeOp::ScatterPoints,
            inputs: vec![
                PortDef::new("count", DataType::Int)
                    .with_default(ParamValue::Int(100))
                    .with_description("Number of points"),
                PortDef::new("width", DataType::Scalar)
                    .with_default(ParamValue::Float(500.0))
                    .with_description("Scatter region width"),
                PortDef::new("height", DataType::Scalar)
                    .with_default(ParamValue::Float(500.0))
                    .with_description("Scatter region height"),
                PortDef::new("seed", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Random seed"),
            ],
            outputs: vec![PortDef::new("points", DataType::Points)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn translate(id: NodeId) -> Self {
        Self {
            id,
            name: "Translate".into(),
            op: NodeOp::Translate,
            inputs: vec![
                PortDef::new("geometry", DataType::Any).with_description("Input geometry"),
                PortDef::new("offset", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Translation offset"),
            ],
            outputs: vec![PortDef::new("geometry", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn rotate(id: NodeId) -> Self {
        Self {
            id,
            name: "Rotate".into(),
            op: NodeOp::Rotate,
            inputs: vec![
                PortDef::new("geometry", DataType::Any).with_description("Input geometry"),
                PortDef::new("angle", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Rotation angle in degrees"),
                PortDef::new("center", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Center of rotation"),
            ],
            outputs: vec![PortDef::new("geometry", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn scale(id: NodeId) -> Self {
        Self {
            id,
            name: "Scale".into(),
            op: NodeOp::Scale,
            inputs: vec![
                PortDef::new("geometry", DataType::Any).with_description("Input geometry"),
                PortDef::new("factor", DataType::Vec2)
                    .with_default(ParamValue::Vec2([1.0, 1.0]))
                    .with_description("Scale factor (x, y)"),
                PortDef::new("center", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Center of scaling"),
            ],
            outputs: vec![PortDef::new("geometry", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn apply_transform(id: NodeId) -> Self {
        Self {
            id,
            name: "Apply Transform".into(),
            op: NodeOp::ApplyTransform,
            inputs: vec![
                PortDef::new("geometry", DataType::Any).with_description("Input geometry"),
                PortDef::new("transform", DataType::Transform)
                    .with_description("Transform to apply"),
            ],
            outputs: vec![PortDef::new("geometry", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn set_fill(id: NodeId) -> Self {
        Self {
            id,
            name: "Set Fill".into(),
            op: NodeOp::SetFill,
            inputs: vec![
                PortDef::new("geometry", DataType::Any).with_description("Input geometry"),
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Fill color (single or batch)"),
            ],
            outputs: vec![PortDef::new("geometry", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn set_stroke(id: NodeId) -> Self {
        Self {
            id,
            name: "Set Stroke".into(),
            op: NodeOp::SetStroke { dash_pattern: String::new() },
            inputs: vec![
                PortDef::new("geometry", DataType::Any).with_description("Input geometry"),
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([0.0, 0.0, 0.0, 1.0]))
                    .with_description("Stroke color (single or batch)"),
                PortDef::new("width", DataType::Scalar)
                    .with_default(ParamValue::Float(2.0))
                    .with_description("Stroke width"),
                PortDef::new("cap", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("End cap: 0=Butt, 1=Round, 2=Square")
                    .hidden(),
                PortDef::new("join", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Line join: 0=Miter, 1=Round, 2=Bevel")
                    .hidden(),
                PortDef::new("miter_limit", DataType::Scalar)
                    .with_default(ParamValue::Float(4.0))
                    .with_description("Miter limit (only for Miter join)")
                    .hidden(),
                PortDef::new("dash_offset", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Dash pattern offset")
                    .hidden(),
                PortDef::new("tolerance", DataType::Scalar)
                    .with_default(ParamValue::Float(0.5))
                    .with_description("Curve flattening tolerance for dash pattern (smaller = more precise)")
                    .hidden(),
            ],
            outputs: vec![PortDef::new("geometry", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 1,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn stroke_to_path(id: NodeId) -> Self {
        Self {
            id,
            name: "Stroke to Path".into(),
            op: NodeOp::StrokeToPath { dash_pattern: String::new() },
            inputs: vec![
                PortDef::new("shape", DataType::Any).with_description("Input shape or path"),
                PortDef::new("width", DataType::Scalar)
                    .with_default(ParamValue::Float(2.0))
                    .with_description("Stroke width"),
                PortDef::new("cap", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("End cap: 0=Butt, 1=Round, 2=Square")
                    .hidden(),
                PortDef::new("join", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Line join: 0=Miter, 1=Round, 2=Bevel")
                    .hidden(),
                PortDef::new("miter_limit", DataType::Scalar)
                    .with_default(ParamValue::Float(4.0))
                    .with_description("Miter limit (only for Miter join)")
                    .hidden(),
                PortDef::new("dash_offset", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Dash pattern offset")
                    .hidden(),
                PortDef::new("tolerance", DataType::Scalar)
                    .with_default(ParamValue::Float(0.5))
                    .with_description("Curve flattening tolerance (smaller = more precise)")
                    .hidden(),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 1,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn set_style(id: NodeId) -> Self {
        Self {
            id,
            name: "Set Style".into(),
            op: NodeOp::SetStyle { dash_pattern: String::new() },
            inputs: vec![
                // 0: path
                PortDef::new("path", DataType::Any)
                    .with_description("Input geometry"),
                // 1: fill_color
                PortDef::new("fill_color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Fill color"),
                // 2: fill_opacity
                PortDef::new("fill_opacity", DataType::Scalar)
                    .with_default(ParamValue::Float(1.0))
                    .with_description("Fill opacity (0-1)")
                    .hidden(),
                // 3: has_fill
                PortDef::new("has_fill", DataType::Bool)
                    .with_default(ParamValue::Bool(true))
                    .with_description("Enable fill")
                    .hidden(),
                // 4: stroke_color
                PortDef::new("stroke_color", DataType::Color)
                    .with_default(ParamValue::Color([0.0, 0.0, 0.0, 1.0]))
                    .with_description("Stroke color"),
                // 5: stroke_width
                PortDef::new("stroke_width", DataType::Scalar)
                    .with_default(ParamValue::Float(2.0))
                    .with_description("Stroke width"),
                // 6: stroke_opacity
                PortDef::new("stroke_opacity", DataType::Scalar)
                    .with_default(ParamValue::Float(1.0))
                    .with_description("Stroke opacity (0-1)")
                    .hidden(),
                // 7: has_stroke
                PortDef::new("has_stroke", DataType::Bool)
                    .with_default(ParamValue::Bool(true))
                    .with_description("Enable stroke")
                    .hidden(),
                // 8: cap
                PortDef::new("cap", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("End cap: 0=Butt, 1=Round, 2=Square")
                    .hidden(),
                // 9: join
                PortDef::new("join", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Line join: 0=Miter, 1=Round, 2=Bevel")
                    .hidden(),
                // 10: miter_limit
                PortDef::new("miter_limit", DataType::Scalar)
                    .with_default(ParamValue::Float(4.0))
                    .with_description("Miter limit (only for Miter join)")
                    .hidden(),
                // 11: dash_offset
                PortDef::new("dash_offset", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Dash pattern offset")
                    .hidden(),
            ],
            outputs: vec![PortDef::new("output", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 1,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn merge(id: NodeId) -> Self {
        Self {
            id,
            name: "Merge".into(),
            op: NodeOp::Merge { keep_separate: false },
            inputs: vec![
                PortDef::new("a", DataType::Any).with_description("First input"),
                PortDef::new("b", DataType::Any).with_description("Second input"),
            ],
            outputs: vec![PortDef::new("merged", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn duplicate(id: NodeId) -> Self {
        Self {
            id,
            name: "Duplicate".into(),
            op: NodeOp::Duplicate,
            inputs: vec![
                PortDef::new("geometry", DataType::Any).with_description("Input geometry"),
                PortDef::new("count", DataType::Int)
                    .with_default(ParamValue::Int(5))
                    .with_description("Number of copies"),
                PortDef::new("transform", DataType::Transform)
                    .with_description("Transform applied per copy (cumulative)"),
            ],
            outputs: vec![PortDef::new("geometry", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn copy_to_points(id: NodeId) -> Self {
        Self {
            id,
            name: "Copy to Points".into(),
            op: NodeOp::CopyToPoints,
            inputs: vec![
                PortDef::new("geometry", DataType::Any)
                    .with_description("Shape to copy to each point"),
                PortDef::new("target_path", DataType::Path)
                    .with_description("Path whose sampled points receive copies"),
                PortDef::new("count", DataType::Int)
                    .with_default(ParamValue::Int(10))
                    .with_description("Number of copies along path"),
                PortDef::new("align", DataType::Bool)
                    .with_default(ParamValue::Bool(true))
                    .with_description("Rotate copies to align with path tangent"),
                PortDef::new("tolerance", DataType::Scalar)
                    .with_default(ParamValue::Float(0.5))
                    .with_description("Curve flattening tolerance (smaller = more precise)"),
            ],
            outputs: vec![
                PortDef::new("geometry", DataType::Shapes),
                PortDef::new("tangent_angles", DataType::Scalars)
                    .with_description("Tangent angle in degrees at each point"),
                PortDef::new("indices", DataType::Scalars)
                    .with_description("Index of each copy (0..count-1)"),
                PortDef::new("count", DataType::Scalar)
                    .with_description("Total number of copies"),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 1,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn place_at_points(id: NodeId) -> Self {
        Self {
            id,
            name: "Place at Points".into(),
            op: NodeOp::PlaceAtPoints,
            inputs: vec![
                PortDef::new("geometry", DataType::Any)
                    .with_description("Shapes to place (batch or single)"),
                PortDef::new("points", DataType::Points)
                    .with_description("Target points from Grid, Scatter Points, etc."),
                PortDef::new("cycle", DataType::Bool)
                    .with_default(ParamValue::Bool(false))
                    .with_description("Cycle shorter list to match longer list length"),
            ],
            outputs: vec![
                PortDef::new("geometry", DataType::Shapes),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn dsl_code(id: NodeId, source: String) -> Self {
        Self {
            id,
            name: "VFS Code".into(),
            op: NodeOp::DslCode {
                source,
                script_inputs: Vec::new(),
                script_outputs: Vec::new(),
            },
            inputs: Vec::new(),
            outputs: Vec::new(),
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn map(id: NodeId) -> Self {
        // script_inputs: element, index, count are built-in (populated from batch).
        // They appear in script_inputs so the DSL compiler sees them, but they
        // do NOT have corresponding graph input ports.
        // Any user-added script inputs beyond these three get graph ports (starting at port 1).
        //
        // script_outputs: each gets a graph output port (1:1 sync).
        //
        // Graph input port 0 is always "batch" (fixed, not in script_inputs).
        Self {
            id,
            name: "Map".into(),
            op: NodeOp::Map {
                source: String::new(),
                script_inputs: vec![
                    ("element".into(), DataType::Scalar),
                    ("index".into(), DataType::Int),
                    ("count".into(), DataType::Int),
                ],
                script_outputs: vec![
                    ("result".into(), DataType::Scalar),
                ],
            },
            inputs: vec![
                PortDef::new("batch", DataType::Any)
                    .with_description("Batch to iterate over"),
            ],
            outputs: vec![
                PortDef::new("result", DataType::Any)
                    .with_description("Collected output batch"),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn generate(id: NodeId) -> Self {
        // script_inputs: index and count are built-in (populated from range).
        // They appear in script_inputs so the DSL compiler sees them, but they
        // do NOT have corresponding graph input ports.
        // Any user-added script inputs beyond these two get graph ports (starting at port 2).
        //
        // Graph input port 0 is "start" (Int), port 1 is "end" (Int).
        Self {
            id,
            name: "Generate".into(),
            op: NodeOp::Generate {
                source: "result = index;".into(),
                script_inputs: vec![
                    ("index".into(), DataType::Int),
                    ("count".into(), DataType::Int),
                ],
                script_outputs: vec![
                    ("result".into(), DataType::Scalar),
                ],
            },
            inputs: vec![
                PortDef::new("start", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Range start (inclusive)"),
                PortDef::new("end", DataType::Int)
                    .with_default(ParamValue::Int(10))
                    .with_description("Range end (exclusive)"),
            ],
            outputs: vec![
                PortDef::new("result", DataType::Any)
                    .with_description("Collected output batch"),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn graph_input(id: NodeId, name: String, data_type: DataType) -> Self {
        Self {
            id,
            name: format!("Input: {}", name),
            op: NodeOp::GraphInput {
                name: name.clone(),
                data_type,
            },
            inputs: vec![],
            outputs: vec![PortDef::new(name, data_type)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn graph_output(id: NodeId, name: String, data_type: DataType) -> Self {
        Self {
            id,
            name: format!("Output: {}", name),
            op: NodeOp::GraphOutput {
                name: name.clone(),
                data_type,
                order: 0,
            },
            inputs: vec![PortDef::new(name, data_type)],
            outputs: vec![],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }
}

impl NodeDef {
    pub fn path_boolean(id: NodeId) -> Self {
        Self {
            id,
            name: "Path Boolean".into(),
            op: NodeOp::PathBoolean { operation: 0 },
            inputs: vec![
                PortDef::new("a", DataType::Path).with_description("First path"),
                PortDef::new("b", DataType::Path).with_description("Second path"),
                PortDef::new("tolerance", DataType::Scalar)
                    .with_default(ParamValue::Float(0.5))
                    .with_description("Curve flattening tolerance (smaller = more precise)"),
            ],
            outputs: vec![
                PortDef::new("result", DataType::Path),
                PortDef::new("parts", DataType::Paths),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 2,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn path_offset(id: NodeId) -> Self {
        Self {
            id,
            name: "Path Offset".into(),
            op: NodeOp::PathOffset,
            inputs: vec![
                PortDef::new("path", DataType::Path).with_description("Input path"),
                PortDef::new("distance", DataType::Scalar)
                    .with_default(ParamValue::Float(10.0))
                    .with_description("Offset distance"),
                PortDef::new("tolerance", DataType::Scalar)
                    .with_default(ParamValue::Float(0.5))
                    .with_description("Curve flattening tolerance (smaller = more precise)"),
            ],
            outputs: vec![PortDef::new("result", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 1,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn path_subdivide(id: NodeId) -> Self {
        Self {
            id,
            name: "Path Subdivide".into(),
            op: NodeOp::PathSubdivide,
            inputs: vec![
                PortDef::new("path", DataType::Path).with_description("Input path"),
                PortDef::new("levels", DataType::Int)
                    .with_default(ParamValue::Int(1))
                    .with_description("Subdivision levels"),
            ],
            outputs: vec![PortDef::new("result", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn path_reverse(id: NodeId) -> Self {
        Self {
            id,
            name: "Path Reverse".into(),
            op: NodeOp::PathReverse,
            inputs: vec![
                PortDef::new("path", DataType::Path).with_description("Input path"),
            ],
            outputs: vec![PortDef::new("result", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn resample_path(id: NodeId) -> Self {
        Self {
            id,
            name: "Resample Path".into(),
            op: NodeOp::ResamplePath,
            inputs: vec![
                PortDef::new("path", DataType::Path).with_description("Input path"),
                PortDef::new("count", DataType::Int)
                    .with_default(ParamValue::Int(32))
                    .with_description("Number of samples"),
                PortDef::new("tolerance", DataType::Scalar)
                    .with_default(ParamValue::Float(0.5))
                    .with_description("Curve flattening tolerance (smaller = more precise)"),
            ],
            outputs: vec![PortDef::new("points", DataType::Points)],
            position: [0.0, 0.0],
            generation: 0,
            version: 1,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn const_scalar(id: NodeId) -> Self {
        Self {
            id,
            name: "Constant Scalar".into(),
            op: NodeOp::ConstScalar,
            inputs: vec![
                PortDef::new("value", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Scalar value"),
            ],
            outputs: vec![PortDef::new("value", DataType::Scalar)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn const_int(id: NodeId) -> Self {
        Self {
            id,
            name: "Constant Int".into(),
            op: NodeOp::ConstInt,
            inputs: vec![
                PortDef::new("value", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Integer value"),
            ],
            outputs: vec![PortDef::new("value", DataType::Int)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn const_vec2(id: NodeId) -> Self {
        Self {
            id,
            name: "Constant Vec2".into(),
            op: NodeOp::ConstVec2,
            inputs: vec![
                PortDef::new("x", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("X component"),
                PortDef::new("y", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Y component"),
            ],
            outputs: vec![PortDef::new("value", DataType::Vec2)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn const_color(id: NodeId) -> Self {
        Self {
            id,
            name: "Constant Color".into(),
            op: NodeOp::ConstColor,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Color value"),
            ],
            outputs: vec![PortDef::new("value", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn adjust_hue(id: NodeId) -> Self {
        Self {
            id,
            name: "Adjust Hue".into(),
            op: NodeOp::AdjustHue,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
                PortDef::new("amount", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Hue shift in degrees (or absolute hue)"),
                PortDef::new("absolute", DataType::Bool)
                    .with_default(ParamValue::Bool(false))
                    .with_description("If true, set hue; if false, shift hue"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn adjust_saturation(id: NodeId) -> Self {
        Self {
            id,
            name: "Adjust Saturation".into(),
            op: NodeOp::AdjustSaturation,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
                PortDef::new("amount", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Saturation adjustment (-1..1)"),
                PortDef::new("absolute", DataType::Bool)
                    .with_default(ParamValue::Bool(false))
                    .with_description("If true, set saturation; if false, shift"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn adjust_lightness(id: NodeId) -> Self {
        Self {
            id,
            name: "Adjust Lightness".into(),
            op: NodeOp::AdjustLightness,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
                PortDef::new("amount", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Lightness adjustment (-1..1)"),
                PortDef::new("absolute", DataType::Bool)
                    .with_default(ParamValue::Bool(false))
                    .with_description("If true, set lightness; if false, shift"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn adjust_luminance(id: NodeId) -> Self {
        Self {
            id,
            name: "Adjust Luminance".into(),
            op: NodeOp::AdjustLuminance,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
                PortDef::new("amount", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("CIE Lab L* adjustment (0..100)"),
                PortDef::new("absolute", DataType::Bool)
                    .with_default(ParamValue::Bool(false))
                    .with_description("If true, set L*; if false, shift"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn invert_color(id: NodeId) -> Self {
        Self {
            id,
            name: "Invert Color".into(),
            op: NodeOp::InvertColor,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn grayscale(id: NodeId) -> Self {
        Self {
            id,
            name: "Grayscale".into(),
            op: NodeOp::Grayscale,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn mix_colors(id: NodeId) -> Self {
        Self {
            id,
            name: "Mix Colors".into(),
            op: NodeOp::MixColors,
            inputs: vec![
                PortDef::new("color_a", DataType::Color)
                    .with_default(ParamValue::Color([0.0, 0.0, 0.0, 1.0]))
                    .with_description("First color"),
                PortDef::new("color_b", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Second color"),
                PortDef::new("factor", DataType::Scalar)
                    .with_default(ParamValue::Float(0.5))
                    .with_description("Mix factor (0=A, 1=B)"),
                PortDef::new("lab_mode", DataType::Bool)
                    .with_default(ParamValue::Bool(false))
                    .with_description("If true, interpolate in CIE Lab space"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn adjust_alpha(id: NodeId) -> Self {
        Self {
            id,
            name: "Adjust Alpha".into(),
            op: NodeOp::AdjustAlpha,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
                PortDef::new("amount", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Alpha adjustment (-1..1)"),
                PortDef::new("absolute", DataType::Bool)
                    .with_default(ParamValue::Bool(false))
                    .with_description("If true, set alpha; if false, shift alpha"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn color_parse(id: NodeId, text: String) -> Self {
        Self {
            id,
            name: "Color Parse".into(),
            op: NodeOp::ColorParse { text },
            inputs: vec![],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn svg_path(id: NodeId, data: String) -> Self {
        Self {
            id,
            name: "SVG Path".into(),
            op: NodeOp::SvgPath { data },
            inputs: vec![],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn load_image(id: NodeId, path: String) -> Self {
        Self {
            id,
            name: "Load Image".into(),
            op: NodeOp::LoadImage { path },
            inputs: vec![
                PortDef::new("position", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Center position"),
                PortDef::new("width", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Display width (0 = native)"),
                PortDef::new("height", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Display height (0 = native)"),
                PortDef::new("opacity", DataType::Scalar)
                    .with_default(ParamValue::Float(1.0))
                    .with_description("Image opacity (0..1)"),
            ],
            outputs: vec![
                PortDef::new("image", DataType::Image),
                PortDef::new("native_width", DataType::Scalar),
                PortDef::new("native_height", DataType::Scalar),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn text(id: NodeId, text: String) -> Self {
        Self {
            id,
            name: "Text".into(),
            op: NodeOp::Text {
                text,
                font_family: String::new(),
                font_path: String::new(),
            },
            inputs: vec![
                PortDef::new("position", DataType::Vec2)
                    .with_default(ParamValue::Vec2([0.0, 0.0]))
                    .with_description("Text anchor position"),
                PortDef::new("font_size", DataType::Scalar)
                    .with_default(ParamValue::Float(24.0))
                    .with_description("Font size in canvas units"),
                PortDef::new("font_weight", DataType::Int)
                    .with_default(ParamValue::Int(400))
                    .with_description("Font weight (100-900, 400=regular, 700=bold)"),
                PortDef::new("font_style", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Font style: 0=Normal, 1=Italic, 2=Oblique"),
                PortDef::new("letter_spacing", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Extra spacing between glyphs"),
                PortDef::new("line_height", DataType::Scalar)
                    .with_default(ParamValue::Float(1.2))
                    .with_description("Line height multiplier"),
                PortDef::new("alignment", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Text alignment: 0=Left, 1=Center, 2=Right"),
                PortDef::new("box_width", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Text box width (0 = unconstrained)"),
                PortDef::new("box_height", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Text box height (0 = unconstrained)"),
                PortDef::new("wrap", DataType::Bool)
                    .with_default(ParamValue::Bool(true))
                    .with_description("Enable word wrapping"),
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Text color"),
                PortDef::new("opacity", DataType::Scalar)
                    .with_default(ParamValue::Float(1.0))
                    .with_description("Opacity (0..1)"),
            ],
            outputs: vec![
                PortDef::new("text", DataType::Text),
                PortDef::new("width", DataType::Scalar),
                PortDef::new("height", DataType::Scalar),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn text_to_path(id: NodeId) -> Self {
        Self {
            id,
            name: "Text to Path".into(),
            op: NodeOp::TextToPath,
            inputs: vec![
                PortDef::new("text", DataType::Text)
                    .with_description("Input text instance"),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn portal_send(id: NodeId, label: String) -> Self {
        Self {
            id,
            name: format!("Send: {}", label),
            op: NodeOp::PortalSend { label },
            inputs: vec![
                PortDef::new("value", DataType::Any)
                    .with_description("Value to send"),
            ],
            outputs: vec![
                PortDef::new("through", DataType::Any)
                    .with_description("Pass-through of the input"),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }

    pub fn portal_receive(id: NodeId, label: String) -> Self {
        Self {
            id,
            name: format!("Receive: {}", label),
            op: NodeOp::PortalReceive { label },
            inputs: vec![],
            outputs: vec![
                PortDef::new("value", DataType::Any)
                    .with_description("Received value"),
            ],
            position: [0.0, 0.0],
            generation: 0,
            version: 0,
            input_visibility: Vec::new(),
            output_visibility: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_defaults_to_zero_on_deserialize() {
        // Simulate a project file saved before versioning was added.
        let json = r#"{
            "id": 1,
            "name": "Circle",
            "op": "Circle",
            "inputs": [],
            "outputs": [],
            "position": [0.0, 0.0],
            "generation": 0
        }"#;
        let node: NodeDef = serde_json::from_str(json).unwrap();
        assert_eq!(node.version, 0);
    }

    #[test]
    fn builtin_factory_sets_current_version() {
        let node = NodeDef::circle(NodeId(1));
        assert_eq!(node.version, 0);
        assert_eq!(node.version, node.op.current_version());
        assert!(!node.is_outdated());
    }

    #[test]
    fn user_defined_ops_not_versioned() {
        let dsl = NodeDef::dsl_code(NodeId(1), String::new());
        assert_eq!(dsl.version, 0);
        assert_eq!(dsl.op.current_version(), 0);
        assert!(!dsl.is_outdated());

        let map = NodeDef::map(NodeId(2));
        assert_eq!(map.version, 0);
        assert!(!map.is_outdated());

        let gen = NodeDef::generate(NodeId(3));
        assert_eq!(gen.version, 0);
        assert!(!gen.is_outdated());

        let gi = NodeDef::graph_input(NodeId(4), "x".into(), DataType::Scalar);
        assert_eq!(gi.version, 0);
        assert!(!gi.is_outdated());

        let go = NodeDef::graph_output(NodeId(5), "y".into(), DataType::Scalar);
        assert_eq!(go.version, 0);
        assert!(!go.is_outdated());
    }

    #[test]
    fn outdated_detection() {
        // Simulate a node whose op has been bumped to version 1,
        // but the saved node is still at version 0.
        let mut node = NodeDef::circle(NodeId(1));
        // Currently Circle is at version 0, so not outdated.
        assert!(!node.is_outdated());

        // If we pretend the current version were higher (as if we bumped it),
        // a node stuck at 0 would be outdated. We test this by manually
        // setting the version below a hypothetical current_version.
        // Since current_version() returns 0 for Circle right now, we test
        // the logic with a node that has version < current_version by using
        // a concrete scenario: set version to 0, op to one that returns > 0.
        // For now, just verify the method works with mismatched values.
        node.version = 0;
        // Patch: all built-in ops return 0 currently, so this is not outdated.
        assert!(!node.is_outdated());
    }

    #[test]
    fn outdated_detection_after_version_bump() {
        // Simulate a node saved at version 0 loaded into code where
        // current_version is now 2 (e.g. PathBoolean gained tolerance + parts port).
        let mut node = NodeDef::path_boolean(NodeId(1));
        assert_eq!(node.op.current_version(), 2);
        assert_eq!(node.version, 2);
        assert!(!node.is_outdated());

        // A node saved before the bump would have version 0.
        node.version = 0;
        assert!(node.is_outdated());
    }

    #[test]
    fn version_survives_serialization_roundtrip() {
        // Simulate a node at version 5 (future bumped version).
        let mut node = NodeDef::circle(NodeId(1));
        node.version = 5;
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: NodeDef = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.version, 5);
    }

    #[test]
    fn port_visibility_defaults() {
        // SetStroke has hidden advanced ports.
        let mut node = NodeDef::set_stroke(NodeId(1));
        node.init_visibility();
        // geometry, color, width are visible (indices 0, 1, 2)
        assert!(node.input_visibility[0]); // geometry
        assert!(node.input_visibility[1]); // color
        assert!(node.input_visibility[2]); // width
        // cap, join, miter_limit, dash_offset, tolerance are hidden
        assert!(!node.input_visibility[3]); // cap
        assert!(!node.input_visibility[4]); // join
        assert!(!node.input_visibility[5]); // miter_limit
        assert!(!node.input_visibility[6]); // dash_offset
        assert!(!node.input_visibility[7]); // tolerance
        // output is visible
        assert!(node.output_visibility[0]);
    }

    #[test]
    fn port_visibility_mapping() {
        let mut node = NodeDef::set_stroke(NodeId(1));
        node.init_visibility();
        // 3 visible inputs: geometry(0), color(1), width(2)
        assert_eq!(node.visible_input_count(), 3);
        assert_eq!(node.visible_input_to_port(0), Some(0)); // geometry
        assert_eq!(node.visible_input_to_port(1), Some(1)); // color
        assert_eq!(node.visible_input_to_port(2), Some(2)); // width
        assert_eq!(node.visible_input_to_port(3), None); // out of range

        // Reverse mapping
        assert_eq!(node.port_to_visible_input(0), Some(0)); // geometry → visible 0
        assert_eq!(node.port_to_visible_input(2), Some(2)); // width → visible 2
        assert_eq!(node.port_to_visible_input(3), None);    // cap → hidden
        assert_eq!(node.port_to_visible_input(7), None);    // tolerance → hidden

        // Show cap (index 3)
        node.input_visibility[3] = true;
        assert_eq!(node.visible_input_count(), 4);
        assert_eq!(node.visible_input_to_port(3), Some(3)); // cap
        assert_eq!(node.port_to_visible_input(3), Some(3)); // cap → visible 3
    }

    #[test]
    fn port_visibility_sync_on_deserialize() {
        // Simulate a node saved without visibility (older format).
        let json = r#"{
            "id": 1,
            "name": "Circle",
            "op": "Circle",
            "inputs": [
                {"name": "radius", "data_type": "Scalar", "description": "", "default_value": null, "expression": null}
            ],
            "outputs": [
                {"name": "path", "data_type": "Path", "description": "", "default_value": null, "expression": null}
            ],
            "position": [0.0, 0.0],
            "generation": 0
        }"#;
        let mut node: NodeDef = serde_json::from_str(json).unwrap();
        assert!(node.input_visibility.is_empty()); // no visibility data in old format
        node.sync_visibility();
        assert_eq!(node.input_visibility.len(), 1);
        assert!(node.input_visibility[0]); // defaults to true
        assert_eq!(node.output_visibility.len(), 1);
        assert!(node.output_visibility[0]);
    }

    #[test]
    fn set_style_factory() {
        let mut node = NodeDef::set_style(NodeId(1));
        node.init_visibility();
        // 12 input ports
        assert_eq!(node.inputs.len(), 12);
        assert_eq!(node.inputs[0].name, "path");
        assert_eq!(node.inputs[1].name, "fill_color");
        assert_eq!(node.inputs[2].name, "fill_opacity");
        assert_eq!(node.inputs[3].name, "has_fill");
        assert_eq!(node.inputs[4].name, "stroke_color");
        assert_eq!(node.inputs[5].name, "stroke_width");
        assert_eq!(node.inputs[6].name, "stroke_opacity");
        assert_eq!(node.inputs[7].name, "has_stroke");
        assert_eq!(node.inputs[8].name, "cap");
        assert_eq!(node.inputs[9].name, "join");
        assert_eq!(node.inputs[10].name, "miter_limit");
        assert_eq!(node.inputs[11].name, "dash_offset");
        // 1 output port
        assert_eq!(node.outputs.len(), 1);
        assert_eq!(node.outputs[0].name, "output");

        // Visible by default: path, fill_color, stroke_color, stroke_width
        assert_eq!(node.visible_input_count(), 4);
        assert!(node.input_visibility[0]);  // path
        assert!(node.input_visibility[1]);  // fill_color
        assert!(!node.input_visibility[2]); // fill_opacity (hidden)
        assert!(!node.input_visibility[3]); // has_fill (hidden)
        assert!(node.input_visibility[4]);  // stroke_color
        assert!(node.input_visibility[5]);  // stroke_width
        assert!(!node.input_visibility[6]); // stroke_opacity (hidden)
        // version
        assert_eq!(node.version, 1);
        assert!(!node.is_outdated());
    }

    #[test]
    fn hidden_port_builder() {
        let port = PortDef::new("test", DataType::Scalar).hidden();
        assert!(!port.visible_by_default);

        let visible_port = PortDef::new("test2", DataType::Scalar);
        assert!(visible_port.visible_by_default);
    }

    #[test]
    fn tolerance_port_nodes_at_version_1() {
        let pb = NodeDef::path_boolean(NodeId(1));
        assert_eq!(pb.version, 2);
        assert_eq!(pb.op.current_version(), 2);
        assert!(!pb.is_outdated());
        assert!(pb.inputs.iter().any(|p| p.name == "tolerance"));
        assert!(pb.outputs.iter().any(|p| p.name == "parts"));

        let rp = NodeDef::resample_path(NodeId(2));
        assert_eq!(rp.version, 1);
        assert_eq!(rp.op.current_version(), 1);
        assert!(!rp.is_outdated());
        assert!(rp.inputs.iter().any(|p| p.name == "tolerance"));

        let cp = NodeDef::copy_to_points(NodeId(3));
        assert_eq!(cp.version, 1);
        assert_eq!(cp.op.current_version(), 1);
        assert!(!cp.is_outdated());
        assert!(cp.inputs.iter().any(|p| p.name == "tolerance"));

        let stp = NodeDef::stroke_to_path(NodeId(4));
        assert_eq!(stp.version, 1);
        assert_eq!(stp.op.current_version(), 1);
        assert!(!stp.is_outdated());
        assert!(stp.inputs.iter().any(|p| p.name == "tolerance"));

        let ss = NodeDef::set_stroke(NodeId(5));
        assert_eq!(ss.version, 1);
        assert_eq!(ss.op.current_version(), 1);
        assert!(!ss.is_outdated());
        assert!(ss.inputs.iter().any(|p| p.name == "tolerance"));
    }
}
