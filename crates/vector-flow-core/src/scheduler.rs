use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use rayon::prelude::*;

use crate::compute::{ComputeBackend, NodeOutputs, ResolvedInputs};
use crate::error::ComputeError;
use crate::graph::Graph;
use crate::node::{NodeOp, ParamValue, PortIndex};
use crate::types::{NodeData, NodeId, TimeContext};

// ---------------------------------------------------------------------------
// EvalCache
// ---------------------------------------------------------------------------

/// Cache keyed by (NodeId, generation). Stores computed outputs per node.
#[derive(Debug, Default)]
pub struct EvalCache {
    entries: HashMap<(NodeId, u64), Vec<NodeData>>,
}

impl EvalCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, node_id: NodeId, generation: u64) -> Option<&Vec<NodeData>> {
        self.entries.get(&(node_id, generation))
    }

    pub fn insert(&mut self, node_id: NodeId, generation: u64, data: Vec<NodeData>) {
        self.entries.insert((node_id, generation), data);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

// ---------------------------------------------------------------------------
// EvalResult
// ---------------------------------------------------------------------------

/// Result of evaluating the entire graph.
pub struct EvalResult {
    pub outputs: HashMap<NodeId, Vec<NodeData>>,
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

pub struct Scheduler {
    backend: Arc<dyn ComputeBackend>,
    cache: RwLock<EvalCache>,
}

impl Scheduler {
    pub fn new(backend: Arc<dyn ComputeBackend>) -> Self {
        Self {
            backend,
            cache: RwLock::new(EvalCache::new()),
        }
    }

    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }

    /// Evaluate the full graph for a given time context.
    pub fn evaluate(
        &self,
        graph: &mut Graph,
        time_ctx: &TimeContext,
    ) -> Result<EvalResult, ComputeError> {
        let topo_order = graph
            .topological_sort()
            .map_err(|e| ComputeError::BackendError(e.to_string()))?;

        // Build portal send label → NodeId map.
        let portal_map = self.build_portal_map(graph);
        let has_portals = !portal_map.is_empty();

        let subgraphs = graph.independent_subgraphs();

        if has_portals {
            // Portal receives may depend on sends in other subgraphs,
            // so evaluate sequentially with a shared output map.
            // Evaluate sends first (topo order puts them before receives
            // since receives depend on nothing — we reorder below).
            let all_nodes: Vec<NodeId> = topo_order.clone();

            // Partition: non-receive nodes first, then receives.
            let mut ordered = Vec::with_capacity(all_nodes.len());
            let mut receives = Vec::new();
            for &id in &all_nodes {
                if let Some(node) = graph.node(id) {
                    if matches!(node.op, NodeOp::PortalReceive { .. }) {
                        receives.push(id);
                        continue;
                    }
                }
                ordered.push(id);
            }
            ordered.extend(receives);

            let mut local_outputs: HashMap<NodeId, Vec<NodeData>> = HashMap::new();
            for node_id in ordered {
                self.evaluate_one(graph, node_id, &mut local_outputs, &portal_map, time_ctx)?;
            }
            Ok(EvalResult { outputs: local_outputs })
        } else {
            // No portals — use parallel subgraph evaluation.
            let subgraph_results: Vec<Result<HashMap<NodeId, Vec<NodeData>>, ComputeError>> =
                subgraphs
                    .par_iter()
                    .map(|subgraph_nodes| {
                        self.evaluate_subgraph(graph, subgraph_nodes, &topo_order, time_ctx)
                    })
                    .collect();

            let mut outputs = HashMap::new();
            for result in subgraph_results {
                outputs.extend(result?);
            }
            Ok(EvalResult { outputs })
        }
    }

    /// Build a map from portal label → send NodeId.
    fn build_portal_map(&self, graph: &Graph) -> HashMap<String, NodeId> {
        let mut map = HashMap::new();
        for node in graph.nodes() {
            if let NodeOp::PortalSend { ref label } = node.op {
                map.insert(label.clone(), node.id);
            }
        }
        map
    }

    fn evaluate_subgraph(
        &self,
        graph: &Graph,
        subgraph_nodes: &[NodeId],
        topo_order: &[NodeId],
        time_ctx: &TimeContext,
    ) -> Result<HashMap<NodeId, Vec<NodeData>>, ComputeError> {
        let subgraph_set: std::collections::HashSet<NodeId> =
            subgraph_nodes.iter().copied().collect();

        let ordered: Vec<NodeId> = topo_order
            .iter()
            .filter(|id| subgraph_set.contains(id))
            .copied()
            .collect();

        let empty_portals = HashMap::new();
        let mut local_outputs: HashMap<NodeId, Vec<NodeData>> = HashMap::new();
        for node_id in ordered {
            self.evaluate_one(graph, node_id, &mut local_outputs, &empty_portals, time_ctx)?;
        }
        Ok(local_outputs)
    }

    /// Evaluate a single node, storing results in `local_outputs`.
    fn evaluate_one(
        &self,
        graph: &Graph,
        node_id: NodeId,
        local_outputs: &mut HashMap<NodeId, Vec<NodeData>>,
        portal_map: &HashMap<String, NodeId>,
        time_ctx: &TimeContext,
    ) -> Result<(), ComputeError> {
        let node = match graph.node(node_id) {
            Some(n) => n,
            None => return Ok(()),
        };

        // Check cache.
        {
            let cache = self.cache.read();
            if let Some(cached) = cache.get(node_id, node.generation) {
                local_outputs.insert(node_id, cached.clone());
                return Ok(());
            }
        }

        // Portal receives: look up the matching send's output.
        if let NodeOp::PortalReceive { ref label } = node.op {
            let results = if let Some(&send_id) = portal_map.get(label) {
                local_outputs.get(&send_id).cloned().unwrap_or_else(|| {
                    vec![NodeData::Scalar(0.0)]
                })
            } else {
                vec![NodeData::Scalar(0.0)]
            };
            local_outputs.insert(node_id, results);
            return Ok(());
        }

        // Resolve inputs: connection > expression > literal.
        let resolved = self.resolve_inputs(graph, node_id, local_outputs, time_ctx);

        // Evaluate.
        let mut node_outputs = NodeOutputs::new(node.outputs.len());
        self.backend
            .evaluate_node(&node.op, &resolved, time_ctx, &mut node_outputs)
            .map_err(|e| ComputeError::NodeEvalFailed {
                node: node_id,
                reason: e.to_string(),
            })?;

        // Collect outputs.
        // For sink nodes (no output ports, e.g. GraphOutput), store resolved
        // inputs so downstream consumers like collect_shapes can find the data.
        let results: Vec<NodeData> = if node_outputs.data.is_empty() {
            resolved.data
        } else {
            node_outputs.data.into_iter().flatten().collect()
        };

        // Store in cache.
        {
            let mut cache = self.cache.write();
            cache.insert(node_id, node.generation, results.clone());
        }

        local_outputs.insert(node_id, results);
        Ok(())
    }

    /// Resolve all input port values for a node.
    /// Priority: connection > expression > literal default.
    fn resolve_inputs(
        &self,
        graph: &Graph,
        node_id: NodeId,
        local_outputs: &HashMap<NodeId, Vec<NodeData>>,
        _time_ctx: &TimeContext,
    ) -> ResolvedInputs {
        let node = match graph.node(node_id) {
            Some(n) => n,
            None => return ResolvedInputs { data: Vec::new() },
        };

        let mut data = Vec::with_capacity(node.inputs.len());

        for (i, port_def) in node.inputs.iter().enumerate() {
            let port_id = crate::node::PortId {
                node: node_id,
                port: PortIndex(i),
            };

            // 1. Check for connection.
            if let Some(edge) = graph.input_connection(port_id) {
                // Look up the upstream node's output.
                if let Some(upstream_outputs) = local_outputs.get(&edge.from.node) {
                    if let Some(value) = upstream_outputs.get(edge.from.port.0) {
                        data.push(value.clone());
                        continue;
                    }
                }
            }

            // 2. Check for expression (stubbed — DSL crate will fill this in).
            //    For now, fall through to literal.

            // 3. Use literal default.
            if let Some(ref default) = port_def.default_value {
                data.push(param_to_node_data(default));
            } else {
                // No default — push a type-appropriate zero value.
                data.push(default_for_type(port_def.data_type));
            }
        }

        ResolvedInputs { data }
    }
}

/// Convert a ParamValue to its corresponding NodeData.
fn param_to_node_data(param: &ParamValue) -> NodeData {
    match param {
        ParamValue::Float(f) => NodeData::Scalar(*f),
        ParamValue::Int(i) => NodeData::Int(*i),
        ParamValue::Bool(b) => NodeData::Bool(*b),
        ParamValue::String(_) => NodeData::Scalar(0.0), // Strings don't flow through edges
        ParamValue::Vec2(v) => NodeData::Vec2(glam::Vec2::new(v[0], v[1])),
        ParamValue::Color(c) => NodeData::Color(crate::types::Color {
            r: c[0],
            g: c[1],
            b: c[2],
            a: c[3],
        }),
    }
}

/// Provide a sensible zero/default for a given DataType.
fn default_for_type(dt: crate::types::DataType) -> NodeData {
    use crate::types::DataType;
    match dt {
        DataType::Scalar | DataType::Any => NodeData::Scalar(0.0),
        DataType::Vec2 => NodeData::Vec2(glam::Vec2::ZERO),
        DataType::Path => NodeData::Path(Arc::new(crate::types::PathData::new())),
        DataType::Paths => NodeData::Paths(Arc::new(Vec::new())),
        DataType::Shape => NodeData::Shape(Arc::new(crate::types::Shape {
            path: crate::types::PathData::new(),
            fill: None,
            stroke: None,
            transform: glam::Affine2::IDENTITY,
        })),
        DataType::Shapes => NodeData::Shapes(Arc::new(Vec::new())),
        DataType::Transform => NodeData::Transform(glam::Affine2::IDENTITY),
        DataType::Color => NodeData::Color(crate::types::Color::BLACK),
        DataType::Bool => NodeData::Bool(false),
        DataType::Int => NodeData::Int(0),
        DataType::Points => {
            NodeData::Points(Arc::new(crate::types::PointBatch::new()))
        }
        DataType::Scalars => NodeData::Scalars(Arc::new(Vec::new())),
        DataType::Colors => NodeData::Colors(Arc::new(Vec::new())),
        DataType::Ints => NodeData::Ints(Arc::new(Vec::new())),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::{ComputeBackend, DslContext, TessellationOutput};
    use crate::graph::Graph;
    use crate::node::{NodeDef, NodeOp, PortId, PortIndex};
    use crate::types::*;

    /// A simple mock backend that passes the first input through as the first output.
    struct PassthroughBackend;

    impl ComputeBackend for PassthroughBackend {
        fn evaluate_node(
            &self,
            _op: &NodeOp,
            inputs: &ResolvedInputs,
            _time_ctx: &TimeContext,
            outputs: &mut NodeOutputs,
        ) -> Result<(), ComputeError> {
            // Pass through first input as first output (if any).
            if let Some(first) = inputs.data.first() {
                if !outputs.data.is_empty() {
                    outputs.data[0] = Some(first.clone());
                }
            }
            Ok(())
        }

        fn transform_points(&self, points: &PointBatch, _transform: &glam::Affine2) -> PointBatch {
            points.clone()
        }

        fn tessellate_path(
            &self,
            _path: &PathData,
            _fill: bool,
            _tolerance: f32,
        ) -> TessellationOutput {
            TessellationOutput {
                vertices: Vec::new(),
                indices: Vec::new(),
            }
        }

        fn execute_dsl(
            &self,
            _func_ptr: *const u8,
            _ctx: &mut DslContext,
        ) -> Result<(), ComputeError> {
            Ok(())
        }

        fn name(&self) -> &str {
            "passthrough"
        }
    }

    #[test]
    fn basic_evaluation() {
        let backend = Arc::new(PassthroughBackend);
        let scheduler = Scheduler::new(backend);

        let mut graph = Graph::new();
        let a = graph.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = graph.add_node(NodeDef::path_reverse(NodeId(0)));

        graph
            .connect(
                PortId { node: a, port: PortIndex(0) },
                PortId { node: b, port: PortIndex(0) },
            )
            .unwrap();

        let time_ctx = TimeContext::default();
        let result = scheduler.evaluate(&mut graph, &time_ctx).unwrap();

        // Both nodes should have been evaluated.
        assert!(result.outputs.contains_key(&a));
        assert!(result.outputs.contains_key(&b));
    }

    #[test]
    fn cache_hit() {
        let backend = Arc::new(PassthroughBackend);
        let scheduler = Scheduler::new(backend);

        let mut graph = Graph::new();
        let a = graph.add_node(NodeDef::circle(NodeId(0)));

        let time_ctx = TimeContext::default();

        // First evaluation populates cache.
        scheduler.evaluate(&mut graph, &time_ctx).unwrap();

        // Second evaluation should hit cache.
        let result = scheduler.evaluate(&mut graph, &time_ctx).unwrap();
        assert!(result.outputs.contains_key(&a));
    }

    #[test]
    fn cache_invalidation_on_generation_bump() {
        let backend = Arc::new(PassthroughBackend);
        let scheduler = Scheduler::new(backend);

        let mut graph = Graph::new();
        let a = graph.add_node(NodeDef::circle(NodeId(0)));

        let time_ctx = TimeContext::default();
        scheduler.evaluate(&mut graph, &time_ctx).unwrap();

        // Bump generation to invalidate cache.
        graph.node_mut(a).unwrap().touch();

        let result = scheduler.evaluate(&mut graph, &time_ctx).unwrap();
        assert!(result.outputs.contains_key(&a));
    }

    #[test]
    fn default_value_resolution() {
        let backend = Arc::new(PassthroughBackend);
        let scheduler = Scheduler::new(backend);

        let mut graph = Graph::new();
        let a = graph.add_node(NodeDef::regular_polygon(NodeId(0)));

        let time_ctx = TimeContext::default();
        let result = scheduler.evaluate(&mut graph, &time_ctx).unwrap();

        // The passthrough backend should have received the default values
        // and passed the first one (sides=6 as Int) through as output.
        let outputs = result.outputs.get(&a).unwrap();
        assert!(!outputs.is_empty());
        match &outputs[0] {
            NodeData::Int(6) => {} // sides default
            other => panic!("Expected Int(6), got {:?}", other),
        }
    }
}
