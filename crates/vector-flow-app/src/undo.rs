use egui_snarl::Snarl;
use vector_flow_core::graph::Graph;

use crate::id_map::IdMap;
use crate::project::rebuild_id_map;
use crate::ui_node::UiNode;

const MAX_UNDO: usize = 100;

#[derive(Clone)]
struct Snapshot {
    graph: Graph,
    snarl: Snarl<UiNode>,
}

/// Snapshot-based undo/redo with automatic coalescing of continuous edits.
///
/// Coalescing works by tracking a fingerprint (graph generation + node position hash).
/// Only the first frame of a change sequence pushes a snapshot; subsequent frames
/// in the same sequence (e.g. slider drag, node drag) are coalesced into one entry.
pub struct UndoHistory {
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    /// Fingerprint from the end of the previous frame.
    prev_fingerprint: u64,
    /// Whether the previous frame was already part of a change sequence.
    was_changing: bool,
    /// Snapshot taken at the start of the current frame (before mutations).
    /// Only captured when not already in a change sequence.
    pre_frame: Option<Snapshot>,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            prev_fingerprint: 0,
            was_changing: false,
            pre_frame: None,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Call at the start of each frame, before any mutations.
    /// Captures a snapshot if we're not in the middle of a continuous edit.
    pub fn begin_frame(&mut self, graph: &Graph, snarl: &Snarl<UiNode>) {
        if !self.was_changing {
            self.pre_frame = Some(Snapshot {
                graph: graph.clone(),
                snarl: snarl.clone(),
            });
        }
    }

    /// Call at the end of each frame, after all mutations.
    /// Detects changes and auto-pushes undo entries.
    pub fn end_frame(&mut self, fingerprint: u64) {
        let changed = fingerprint != self.prev_fingerprint;

        if changed {
            if !self.was_changing {
                // First frame of a change sequence — push the pre-frame snapshot.
                if let Some(snapshot) = self.pre_frame.take() {
                    self.push_snapshot(snapshot);
                    self.redo_stack.clear();
                }
            }
            self.was_changing = true;
        } else {
            self.was_changing = false;
        }

        self.prev_fingerprint = fingerprint;
    }

    /// Undo: restore the previous state. Returns the restored (graph, snarl, id_map) if successful.
    pub fn undo(&mut self, graph: &Graph, snarl: &Snarl<UiNode>) -> Option<(Graph, Snarl<UiNode>, IdMap)> {
        let snapshot = self.undo_stack.pop()?;

        // Push current state to redo.
        self.redo_stack.push(Snapshot {
            graph: graph.clone(),
            snarl: snarl.clone(),
        });

        let id_map = rebuild_id_map(&snapshot.snarl);
        self.was_changing = false;
        self.pre_frame = None;
        Some((snapshot.graph, snapshot.snarl, id_map))
    }

    /// Redo: restore the next state. Returns the restored (graph, snarl, id_map) if successful.
    pub fn redo(&mut self, graph: &Graph, snarl: &Snarl<UiNode>) -> Option<(Graph, Snarl<UiNode>, IdMap)> {
        let snapshot = self.redo_stack.pop()?;

        // Push current state to undo.
        self.push_snapshot(Snapshot {
            graph: graph.clone(),
            snarl: snarl.clone(),
        });

        let id_map = rebuild_id_map(&snapshot.snarl);
        self.was_changing = false;
        self.pre_frame = None;
        Some((snapshot.graph, snapshot.snarl, id_map))
    }

    /// Update the fingerprint without triggering change detection.
    /// Call after restoring state from undo/redo.
    pub fn sync_fingerprint(&mut self, fingerprint: u64) {
        self.prev_fingerprint = fingerprint;
        self.was_changing = false;
        self.pre_frame = None;
    }

    /// Clear all history (e.g. on project load or new).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.was_changing = false;
        self.pre_frame = None;
        self.prev_fingerprint = 0;
    }

    fn push_snapshot(&mut self, snapshot: Snapshot) {
        if self.undo_stack.len() >= MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
    }
}
