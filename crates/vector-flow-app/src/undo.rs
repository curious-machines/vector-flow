use egui_snarl::Snarl;
use vector_flow_core::graph::Graph;

use crate::id_map::IdMap;
use crate::project::{rebuild_id_map, ProjectSettings};
use crate::ui_node::UiNode;

const MAX_UNDO: usize = 100;

#[derive(Clone)]
struct Snapshot {
    graph: Graph,
    snarl: Snarl<UiNode>,
    settings: ProjectSettings,
    label: String,
}

/// Info captured at begin_frame for computing undo labels by comparison.
#[derive(Clone)]
struct FrameInfo {
    graph_gen: u64,
    node_pos_hash: u64,
    fps_bits: u32,
    canvas_width: u32,
    canvas_height: u32,
    bg_color: Option<[u32; 4]>,
}

impl FrameInfo {
    fn diff_label(&self, other: &FrameInfo) -> String {
        if self.graph_gen != other.graph_gen {
            return "Edit Graph".to_string();
        }
        if self.node_pos_hash != other.node_pos_hash {
            return "Move Nodes".to_string();
        }
        if self.fps_bits != other.fps_bits {
            return "Change FPS".to_string();
        }
        if self.canvas_width != other.canvas_width || self.canvas_height != other.canvas_height {
            return "Resize Canvas".to_string();
        }
        if self.bg_color != other.bg_color {
            return "Change Background".to_string();
        }
        "Edit".to_string()
    }
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
    /// State info captured alongside pre_frame, for computing diff labels.
    pre_frame_info: Option<FrameInfo>,
    /// Number of consecutive frames with no fingerprint change. Used to detect
    /// when a stable gap separates two distinct actions so we don't coalesce them.
    stable_frames: u32,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            prev_fingerprint: 0,
            was_changing: false,
            pre_frame: None,
            pre_frame_info: None,
            stable_frames: 0,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Call at the start of each frame, before any mutations.
    /// Captures a snapshot if we're not in the middle of a continuous edit,
    /// or if there has been a stable gap (fingerprint unchanged for at least
    /// one frame) indicating a previous action has completed.
    pub fn begin_frame(&mut self, graph: &Graph, snarl: &Snarl<UiNode>, settings: &ProjectSettings, node_pos_hash: u64) {
        if !self.was_changing || self.stable_frames > 0 {
            // If was_changing but we had a stable gap, reset so this frame
            // starts a fresh change sequence.
            if self.was_changing && self.stable_frames > 0 {
                self.was_changing = false;
            }
            self.pre_frame_info = Some(FrameInfo {
                graph_gen: graph.generation(),
                node_pos_hash,
                fps_bits: settings.fps.to_bits(),
                canvas_width: settings.canvas_width,
                canvas_height: settings.canvas_height,
                bg_color: settings.background_color.map(|c| {
                    [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()]
                }),
            });
            self.pre_frame = Some(Snapshot {
                graph: graph.clone(),
                snarl: snarl.clone(),
                settings: settings.clone(),
                label: String::new(),
            });
        }
    }

    /// Call at the end of each frame, after all mutations.
    /// Detects changes and auto-pushes undo entries.
    /// `current_graph` and `current_settings` are used to compute a descriptive label.
    /// `pointer_down`: true if the primary mouse button is held — prevents premature
    /// coalescing reset during slow drags where the value may not change every frame.
    pub fn end_frame(&mut self, fingerprint: u64, current_graph: &Graph, current_settings: &ProjectSettings, node_pos_hash: u64, pointer_down: bool) {
        let current_info = FrameInfo {
            graph_gen: current_graph.generation(),
            node_pos_hash,
            fps_bits: current_settings.fps.to_bits(),
            canvas_width: current_settings.canvas_width,
            canvas_height: current_settings.canvas_height,
            bg_color: current_settings.background_color.map(|c| {
                [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()]
            }),
        };
        let changed = fingerprint != self.prev_fingerprint;

        if changed {
            if !self.was_changing {
                // First frame of a change sequence — push the pre-frame snapshot.
                if let Some(mut snapshot) = self.pre_frame.take() {
                    // Compute label by comparing pre-frame info with current state.
                    let label = if let Some(ref pre_info) = self.pre_frame_info {
                        pre_info.diff_label(&current_info)
                    } else {
                        "Edit".to_string()
                    };
                    snapshot.label = label;
                    self.push_snapshot(snapshot);
                    self.redo_stack.clear();
                }
            }
            self.was_changing = true;
            self.stable_frames = 0;
        } else if !pointer_down {
            // Only reset when the pointer is released — during slow drags the
            // value may not change every frame, but we still want to coalesce.
            self.was_changing = false;
            self.stable_frames = 0;
        } else {
            // Pointer is down but no change this frame. Count stable frames
            // so begin_frame can detect a gap between distinct actions.
            self.stable_frames = self.stable_frames.saturating_add(1);
        }

        self.prev_fingerprint = fingerprint;
    }

