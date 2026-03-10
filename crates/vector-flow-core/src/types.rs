use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Affine2, Vec2};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// IDs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkBoxId(pub u64);

// ---------------------------------------------------------------------------
// Geometry primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, Serialize, Deserialize)]
#[repr(C)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable, Serialize, Deserialize)]
#[repr(C)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const TRANSPARENT: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LineJoin {
    Miter(f32),
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeStyle {
    pub color: Color,
    pub width: f32,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub dash_array: Vec<f32>,
    pub dash_offset: f32,
}

// ---------------------------------------------------------------------------
// Path types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathVerb {
    MoveTo(Point),
    LineTo(Point),
    QuadTo { ctrl: Point, to: Point },
    CubicTo { ctrl1: Point, ctrl2: Point, to: Point },
    Close,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathData {
    pub verbs: Vec<PathVerb>,
    pub closed: bool,
}

impl PathData {
    pub fn new() -> Self {
        Self { verbs: Vec::new(), closed: false }
    }
}

impl Default for PathData {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct Shape {
    pub path: Arc<PathData>,
    pub fill: Option<Color>,
    pub stroke: Option<StrokeStyle>,
    pub transform: Affine2,
}

// ---------------------------------------------------------------------------
// Image types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>, // RGBA8, row-major, top-to-bottom
    pub source_path: String,
}

#[derive(Debug, Clone)]
pub struct ImageInstance {
    pub image: Arc<ImageData>,
    pub transform: Affine2,
    pub opacity: f32,
}

// ---------------------------------------------------------------------------
// Text types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub font_family: String,
    pub font_path: String,
    pub font_size: f64,
    pub font_weight: u16,
    pub font_style: FontStyle,
    pub letter_spacing: f64,
    pub line_height: f64,
    pub alignment: TextAlignment,
    pub wrap: bool,
    pub box_width: f64,
    pub box_height: f64,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_path: String::new(),
            font_size: 24.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            letter_spacing: 0.0,
            line_height: 1.2,
            alignment: TextAlignment::Left,
            wrap: true,
            box_width: 0.0,
            box_height: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

#[derive(Debug, Clone)]
pub struct TextLayout {
    pub glyphs: Vec<PositionedGlyph>,
    pub bounds: (f32, f32),
    pub font_data: Arc<Vec<u8>>,
    pub font_index: u32,
}

#[derive(Debug, Clone)]
pub struct TextInstance {
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
    pub transform: Affine2,
    pub opacity: f32,
    pub layout: Arc<TextLayout>,
}

// ---------------------------------------------------------------------------
// SoA Point Batch (SIMD-friendly)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct PointBatch {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

impl PointBatch {
    pub fn new() -> Self {
        Self { xs: Vec::new(), ys: Vec::new() }
    }

    pub fn len(&self) -> usize {
        debug_assert_eq!(self.xs.len(), self.ys.len());
        self.xs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn from_points(points: &[Point]) -> Self {
        let mut xs = Vec::with_capacity(points.len());
        let mut ys = Vec::with_capacity(points.len());
        for p in points {
            xs.push(p.x);
            ys.push(p.y);
        }
        Self { xs, ys }
    }

    pub fn to_points(&self) -> Vec<Point> {
        debug_assert_eq!(self.xs.len(), self.ys.len());
        self.xs
            .iter()
            .zip(self.ys.iter())
            .map(|(&x, &y)| Point { x, y })
            .collect()
    }
}

impl Default for PointBatch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// NodeData — what flows through edges
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum NodeData {
    // Single values
    Scalar(f64),
    Vec2(Vec2),
    Path(Arc<PathData>),
    Shape(Arc<Shape>),
    Transform(Affine2),
    Color(Color),
    Bool(bool),
    Int(i64),
    // Batch values
    Points(Arc<PointBatch>),
    Scalars(Arc<Vec<f64>>),
    Colors(Arc<Vec<Color>>),
    Ints(Arc<Vec<i64>>),
    Paths(Arc<Vec<PathData>>),
    Shapes(Arc<Vec<Shape>>),
    Image(Arc<ImageInstance>),
    Text(Arc<TextInstance>),
    /// Bundle of heterogeneous data items (produced by Merge with mixed types).
    Mixed(Arc<Vec<NodeData>>),
}

impl NodeData {
    pub fn data_type(&self) -> DataType {
        match self {
            NodeData::Scalar(_) => DataType::Scalar,
            NodeData::Vec2(_) => DataType::Vec2,
            NodeData::Path(_) => DataType::Path,
            NodeData::Shape(_) => DataType::Shape,
            NodeData::Transform(_) => DataType::Transform,
            NodeData::Color(_) => DataType::Color,
            NodeData::Bool(_) => DataType::Bool,
            NodeData::Int(_) => DataType::Int,
            NodeData::Points(_) => DataType::Points,
            NodeData::Scalars(_) => DataType::Scalars,
            NodeData::Colors(_) => DataType::Colors,
            NodeData::Ints(_) => DataType::Ints,
            NodeData::Paths(_) => DataType::Paths,
            NodeData::Shapes(_) => DataType::Shapes,
            NodeData::Image(_) => DataType::Image,
            NodeData::Text(_) => DataType::Text,
            NodeData::Mixed(_) => DataType::Any,
        }
    }
}

// ---------------------------------------------------------------------------
// DataType — port type descriptors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataType {
    Scalar,
    Vec2,
    Points,
    Path,
    Paths,
    Shape,
    Shapes,
    Transform,
    Color,
    Bool,
    Int,
    Scalars,
    Colors,
    Ints,
    Image,
    Text,
    Any,
}

