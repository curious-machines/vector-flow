use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers,
};

use vector_flow_core::compute::TessellationOutput;
use vector_flow_core::types::{PathData, PathVerb};

/// Tessellate a PathData into triangles using lyon.
pub fn tessellate_path_lyon(path: &PathData, fill: bool, tolerance: f32) -> TessellationOutput {
    if !fill || path.verbs.is_empty() {
        return TessellationOutput {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
    }

    let lyon_path = build_lyon_path(path);

    let mut geometry: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();

    let options = FillOptions::tolerance(tolerance);

    let result = tessellator.tessellate_path(
        &lyon_path,
        &options,
        &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| {
            let p = vertex.position();
            [p.x, p.y]
        }),
    );

    if result.is_err() {
        log::warn!("Tessellation failed, returning empty mesh");
        return TessellationOutput {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
    }

    TessellationOutput {
        vertices: geometry.vertices,
        indices: geometry.indices,
    }
}

fn build_lyon_path(path: &PathData) -> Path {
    let mut builder = Path::builder();
    let mut in_subpath = false;

    for v in &path.verbs {
        match *v {
            PathVerb::MoveTo(p) => {
                if in_subpath {
                    builder.end(false);
                }
                builder.begin(point(p.x, p.y));
                in_subpath = true;
            }
            PathVerb::LineTo(p) => {
                builder.line_to(point(p.x, p.y));
            }
            PathVerb::QuadTo { ctrl, to } => {
                builder.quadratic_bezier_to(point(ctrl.x, ctrl.y), point(to.x, to.y));
            }
            PathVerb::CubicTo { ctrl1, ctrl2, to } => {
                builder.cubic_bezier_to(
                    point(ctrl1.x, ctrl1.y),
                    point(ctrl2.x, ctrl2.y),
                    point(to.x, to.y),
                );
            }
            PathVerb::Close => {
                builder.end(true);
                in_subpath = false;
            }
        }
    }

    if in_subpath {
        builder.end(false);
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::types::Point;

    #[test]
    fn circle_tessellates_to_nonempty_mesh() {
        // Build a square as a simple closed polygon
        let path = PathData {
            verbs: vec![
                PathVerb::MoveTo(Point { x: -50.0, y: -50.0 }),
                PathVerb::LineTo(Point { x: 50.0, y: -50.0 }),
                PathVerb::LineTo(Point { x: 50.0, y: 50.0 }),
                PathVerb::LineTo(Point { x: -50.0, y: 50.0 }),
                PathVerb::Close,
            ],
            closed: true,
        };

        let output = tessellate_path_lyon(&path, true, 0.1);
        assert!(!output.vertices.is_empty(), "vertices should not be empty");
        assert!(!output.indices.is_empty(), "indices should not be empty");
        // A square should tessellate to 2 triangles = 6 indices
        assert_eq!(output.indices.len(), 6);
    }

    #[test]
    fn empty_path_returns_empty() {
        let path = PathData::new();
        let output = tessellate_path_lyon(&path, true, 0.1);
        assert!(output.vertices.is_empty());
        assert!(output.indices.is_empty());
    }
}
