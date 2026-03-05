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
        entry!("Rectangle", Generators, NodeDef::rectangle),
        entry!("Regular Polygon", Generators, NodeDef::regular_polygon),
        entry!("Line", Generators, NodeDef::line),
        entry!("Point Grid", Generators, NodeDef::point_grid),
        entry!("Scatter Points", Generators, NodeDef::scatter_points),
        // Transforms
        entry!("Translate", Transforms, NodeDef::translate),
        entry!("Rotate", Transforms, NodeDef::rotate),
        entry!("Scale", Transforms, NodeDef::scale),
        entry!("Apply Transform", Transforms, NodeDef::apply_transform),
        // Path Ops
        entry!("Path Union", PathOps, NodeDef::path_union),
        entry!("Path Intersect", PathOps, NodeDef::path_intersect),
        entry!("Path Difference", PathOps, NodeDef::path_difference),
        entry!("Path Offset", PathOps, NodeDef::path_offset),
        entry!("Path Subdivide", PathOps, NodeDef::path_subdivide),
        entry!("Path Reverse", PathOps, NodeDef::path_reverse),
        entry!("Resample Path", PathOps, NodeDef::resample_path),
        // Styling
        entry!("Set Fill", Styling, NodeDef::set_fill),
        entry!("Set Stroke", Styling, NodeDef::set_stroke),
        // Utility
        entry!("Merge", Utility, NodeDef::merge),
        entry!("Duplicate", Utility, NodeDef::duplicate),
        entry!("Copy to Points", Utility, NodeDef::copy_to_points),
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
        NodeOp::SetStroke => "Set Stroke",
        NodeOp::Merge => "Merge",
        NodeOp::Duplicate => "Duplicate",
        NodeOp::CopyToPoints => "Copy to Points",
        NodeOp::DslCode { .. } => "DSL Code",
        NodeOp::GraphInput { .. } => "Graph Input",
        NodeOp::GraphOutput { .. } => "Graph Output",
    }
}