impl DataType {
    /// Returns true if a value of type `self` can be used where `target` is expected.
    pub fn can_promote_to(&self, target: &DataType) -> bool {
        if *target == DataType::Any || *self == DataType::Any || *self == *target {
            return true;
        }
        matches!(
            (self, target),
            (DataType::Path, DataType::Paths)
                | (DataType::Path, DataType::Shape)
                | (DataType::Shape, DataType::Path)
                | (DataType::Shape, DataType::Shapes)
                | (DataType::Shapes, DataType::Path)
                | (DataType::Paths, DataType::Path)
                | (DataType::Scalar, DataType::Vec2)
                | (DataType::Int, DataType::Scalar)
                | (DataType::Scalar, DataType::Int)
                | (DataType::Points, DataType::Vec2)
                | (DataType::Scalars, DataType::Scalar)
                | (DataType::Ints, DataType::Int)
                | (DataType::Colors, DataType::Color)
        )
    }
}

// ---------------------------------------------------------------------------
// EvalContext — global evaluation context (time, project settings, etc.)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EvalContext {
    pub frame: u64,
    pub time_secs: f32,
    pub fps: f32,
    /// Base directory for resolving relative file paths (e.g. project dir).
    /// Empty string means use current working directory.
    pub project_dir: String,
    /// Zoom-aware curve flattening tolerance.  Nodes that flatten curves
    /// (e.g. StrokeToPath) use this when no explicit tolerance is provided.
    pub tolerance: f32,
}

impl Default for EvalContext {
    fn default() -> Self {
        Self {
            frame: 0,
            time_secs: 0.0,
            fps: 30.0,
            project_dir: String::new(),
            tolerance: 0.5,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_batch_len_and_conversion() {
        let points = vec![
            Point { x: 1.0, y: 2.0 },
            Point { x: 3.0, y: 4.0 },
            Point { x: 5.0, y: 6.0 },
        ];
        let batch = PointBatch::from_points(&points);
        assert_eq!(batch.len(), 3);
        assert!(!batch.is_empty());
        assert_eq!(batch.to_points(), points);
    }

    #[test]
    fn empty_point_batch() {
        let batch = PointBatch::new();
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
    }

    #[test]
    fn data_type_promotion() {
        assert!(DataType::Path.can_promote_to(&DataType::Paths));
        assert!(DataType::Shape.can_promote_to(&DataType::Shapes));
        assert!(DataType::Scalar.can_promote_to(&DataType::Vec2));
        assert!(DataType::Int.can_promote_to(&DataType::Scalar));
        assert!(DataType::Scalar.can_promote_to(&DataType::Int));

        // Identity
        assert!(DataType::Scalar.can_promote_to(&DataType::Scalar));

        // Any accepts everything
        assert!(DataType::Path.can_promote_to(&DataType::Any));
        assert!(DataType::Bool.can_promote_to(&DataType::Any));

        // Path-family cross-promotions
        assert!(DataType::Shape.can_promote_to(&DataType::Path));
        assert!(DataType::Shapes.can_promote_to(&DataType::Path));
        assert!(DataType::Paths.can_promote_to(&DataType::Path));

        // Non-promotable
        assert!(!DataType::Bool.can_promote_to(&DataType::Scalar));
        assert!(!DataType::Vec2.can_promote_to(&DataType::Scalar));
    }

    #[test]
    fn node_data_type_round_trip() {
        let d = NodeData::Scalar(42.0);
        assert_eq!(d.data_type(), DataType::Scalar);

        let d = NodeData::Points(Arc::new(PointBatch::new()));
        assert_eq!(d.data_type(), DataType::Points);
    }
}