    /// Undo: restore the previous state. Returns the restored (graph, snarl, id_map) if successful.
    pub fn undo(&mut self, graph: &Graph, snarl: &Snarl<UiNode>, settings: &ProjectSettings) -> Option<(Graph, Snarl<UiNode>, IdMap, ProjectSettings)> {
        let snapshot = self.undo_stack.pop()?;

        // Push current state to redo with the label of the entry we're undoing.
        self.redo_stack.push(Snapshot {
            graph: graph.clone(),
            snarl: snarl.clone(),
            settings: settings.clone(),
            label: snapshot.label.clone(),
        });

        let id_map = rebuild_id_map(&snapshot.snarl);
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
        Some((snapshot.graph, snapshot.snarl, id_map, snapshot.settings))
    }

    /// Redo: restore the next state. Returns the restored (graph, snarl, id_map, settings) if successful.
    pub fn redo(&mut self, graph: &Graph, snarl: &Snarl<UiNode>, settings: &ProjectSettings) -> Option<(Graph, Snarl<UiNode>, IdMap, ProjectSettings)> {
        let snapshot = self.redo_stack.pop()?;

        // Push current state to undo with the label of the entry we're redoing.
        self.push_snapshot(Snapshot {
            graph: graph.clone(),
            snarl: snarl.clone(),
            settings: settings.clone(),
            label: snapshot.label.clone(),
        });

        let id_map = rebuild_id_map(&snapshot.snarl);
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
        Some((snapshot.graph, snapshot.snarl, id_map, snapshot.settings))
    }

    /// Label of the top undo entry (what will be undone).
    pub fn undo_label(&self) -> Option<&str> {
        self.undo_stack.last().map(|s| s.label.as_str())
    }

    /// Label of the top redo entry (what will be redone).
    pub fn redo_label(&self) -> Option<&str> {
        self.redo_stack.last().map(|s| s.label.as_str())
    }

    /// Update the fingerprint without triggering change detection.
    /// Call after restoring state from undo/redo.
    pub fn sync_fingerprint(&mut self, fingerprint: u64) {
        self.prev_fingerprint = fingerprint;
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
        self.stable_frames = 0;
    }

    /// Clear all history (e.g. on project load or new).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.was_changing = false;
        self.pre_frame = None;
        self.pre_frame_info = None;
        self.prev_fingerprint = 0;
        self.stable_frames = 0;
    }

    /// Returns the FPS from the pre-frame snapshot (captured before UI mutations).
    pub fn pre_frame_fps(&self) -> Option<f32> {
        self.pre_frame.as_ref().map(|s| s.settings.fps)
    }

    fn push_snapshot(&mut self, snapshot: Snapshot) {
        if self.undo_stack.len() >= MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> (Graph, Snarl<UiNode>, ProjectSettings) {
        (Graph::new(), Snarl::new(), ProjectSettings::default())
    }

    fn fingerprint(gen: u64, pos: u64) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        gen.hash(&mut h);
        pos.hash(&mut h);
        h.finish()
    }

    #[test]
    fn separate_actions_with_pointer_held_produce_two_undo_entries() {
        // Simulates: connect edge (graph gen changes), then immediately start
        // dragging a node (positions change) while pointer stays down.
        let (graph, snarl, settings) = make_state();
        let mut undo = UndoHistory::new();
        undo.sync_fingerprint(fingerprint(0, 0));

        // Frame 1: connection happens (graph gen 0 → 1), pointer down.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(1, 0), &graph, &settings, 0, true);
        assert_eq!(undo.undo_stack.len(), 1, "connection should push snapshot");

        // Frame 2: no change yet, pointer still down (about to start drag).
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(1, 0), &graph, &settings, 0, true);
        assert_eq!(undo.undo_stack.len(), 1, "no change = no new snapshot");

        // Frame 3: node starts moving (pos hash changes), pointer down.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(1, 1), &graph, &settings, 1, true);
        assert_eq!(undo.undo_stack.len(), 2, "move should be separate undo entry");
    }

    #[test]
    fn continuous_drag_coalesces_into_one_entry() {
        // Dragging a node across multiple frames should produce one undo entry.
        let (graph, snarl, settings) = make_state();
        let mut undo = UndoHistory::new();
        undo.sync_fingerprint(fingerprint(0, 0));

        // Frame 1: drag starts.
        undo.begin_frame(&graph, &snarl, &settings, 0);
        undo.end_frame(fingerprint(0, 1), &graph, &settings, 1, true);
        assert_eq!(undo.undo_stack.len(), 1);

        // Frame 2: drag continues.
        undo.begin_frame(&graph, &snarl, &settings, 1);
        undo.end_frame(fingerprint(0, 2), &graph, &settings, 2, true);
        assert_eq!(undo.undo_stack.len(), 1, "continuous drag should coalesce");

        // Frame 3: drag continues.
        undo.begin_frame(&graph, &snarl, &settings, 2);
        undo.end_frame(fingerprint(0, 3), &graph, &settings, 3, true);
        assert_eq!(undo.undo_stack.len(), 1, "still coalescing");
    }
}
