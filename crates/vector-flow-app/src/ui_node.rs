use egui::Color32;
use serde::{Deserialize, Serialize};
use vector_flow_core::node::{NodeDef, NodeOp};
use vector_flow_core::types::{DataType, NodeId as CoreNodeId};

/// Display-only metadata stored in the snarl graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiNode {
    pub core_id: CoreNodeId,
    pub display_name: String,
    #[serde(with = "color32_serde")]
    pub color: Color32,
    pub pinned: bool,
}

mod color32_serde {
    use egui::Color32;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Rgba(u8, u8, u8, u8);

    pub fn serialize<S: Serializer>(c: &Color32, s: S) -> Result<S::Ok, S::Error> {
        Rgba(c.r(), c.g(), c.b(), c.a()).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Color32, D::Error> {
        let Rgba(r, g, b, a) = Rgba::deserialize(d)?;
        Ok(Color32::from_rgba_unmultiplied(r, g, b, a))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCategory {
    Generators,
    Transforms,
    PathOps,
    Styling,
    Color,
    Text,
    Code,
    Utility,
    GraphIO,
}

impl NodeCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Generators => "Generators",
            Self::Transforms => "Transforms",
            Self::PathOps => "Path Ops",
            Self::Styling => "Styling",
            Self::Color => "Color",
            Self::Text => "Text",
            Self::Code => "Code",
            Self::Utility => "Utility",
            Self::GraphIO => "Graph I/O",
        }
    }
}

pub struct CatalogEntry {
    pub label: &'static str,
    pub category: NodeCategory,
    pub factory: fn(CoreNodeId) -> NodeDef,
    pub color: Color32,
}

/// Color per category.
fn cat_color(cat: NodeCategory) -> Color32 {
    match cat {
        NodeCategory::Generators => Color32::from_rgb(80, 160, 80),
        NodeCategory::Transforms => Color32::from_rgb(80, 120, 200),
        NodeCategory::PathOps => Color32::from_rgb(200, 120, 60),
        NodeCategory::Styling => Color32::from_rgb(180, 80, 180),
        NodeCategory::Color => Color32::from_rgb(220, 100, 200),
        NodeCategory::Text => Color32::from_rgb(100, 180, 220),
        NodeCategory::Code => Color32::from_rgb(120, 200, 160),
        NodeCategory::Utility => Color32::from_rgb(140, 140, 140),
        NodeCategory::GraphIO => Color32::from_rgb(200, 200, 80),
    }
}

/// Color for a DataType (used on pins).
pub fn data_type_color(dt: DataType) -> Color32 {
    match dt {
        DataType::Scalar => Color32::from_rgb(120, 200, 120),
        DataType::Vec2 => Color32::from_rgb(120, 180, 220),
        DataType::Points => Color32::from_rgb(80, 200, 200),
        DataType::Path => Color32::from_rgb(220, 180, 80),
        DataType::Paths => Color32::from_rgb(220, 160, 60),
        DataType::Shape => Color32::from_rgb(220, 100, 100),
        DataType::Shapes => Color32::from_rgb(200, 80, 80),
        DataType::Transform => Color32::from_rgb(100, 100, 220),
        DataType::Color => Color32::from_rgb(220, 80, 220),
        DataType::Bool => Color32::from_rgb(200, 200, 200),
        DataType::Int => Color32::from_rgb(100, 200, 160),
        DataType::Scalars => Color32::from_rgb(100, 180, 100),
        DataType::Colors => Color32::from_rgb(200, 60, 200),
        DataType::Ints => Color32::from_rgb(80, 180, 140),
        DataType::Image => Color32::from_rgb(160, 120, 200),
        DataType::Text => Color32::from_rgb(100, 180, 220),
        DataType::Any => Color32::from_rgb(180, 180, 180),
    }
}

macro_rules! entry {
    ($label:expr, $cat:expr, $factory:expr) => {
        CatalogEntry {
            label: $label,
            category: $cat,
            factory: $factory,
            color: cat_color($cat),
        }
    };
}

