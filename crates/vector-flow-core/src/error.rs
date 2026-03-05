use crate::types::{DataType, NodeId};

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("connecting {from:?} -> {to:?} would create a cycle")]
    CycleDetected { from: NodeId, to: NodeId },

    #[error("node {0:?} not found")]
    NodeNotFound(NodeId),

    #[error("port index {port} out of range on node {node:?}")]
    PortNotFound { node: NodeId, port: usize },

    #[error("cannot connect {source_type:?} to {target_type:?}")]
    TypeMismatch { source_type: DataType, target_type: DataType },

    #[error("input port {node:?}[{port}] already has a connection")]
    DuplicateConnection { node: NodeId, port: usize },
}

#[derive(Debug, thiserror::Error)]
pub enum ComputeError {
    #[error("evaluation failed for node {node:?}: {reason}")]
    NodeEvalFailed { node: NodeId, reason: String },

    #[error("DSL error: {0}")]
    DslError(String),

    #[error("backend error: {0}")]
    BackendError(String),
}
