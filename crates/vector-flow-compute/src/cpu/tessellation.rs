use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers,
};

use vector_flow_core::compute::TessellationOutput;
use vector_flow_core::types::PathData;

use super::path_ops;

/// Tessellate a PathData into triangles using lyon.
pub fn tessellate_path_lyon(path: &PathData, fill: bool, tolerance: f32) -> TessellationOutput {
    if !fill || path.verbs.is_empty() {
        return TessellationOutput {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
    }

    let lyon_path = path_ops::build_lyon_path(path);

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

#[cfg(test)]
mod tests {
    use super::*;
    use vector_flow_core::types::{PathVerb, Point};

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
