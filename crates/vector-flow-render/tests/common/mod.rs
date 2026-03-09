#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::Arc;

use vector_flow_compute::CpuBackend;
use vector_flow_core::graph::Graph;
use vector_flow_core::node::{NodeDef, NodeOp, PortId, PortIndex};
use vector_flow_core::scheduler::{EvalResult, Scheduler};
use vector_flow_core::types::{EvalContext, NodeId};
use vector_flow_render::{CollectedScene, PreparedScene, collect_scene, prepare_scene_full};

/// Headless test harness for the graph → evaluate → collect scene pipeline.
pub struct SceneTest {
    pub graph: Graph,
    scheduler: Scheduler,
    ctx: EvalContext,
}

impl SceneTest {
    pub fn new() -> Self {
        let backend = CpuBackend::new().expect("CpuBackend::new() failed");
        let scheduler = Scheduler::new(Arc::new(backend));
        Self {
            graph: Graph::new(),
            scheduler,
            ctx: EvalContext::default(),
        }
    }

    /// Set the current frame and compute time_secs from fps.
    pub fn set_frame(&mut self, frame: u64) {
        self.ctx.frame = frame;
        self.ctx.time_secs = frame as f32 / self.ctx.fps;
    }

    /// Evaluate the graph and return the raw result.
    pub fn evaluate(&mut self) -> EvalResult {
        self.scheduler
            .evaluate(&mut self.graph, &self.ctx, None)
            .expect("evaluation failed")
    }

    /// Evaluate and collect scene using GraphOutput visibility filter
    /// (only GraphOutput nodes shown; if none exist, show all).
    pub fn collect(&mut self) -> CollectedScene {
        let result = self.evaluate();
        let graph_outputs: HashSet<NodeId> = self
            .graph
            .nodes()
            .filter(|n| matches!(n.op, NodeOp::GraphOutput { .. }))
            .map(|n| n.id)
            .collect();
        let visible = if graph_outputs.is_empty() {
            None
        } else {
            Some(graph_outputs)
        };
        collect_scene(&result, visible.as_ref())
    }

    /// Evaluate and collect ALL nodes (no visibility filter).
    pub fn collect_all(&mut self) -> CollectedScene {
        let result = self.evaluate();
        collect_scene(&result, None)
    }

    /// Collect and then tessellate into GPU-ready geometry.
    pub fn prepare(&mut self) -> PreparedScene {
        let scene = self.collect();
        prepare_scene_full(&scene, 0.5)
    }

    /// Collect all and then tessellate into GPU-ready geometry.
    pub fn prepare_all(&mut self) -> PreparedScene {
        let scene = self.collect_all();
        prepare_scene_full(&scene, 0.5)
    }
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

pub fn assert_no_errors(result: &EvalResult) {
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
}

pub fn assert_shape_count(scene: &CollectedScene, expected: usize) {
    assert_eq!(
        scene.shapes.len(),
        expected,
        "expected {} shapes, got {}",
        expected,
        scene.shapes.len()
    );
}

pub fn assert_image_count(scene: &CollectedScene, expected: usize) {
    assert_eq!(
        scene.images.len(),
        expected,
        "expected {} images, got {}",
        expected,
        scene.images.len()
    );
}

pub fn assert_text_count(scene: &CollectedScene, expected: usize) {
    assert_eq!(
        scene.texts.len(),
        expected,
        "expected {} texts, got {}",
        expected,
        scene.texts.len()
    );
}

// ---------------------------------------------------------------------------
// Graph-building helpers
// ---------------------------------------------------------------------------

/// Add a node to the graph and return its NodeId.
pub fn add_node(test: &mut SceneTest, node_fn: fn(NodeId) -> NodeDef) -> NodeId {
    let node = node_fn(NodeId(0)); // id will be overwritten by add_node
    test.graph.add_node(node)
}

/// Connect output port `from_port` of `from_node` to input port `to_port` of `to_node`.
pub fn connect(
    test: &mut SceneTest,
    from_node: NodeId,
    from_port: usize,
    to_node: NodeId,
    to_port: usize,
) {
    test.graph
        .connect(
            PortId { node: from_node, port: PortIndex(from_port) },
            PortId { node: to_node, port: PortIndex(to_port) },
        )
        .expect("connect failed");
}
