use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::error::GraphError;
use crate::node::{NodeDef, PortId, PortIndex};
use crate::types::NodeId;

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: PortId,
    pub to: PortId,
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    nodes: HashMap<NodeId, NodeDef>,
    edges: Vec<Edge>,
    next_id: u64,
    generation: u64,

    #[serde(skip)]
    adjacency_out: HashMap<NodeId, Vec<NodeId>>,
    #[serde(skip)]
    adjacency_in: HashMap<NodeId, Vec<NodeId>>,
    #[serde(skip)]
    topo_cache: Option<Vec<NodeId>>,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            next_id: 1,
            generation: 0,
            adjacency_out: HashMap::new(),
            adjacency_in: HashMap::new(),
            topo_cache: None,
        }
    }

    // -- Accessors --

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn node(&self, id: NodeId) -> Option<&NodeDef> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut NodeDef> {
        let node = self.nodes.get_mut(&id)?;
        self.generation += 1;
        Some(node)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &NodeDef> {
        self.nodes.values()
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // -- Mutation --

    /// Add a pre-built NodeDef to the graph, returning its id.
    /// The NodeDef's id is overwritten with a fresh one.
    pub fn add_node(&mut self, mut node: NodeDef) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        node.id = id;
        self.adjacency_out.entry(id).or_default();
        self.adjacency_in.entry(id).or_default();
        self.nodes.insert(id, node);
        self.bump_generation();
        id
    }

    pub fn remove_node(&mut self, id: NodeId) -> Result<NodeDef, GraphError> {
        let node = self.nodes.remove(&id).ok_or(GraphError::NodeNotFound(id))?;
        // Remove all edges touching this node.
        self.edges.retain(|e| e.from.node != id && e.to.node != id);
        self.adjacency_out.remove(&id);
        self.adjacency_in.remove(&id);
        // Clean references in other nodes' adjacency lists.
        for list in self.adjacency_out.values_mut() {
            list.retain(|n| *n != id);
        }
        for list in self.adjacency_in.values_mut() {
            list.retain(|n| *n != id);
        }
        self.bump_generation();
        Ok(node)
    }

    /// Connect an output port to an input port.
    /// Validates types (via `can_promote_to`), enforces single-connection-per-input,
    /// and rejects cycles.
    pub fn connect(&mut self, from: PortId, to: PortId) -> Result<(), GraphError> {
        // Validate nodes exist.
        let from_node = self
            .nodes
            .get(&from.node)
            .ok_or(GraphError::NodeNotFound(from.node))?;
        let to_node = self
            .nodes
            .get(&to.node)
            .ok_or(GraphError::NodeNotFound(to.node))?;

        // Validate port indices.
        let from_port = from_node
            .outputs
            .get(from.port.0)
            .ok_or(GraphError::PortNotFound {
                node: from.node,
                port: from.port.0,
            })?;
        let to_port = to_node
            .inputs
            .get(to.port.0)
            .ok_or(GraphError::PortNotFound {
                node: to.node,
                port: to.port.0,
            })?;

        // Type check.
        if !from_port.data_type.can_promote_to(&to_port.data_type) {
            return Err(GraphError::TypeMismatch {
                source_type: from_port.data_type,
                target_type: to_port.data_type,
            });
        }

        // Enforce single connection per input port.
        if self.edges.iter().any(|e| e.to == to) {
            return Err(GraphError::DuplicateConnection {
                node: to.node,
                port: to.port.0,
            });
        }

        // Cycle check: would adding from.node -> to.node create a cycle?
        if from.node == to.node || self.would_create_cycle(from.node, to.node) {
            return Err(GraphError::CycleDetected {
                from: from.node,
                to: to.node,
            });
        }

        // All checks passed — add the edge.
        self.edges.push(Edge {
            from,
            to,
        });
        self.adjacency_out
            .entry(from.node)
            .or_default()
            .push(to.node);
        self.adjacency_in
            .entry(to.node)
            .or_default()
            .push(from.node);

        self.bump_generation();
        Ok(())
    }

    /// Disconnect a specific edge (from -> to).
    pub fn disconnect(&mut self, from: PortId, to: PortId) -> bool {
        let before = self.edges.len();
        self.edges.retain(|e| !(e.from == from && e.to == to));
        let removed = self.edges.len() < before;
        if removed {
            // Rebuild adjacency for affected nodes.
            self.rebuild_adjacency_for(from.node);
            self.rebuild_adjacency_for(to.node);
            self.bump_generation();
        }
        removed
    }

    /// Disconnect all edges to/from a specific input port on a node.
    pub fn disconnect_input_port(&mut self, node: NodeId, port: PortIndex) {
        let pid = PortId { node, port };
        let before = self.edges.len();
        self.edges.retain(|e| e.to != pid);
        if self.edges.len() < before {
            self.rebuild_adjacency_for(node);
            self.bump_generation();
        }
    }

    /// Find the edge connected to a specific input port, if any.
    pub fn input_connection(&self, to: PortId) -> Option<&Edge> {
        self.edges.iter().find(|e| e.to == to)
    }

    // -- Topology --

    /// Topological sort using Kahn's algorithm. Cached until topology changes.
    pub fn topological_sort(&mut self) -> Result<Vec<NodeId>, GraphError> {
        if let Some(ref cached) = self.topo_cache {
            return Ok(cached.clone());
        }

        let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
        for &id in self.nodes.keys() {
            in_degree.insert(id, 0);
        }
        for neighbors in self.adjacency_out.values() {
            for &neighbor in neighbors {
                *in_degree.entry(neighbor).or_default() += 1;
            }
        }

        let mut queue: VecDeque<NodeId> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();
        // Sort for deterministic output.
        let mut initial: Vec<NodeId> = queue.drain(..).collect();
        initial.sort_by_key(|id| id.0);
        queue.extend(initial);

        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(node) = queue.pop_front() {
            order.push(node);
            if let Some(neighbors) = self.adjacency_out.get(&node) {
                let mut sorted_neighbors = neighbors.clone();
                sorted_neighbors.sort_by_key(|id| id.0);
                for neighbor in sorted_neighbors {
                    let deg = in_degree.get_mut(&neighbor).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            // This shouldn't happen if connect() validates correctly,
            // but guard against it.
            let missing = self
                .nodes
                .keys()
                .find(|id| !order.contains(id))
                .copied()
                .unwrap();
            return Err(GraphError::CycleDetected {
                from: missing,
                to: missing,
            });
        }

        self.topo_cache = Some(order.clone());
        Ok(order)
    }

    /// Check whether adding an edge from `from` to `to` would create a cycle.
    /// Does a DFS from `to` to see if we can reach `from`.
    fn would_create_cycle(&self, from: NodeId, to: NodeId) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![to];
        while let Some(current) = stack.pop() {
            if current == from {
                return true;
            }
            if visited.insert(current) {
                if let Some(neighbors) = self.adjacency_out.get(&current) {
                    stack.extend(neighbors);
                }
            }
        }
        false
    }

    /// Return the set of all nodes reachable from `roots` via outgoing edges.
    /// The roots themselves are NOT included in the result.
    pub fn downstream_of(&self, roots: &[NodeId]) -> HashSet<NodeId> {
        let mut visited = HashSet::new();
        let mut stack: Vec<NodeId> = Vec::new();
        for &root in roots {
            if let Some(neighbors) = self.adjacency_out.get(&root) {
                stack.extend(neighbors);
            }
        }
        while let Some(current) = stack.pop() {
            if visited.insert(current) {
                if let Some(neighbors) = self.adjacency_out.get(&current) {
                    stack.extend(neighbors);
                }
            }
        }
        visited
    }

    /// Find independent subgraphs using Union-Find.
    /// Returns groups of NodeIds that can be evaluated in parallel.
    pub fn independent_subgraphs(&self) -> Vec<Vec<NodeId>> {
        if self.nodes.is_empty() {
            return Vec::new();
        }

        let ids: Vec<NodeId> = {
            let mut v: Vec<NodeId> = self.nodes.keys().copied().collect();
            v.sort_by_key(|id| id.0);
            v
        };
        let index_of: HashMap<NodeId, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        let mut parent: Vec<usize> = (0..ids.len()).collect();
        let mut rank: Vec<usize> = vec![0; ids.len()];

        fn find(parent: &mut [usize], x: usize) -> usize {
            if parent[x] != x {
                parent[x] = find(parent, parent[x]);
            }
            parent[x]
        }

        fn union(parent: &mut [usize], rank: &mut [usize], a: usize, b: usize) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra == rb {
                return;
            }
            if rank[ra] < rank[rb] {
                parent[ra] = rb;
            } else if rank[ra] > rank[rb] {
                parent[rb] = ra;
            } else {
                parent[rb] = ra;
                rank[ra] += 1;
            }
        }

        for edge in &self.edges {
            let a = index_of[&edge.from.node];
            let b = index_of[&edge.to.node];
            union(&mut parent, &mut rank, a, b);
        }

        let mut groups: HashMap<usize, Vec<NodeId>> = HashMap::new();
        for (i, &id) in ids.iter().enumerate() {
            let root = find(&mut parent, i);
            groups.entry(root).or_default().push(id);
        }

        groups.into_values().collect()
    }

    // -- Internal helpers --

    fn bump_generation(&mut self) {
        self.generation += 1;
        self.invalidate_topo();
    }

    fn invalidate_topo(&mut self) {
        self.topo_cache = None;
    }

    fn rebuild_adjacency_for(&mut self, node_id: NodeId) {
        // Clear and rebuild outgoing.
        if let Some(out) = self.adjacency_out.get_mut(&node_id) {
            out.clear();
        }
        // Clear incoming references to this node from others.
        for list in self.adjacency_in.values_mut() {
            list.retain(|n| *n != node_id);
        }
        if let Some(inc) = self.adjacency_in.get_mut(&node_id) {
            inc.clear();
        }

        for edge in &self.edges {
            if edge.from.node == node_id {
                self.adjacency_out
                    .entry(node_id)
                    .or_default()
                    .push(edge.to.node);
                self.adjacency_in
                    .entry(edge.to.node)
                    .or_default()
                    .push(node_id);
            }
            if edge.to.node == node_id {
                self.adjacency_in
                    .entry(node_id)
                    .or_default()
                    .push(edge.from.node);
                self.adjacency_out
                    .entry(edge.from.node)
                    .or_default()
                    .push(node_id);
            }
        }
    }

    /// Rebuild all adjacency caches from edges. Called after deserialization.
    pub fn rebuild_caches(&mut self) {
        self.adjacency_out.clear();
        self.adjacency_in.clear();
        self.topo_cache = None;

        for id in self.nodes.keys() {
            self.adjacency_out.entry(*id).or_default();
            self.adjacency_in.entry(*id).or_default();
        }

        for edge in &self.edges {
            self.adjacency_out
                .entry(edge.from.node)
                .or_default()
                .push(edge.to.node);
            self.adjacency_in
                .entry(edge.to.node)
                .or_default()
                .push(edge.from.node);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{NodeDef, PortIndex};
    use crate::types::DataType;

    #[test]
    fn add_and_remove_nodes() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::circle(NodeId(0)));
        let b = g.add_node(NodeDef::circle(NodeId(0)));
        assert_eq!(g.node_count(), 2);

        g.remove_node(a).unwrap();
        assert_eq!(g.node_count(), 1);
        assert!(g.node(a).is_none());
        assert!(g.node(b).is_some());
    }

    #[test]
    fn remove_nonexistent_node() {
        let mut g = Graph::new();
        assert!(g.remove_node(NodeId(999)).is_err());
    }

    #[test]
    fn connect_valid_types() {
        // RegularPolygon outputs Path[0], SetFill expects Shape[0] — not compatible
        // But we can connect RegularPolygon Path -> PathReverse Path
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = g.add_node(NodeDef::path_reverse(NodeId(0)));

        let result = g.connect(
            PortId { node: a, port: PortIndex(0) },
            PortId { node: b, port: PortIndex(0) },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn connect_type_mismatch() {
        // Path output -> Int input should fail
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = g.add_node(NodeDef::regular_polygon(NodeId(0)));

        let result = g.connect(
            PortId { node: a, port: PortIndex(0) }, // Path output
            PortId { node: b, port: PortIndex(0) }, // Int input (sides)
        );
        assert!(matches!(result, Err(GraphError::TypeMismatch { .. })));
    }

    #[test]
    fn connect_duplicate_input() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let c = g.add_node(NodeDef::path_reverse(NodeId(0)));

        // First connection ok
        g.connect(
            PortId { node: a, port: PortIndex(0) },
            PortId { node: c, port: PortIndex(0) },
        )
        .unwrap();

        // Second connection to same input should fail
        let result = g.connect(
            PortId { node: b, port: PortIndex(0) },
            PortId { node: c, port: PortIndex(0) },
        );
        assert!(matches!(result, Err(GraphError::DuplicateConnection { .. })));
    }

    #[test]
    fn connect_creates_cycle() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::path_reverse(NodeId(0)));
        let b = g.add_node(NodeDef::path_reverse(NodeId(0)));

        g.connect(
            PortId { node: a, port: PortIndex(0) },
            PortId { node: b, port: PortIndex(0) },
        )
        .unwrap();

        // b -> a would create a cycle
        let result = g.connect(
            PortId { node: b, port: PortIndex(0) },
            PortId { node: a, port: PortIndex(0) },
        );
        assert!(matches!(result, Err(GraphError::CycleDetected { .. })));
    }

    #[test]
    fn self_connection_rejected() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::path_reverse(NodeId(0)));

        let result = g.connect(
            PortId { node: a, port: PortIndex(0) },
            PortId { node: a, port: PortIndex(0) },
        );
        assert!(matches!(result, Err(GraphError::CycleDetected { .. })));
    }

    #[test]
    fn disconnect_edge() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = g.add_node(NodeDef::path_reverse(NodeId(0)));

        let from = PortId { node: a, port: PortIndex(0) };
        let to = PortId { node: b, port: PortIndex(0) };

        g.connect(from, to).unwrap();
        assert_eq!(g.edges().len(), 1);

        let removed = g.disconnect(from, to);
        assert!(removed);
        assert_eq!(g.edges().len(), 0);

        // Disconnect nonexistent edge returns false.
        let removed = g.disconnect(from, to);
        assert!(!removed);
    }

    #[test]
    fn topological_sort_linear() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = g.add_node(NodeDef::path_reverse(NodeId(0)));
        let c = g.add_node(NodeDef::path_reverse(NodeId(0)));

        g.connect(
            PortId { node: a, port: PortIndex(0) },
            PortId { node: b, port: PortIndex(0) },
        )
        .unwrap();
        g.connect(
            PortId { node: b, port: PortIndex(0) },
            PortId { node: c, port: PortIndex(0) },
        )
        .unwrap();

        let order = g.topological_sort().unwrap();
        assert_eq!(order.len(), 3);

        // a must come before b, b before c.
        let pos_a = order.iter().position(|&id| id == a).unwrap();
        let pos_b = order.iter().position(|&id| id == b).unwrap();
        let pos_c = order.iter().position(|&id| id == c).unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn topological_sort_cached() {
        let mut g = Graph::new();
        let _a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let _b = g.add_node(NodeDef::path_reverse(NodeId(0)));

        let order1 = g.topological_sort().unwrap();
        let order2 = g.topological_sort().unwrap();
        assert_eq!(order1, order2);
    }

    #[test]
    fn independent_subgraphs_disconnected() {
        let mut g = Graph::new();
        let _a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let _b = g.add_node(NodeDef::circle(NodeId(0)));
        let _c = g.add_node(NodeDef::rectangle(NodeId(0)));

        let subgraphs = g.independent_subgraphs();
        // Three disconnected nodes = three subgraphs.
        assert_eq!(subgraphs.len(), 3);
    }

    #[test]
    fn independent_subgraphs_connected() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = g.add_node(NodeDef::path_reverse(NodeId(0)));
        let _c = g.add_node(NodeDef::circle(NodeId(0)));

        g.connect(
            PortId { node: a, port: PortIndex(0) },
            PortId { node: b, port: PortIndex(0) },
        )
        .unwrap();

        let subgraphs = g.independent_subgraphs();
        // a-b connected, c alone = 2 subgraphs.
        assert_eq!(subgraphs.len(), 2);
    }

    #[test]
    fn generation_increments_on_topology_change() {
        let mut g = Graph::new();
        let gen0 = g.generation();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        // add_node doesn't bump generation (only invalidates topo cache)
        let b = g.add_node(NodeDef::path_reverse(NodeId(0)));

        let from = PortId { node: a, port: PortIndex(0) };
        let to = PortId { node: b, port: PortIndex(0) };
        g.connect(from, to).unwrap();
        let gen1 = g.generation();
        assert!(gen1 > gen0);

        g.disconnect(from, to);
        let gen2 = g.generation();
        assert!(gen2 > gen1);
    }

    #[test]
    fn type_promotion_in_connect() {
        // Int -> Scalar promotion should work
        let mut g = Graph::new();
        // GraphInput(Int) -> input expecting Scalar
        let a = g.add_node(NodeDef::graph_input(
            NodeId(0),
            "val".into(),
            DataType::Int,
        ));
        let b = g.add_node(NodeDef::circle(NodeId(0)));

        // a outputs Int, b input[0] (radius) is Scalar
        let result = g.connect(
            PortId { node: a, port: PortIndex(0) },
            PortId { node: b, port: PortIndex(0) },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn input_connection_lookup() {
        let mut g = Graph::new();
        let a = g.add_node(NodeDef::regular_polygon(NodeId(0)));
        let b = g.add_node(NodeDef::path_reverse(NodeId(0)));

        let to = PortId { node: b, port: PortIndex(0) };
        assert!(g.input_connection(to).is_none());

        let from = PortId { node: a, port: PortIndex(0) };
        g.connect(from, to).unwrap();
        let edge = g.input_connection(to).unwrap();
        assert_eq!(edge.from, from);
    }
}
