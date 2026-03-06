use glam::Affine2;

use crate::error::ComputeError;
use crate::node::NodeOp;
use crate::types::{NodeData, PathData, PointBatch, EvalContext};

// ---------------------------------------------------------------------------
// Resolved inputs / outputs
// ---------------------------------------------------------------------------

/// All input ports resolved to concrete values (connection > expression > literal).
pub struct ResolvedInputs {
    pub data: Vec<NodeData>,
}

/// Output slots for a node evaluation. Each output port may or may not produce a value.
pub struct NodeOutputs {
    pub data: Vec<Option<NodeData>>,
    /// Non-fatal error message (e.g. DSL compile error). The node still produces
    /// default outputs but the UI can display this to the user.
    pub error: Option<String>,
}

impl NodeOutputs {
    pub fn new(port_count: usize) -> Self {
        Self {
            data: vec![None; port_count],
            error: None,
        }
    }
}

/// Tessellated geometry ready for GPU upload.
pub struct TessellationOutput {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

// ---------------------------------------------------------------------------
// ComputeBackend trait
// ---------------------------------------------------------------------------

/// Abstraction over the execution engine (CPU, GPU, etc.).
pub trait ComputeBackend: Send + Sync {
    /// Evaluate a single node given its resolved inputs.
    fn evaluate_node(
        &self,
        op: &NodeOp,
        inputs: &ResolvedInputs,
        time_ctx: &EvalContext,
        outputs: &mut NodeOutputs,
    ) -> Result<(), ComputeError>;

    /// Transform a batch of points by an affine transform.
    fn transform_points(&self, points: &PointBatch, transform: &Affine2) -> PointBatch;

    /// Tessellate a path into triangles for rendering.
    fn tessellate_path(&self, path: &PathData, fill: bool, tolerance: f32) -> TessellationOutput;

    /// Execute a JIT-compiled DSL function.
    /// # Safety
    /// `func_ptr` must point to a valid compiled function.
    fn execute_dsl(&self, func_ptr: *const u8, ctx: &mut DslContext) -> Result<(), ComputeError>;

    /// Human-readable backend name (e.g. "CPU", "GPU").
    fn name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// DslContext — runtime context passed to JIT-compiled DSL functions
// ---------------------------------------------------------------------------

/// Context passed to JIT-compiled DSL code.
///
/// All fields are `#[repr(C)]`-safe so Cranelift can compute field offsets
/// reliably via `std::mem::offset_of!`.
#[repr(C)]
pub struct DslContext {
    /// Fixed slots for common values (up to 8).
    pub slots: [f64; 8],
    /// Pointer to heap-allocated overflow storage (for functions with >8 locals).
    pub overflow_ptr: *mut f64,
    /// Number of valid f64 values at `overflow_ptr`.
    pub overflow_len: u32,
    /// Padding to keep 8-byte alignment for `frame`.
    pub _pad0: u32,
    /// Current frame number.
    pub frame: u64,
    /// Current time in seconds.
    pub time_secs: f32,
    /// Frames per second.
    pub fps: f32,
}

// SAFETY: DslContext is only used on the thread that creates it.
// The raw pointer is into a Vec owned by the same DslContext wrapper.
unsafe impl Send for DslContext {}
unsafe impl Sync for DslContext {}

impl DslContext {
    pub fn new(time_ctx: &EvalContext) -> Self {
        Self {
            slots: [0.0; 8],
            overflow_ptr: std::ptr::null_mut(),
            overflow_len: 0,
            _pad0: 0,
            frame: time_ctx.frame,
            time_secs: time_ctx.time_secs,
            fps: time_ctx.fps,
        }
    }

    /// Allocate overflow storage for functions that need more than 8 locals.
    /// Returns a Vec that owns the memory — caller must keep it alive while
    /// `self.overflow_ptr` is in use.
    pub fn alloc_overflow(&mut self, count: usize) -> Vec<f64> {
        let mut storage = vec![0.0f64; count];
        self.overflow_ptr = storage.as_mut_ptr();
        self.overflow_len = count as u32;
        storage
    }
}
