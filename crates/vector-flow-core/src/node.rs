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
}

impl PortDef {
    pub fn new(name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            data_type,
            description: String::new(),
            default_value: None,
            expression: None,
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
    PathUnion,
    PathIntersect,
    PathDifference,
    PathOffset,
    PathSubdivide,
    PathReverse,
    ResamplePath,
    // Styling
    SetFill,
    SetStroke { dash_pattern: String },
    StrokeToPath { dash_pattern: String },
    // Color operations
    AdjustHue,
    AdjustSaturation,
    AdjustLightness,
    AdjustLuminance,
    InvertColor,
    Grayscale,
    MixColors,
    SetAlpha,
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
    Merge,
    Duplicate,

    // DSL
    DslCode {
        source: String,
        /// Port definitions for the compiler (name, DataType).
        /// Kept in sync with NodeDef.inputs / NodeDef.outputs by the UI.
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
    GraphOutput { name: String, data_type: DataType },
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

    /// Whether this node supports a variable number of inputs.
    pub fn is_variadic(&self) -> bool {
        matches!(self.op, NodeOp::PathUnion | NodeOp::Merge)
    }

    /// Add another variadic input port. Returns the new port index.
    pub fn add_variadic_input(&mut self) -> Option<usize> {
        if !self.is_variadic() {
            return None;
        }
        let idx = self.inputs.len();
        let name = variadic_port_name(idx);
        let desc = match self.op {
            NodeOp::Merge => "Input",
            _ => "Path input",
        };
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
                PortDef::new("segments", DataType::Int)
                    .with_default(ParamValue::Int(64))
                    .with_description("Number of segments for approximation"),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
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
        }
    }

    pub fn set_fill(id: NodeId) -> Self {
        Self {
            id,
            name: "Set Fill".into(),
            op: NodeOp::SetFill,
            inputs: vec![
                PortDef::new("shape", DataType::Shape).with_description("Input shape"),
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Fill color"),
            ],
            outputs: vec![PortDef::new("shape", DataType::Shape)],
            position: [0.0, 0.0],
            generation: 0,
        }
    }

    pub fn set_stroke(id: NodeId) -> Self {
        Self {
            id,
            name: "Set Stroke".into(),
            op: NodeOp::SetStroke { dash_pattern: String::new() },
            inputs: vec![
                PortDef::new("shape", DataType::Shape).with_description("Input shape"),
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([0.0, 0.0, 0.0, 1.0]))
                    .with_description("Stroke color"),
                PortDef::new("width", DataType::Scalar)
                    .with_default(ParamValue::Float(2.0))
                    .with_description("Stroke width"),
                PortDef::new("cap", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("End cap: 0=Butt, 1=Round, 2=Square"),
                PortDef::new("join", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Line join: 0=Miter, 1=Round, 2=Bevel"),
                PortDef::new("miter_limit", DataType::Scalar)
                    .with_default(ParamValue::Float(4.0))
                    .with_description("Miter limit (only for Miter join)"),
                PortDef::new("dash_offset", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Dash pattern offset"),
            ],
            outputs: vec![PortDef::new("shape", DataType::Shape)],
            position: [0.0, 0.0],
            generation: 0,
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
                    .with_description("End cap: 0=Butt, 1=Round, 2=Square"),
                PortDef::new("join", DataType::Int)
                    .with_default(ParamValue::Int(0))
                    .with_description("Line join: 0=Miter, 1=Round, 2=Bevel"),
                PortDef::new("miter_limit", DataType::Scalar)
                    .with_default(ParamValue::Float(4.0))
                    .with_description("Miter limit (only for Miter join)"),
                PortDef::new("dash_offset", DataType::Scalar)
                    .with_default(ParamValue::Float(0.0))
                    .with_description("Dash pattern offset"),
            ],
            outputs: vec![PortDef::new("path", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
        }
    }

    pub fn merge(id: NodeId) -> Self {
        Self {
            id,
            name: "Merge".into(),
            op: NodeOp::Merge,
            inputs: vec![
                PortDef::new("a", DataType::Any).with_description("First input"),
                PortDef::new("b", DataType::Any).with_description("Second input"),
            ],
            outputs: vec![PortDef::new("merged", DataType::Any)],
            position: [0.0, 0.0],
            generation: 0,
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
        }
    }

    pub fn dsl_code(id: NodeId, source: String) -> Self {
        Self {
            id,
            name: "DSL Code".into(),
            op: NodeOp::DslCode {
                source,
                script_inputs: Vec::new(),
                script_outputs: Vec::new(),
            },
            inputs: Vec::new(),
            outputs: Vec::new(),
            position: [0.0, 0.0],
            generation: 0,
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
        }
    }

    pub fn graph_output(id: NodeId, name: String, data_type: DataType) -> Self {
        Self {
            id,
            name: format!("Output: {}", name),
            op: NodeOp::GraphOutput {
                name: name.clone(),
                data_type,
            },
            inputs: vec![PortDef::new(name, data_type)],
            outputs: vec![],
            position: [0.0, 0.0],
            generation: 0,
        }
    }
}

impl NodeDef {
    pub fn path_union(id: NodeId) -> Self {
        Self {
            id,
            name: "Path Union".into(),
            op: NodeOp::PathUnion,
            inputs: vec![
                PortDef::new("a", DataType::Any).with_description("Path input"),
                PortDef::new("b", DataType::Any).with_description("Path input"),
            ],
            outputs: vec![PortDef::new("result", DataType::Shapes)],
            position: [0.0, 0.0],
            generation: 0,
        }
    }

    pub fn path_intersect(id: NodeId) -> Self {
        Self {
            id,
            name: "Path Intersect".into(),
            op: NodeOp::PathIntersect,
            inputs: vec![
                PortDef::new("a", DataType::Path).with_description("First path"),
                PortDef::new("b", DataType::Path).with_description("Second path"),
            ],
            outputs: vec![PortDef::new("result", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
        }
    }

    pub fn path_difference(id: NodeId) -> Self {
        Self {
            id,
            name: "Path Difference".into(),
            op: NodeOp::PathDifference,
            inputs: vec![
                PortDef::new("a", DataType::Path).with_description("Base path"),
                PortDef::new("b", DataType::Path).with_description("Path to subtract"),
            ],
            outputs: vec![PortDef::new("result", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
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
            ],
            outputs: vec![PortDef::new("result", DataType::Path)],
            position: [0.0, 0.0],
            generation: 0,
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
            ],
            outputs: vec![PortDef::new("points", DataType::Points)],
            position: [0.0, 0.0],
            generation: 0,
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
        }
    }

    pub fn set_alpha(id: NodeId) -> Self {
        Self {
            id,
            name: "Set Alpha".into(),
            op: NodeOp::SetAlpha,
            inputs: vec![
                PortDef::new("color", DataType::Color)
                    .with_default(ParamValue::Color([1.0, 1.0, 1.0, 1.0]))
                    .with_description("Input color"),
                PortDef::new("alpha", DataType::Scalar)
                    .with_default(ParamValue::Float(1.0))
                    .with_description("Alpha value (0..1)"),
            ],
            outputs: vec![PortDef::new("color", DataType::Color)],
            position: [0.0, 0.0],
            generation: 0,
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
        }
    }
}