pub fn node_catalog() -> Vec<CatalogEntry> {
    use NodeCategory::*;
    vec![
        // Generators
        entry!("Circle", Generators, NodeDef::circle),
        entry!("Line", Generators, NodeDef::line),
        CatalogEntry {
            label: "Load Image",
            category: Generators,
            factory: |id| NodeDef::load_image(id, String::new()),
            color: cat_color(Generators),
        },
        entry!("Point Grid", Generators, NodeDef::point_grid),
        entry!("Rectangle", Generators, NodeDef::rectangle),
        entry!("Regular Polygon", Generators, NodeDef::regular_polygon),
        entry!("Scatter Points", Generators, NodeDef::scatter_points),
        CatalogEntry {
            label: "SVG Path",
            category: Generators,
            factory: |id| NodeDef::svg_path(id, String::new()),
            color: cat_color(Generators),
        },
        // Transforms
        entry!("Apply Transform", Transforms, NodeDef::apply_transform),
        entry!("Rotate", Transforms, NodeDef::rotate),
        entry!("Scale", Transforms, NodeDef::scale),
        entry!("Translate", Transforms, NodeDef::translate),
        // Path Ops
        entry!("Path Difference", PathOps, NodeDef::path_difference),
        entry!("Path Intersect", PathOps, NodeDef::path_intersect),
        entry!("Path Offset", PathOps, NodeDef::path_offset),
        entry!("Path Reverse", PathOps, NodeDef::path_reverse),
        entry!("Path Subdivide", PathOps, NodeDef::path_subdivide),
        entry!("Path Union", PathOps, NodeDef::path_union),
        entry!("Resample Path", PathOps, NodeDef::resample_path),
        // Styling
        entry!("Set Fill", Styling, NodeDef::set_fill),
        entry!("Set Stroke", Styling, NodeDef::set_stroke),
        entry!("Stroke to Path", Styling, NodeDef::stroke_to_path),
        // Color
        entry!("Adjust Alpha", Color, NodeDef::adjust_alpha),
        entry!("Adjust Hue", Color, NodeDef::adjust_hue),
        entry!("Adjust Lightness", Color, NodeDef::adjust_lightness),
        entry!("Adjust Luminance", Color, NodeDef::adjust_luminance),
        entry!("Adjust Saturation", Color, NodeDef::adjust_saturation),
        CatalogEntry {
            label: "Color Parse",
            category: Color,
            factory: |id| NodeDef::color_parse(id, "#FFFFFF".into()),
            color: cat_color(Color),
        },
        entry!("Grayscale", Color, NodeDef::grayscale),
        entry!("Invert Color", Color, NodeDef::invert_color),
        entry!("Mix Colors", Color, NodeDef::mix_colors),
        // Text
        CatalogEntry {
            label: "Text",
            category: Text,
            factory: |id| NodeDef::text(id, "Hello World".into()),
            color: cat_color(Text),
        },
        entry!("Text to Path", Text, NodeDef::text_to_path),
        // Code
        entry!("Map", Code, NodeDef::map),
        CatalogEntry {
            label: "VFS Code",
            category: Code,
            factory: |id| NodeDef::dsl_code(id, String::new()),
            color: cat_color(Code),
        },
        // Utility
        entry!("Constant Color", Utility, NodeDef::const_color),
        entry!("Constant Int", Utility, NodeDef::const_int),
        entry!("Constant Scalar", Utility, NodeDef::const_scalar),
        entry!("Constant Vec2", Utility, NodeDef::const_vec2),
        entry!("Copy to Points", Utility, NodeDef::copy_to_points),
        entry!("Duplicate", Utility, NodeDef::duplicate),
        entry!("Merge", Utility, NodeDef::merge),
        CatalogEntry {
            label: "Portal Receive",
            category: Utility,
            factory: |id| NodeDef::portal_receive(id, "net".into()),
            color: cat_color(Utility),
        },
        CatalogEntry {
            label: "Portal Send",
            category: Utility,
            factory: |id| NodeDef::portal_send(id, "net".into()),
            color: cat_color(Utility),
        },
        // Graph I/O
        CatalogEntry {
            label: "Graph Output",
            category: GraphIO,
            factory: |id| NodeDef::graph_output(id, "output".into(), DataType::Any),
            color: cat_color(GraphIO),
        },
    ]
}

/// Determine NodeOp label for display.
pub fn node_op_label(op: &NodeOp) -> &'static str {
    match op {
        NodeOp::Circle => "Circle",
        NodeOp::Rectangle => "Rectangle",
        NodeOp::RegularPolygon => "Regular Polygon",
        NodeOp::Line => "Line",
        NodeOp::PointGrid => "Point Grid",
        NodeOp::ScatterPoints => "Scatter Points",
        NodeOp::Translate => "Translate",
        NodeOp::Rotate => "Rotate",
        NodeOp::Scale => "Scale",
        NodeOp::ApplyTransform => "Apply Transform",
        NodeOp::PathUnion => "Path Union",
        NodeOp::PathIntersect => "Path Intersect",
        NodeOp::PathDifference => "Path Difference",
        NodeOp::PathOffset => "Path Offset",
        NodeOp::PathSubdivide => "Path Subdivide",
        NodeOp::PathReverse => "Path Reverse",
        NodeOp::ResamplePath => "Resample Path",
        NodeOp::SetFill => "Set Fill",
        NodeOp::SetStroke { .. } => "Set Stroke",
        NodeOp::StrokeToPath { .. } => "Stroke to Path",
        NodeOp::AdjustHue => "Adjust Hue",
        NodeOp::AdjustSaturation => "Adjust Saturation",
        NodeOp::AdjustLightness => "Adjust Lightness",
        NodeOp::AdjustLuminance => "Adjust Luminance",
        NodeOp::InvertColor => "Invert Color",
        NodeOp::Grayscale => "Grayscale",
        NodeOp::MixColors => "Mix Colors",
        NodeOp::AdjustAlpha => "Adjust Alpha",
        NodeOp::ColorParse { .. } => "Color Parse",
        NodeOp::SvgPath { .. } => "SVG Path",
        NodeOp::ConstScalar => "Constant Scalar",
        NodeOp::ConstInt => "Constant Int",
        NodeOp::ConstVec2 => "Constant Vec2",
        NodeOp::ConstColor => "Constant Color",
        NodeOp::PortalSend { .. } => "Portal Send",
        NodeOp::PortalReceive { .. } => "Portal Receive",
        NodeOp::Merge => "Merge",
        NodeOp::Duplicate => "Duplicate",
        NodeOp::CopyToPoints => "Copy to Points",
        NodeOp::LoadImage { .. } => "Load Image",
        NodeOp::Text { .. } => "Text",
        NodeOp::TextToPath => "Text to Path",
        NodeOp::DslCode { .. } => "VFS Code",
        NodeOp::Map { .. } => "Map",
        NodeOp::GraphInput { .. } => "Graph Input",
        NodeOp::GraphOutput { .. } => "Graph Output",
    }
}
